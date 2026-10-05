use sha2::{Digest, Sha256};

use crate::QList;

pub const HASH_SCHEMA_VERSION: u32 = 1;

/// One canonical result record for one n.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultRecord {
    pub n: u64,
    pub status: u8,
    pub p: u64,
    pub q: u64,
    pub q_candidates_tested: u64,
}

pub fn hash_q_list(q_list: &QList) -> [u8; 32] {
    let mut hasher = Sha256::new();
    // Values that fit in u32 keep the original v1 wire format so that Q-list
    // hashes computed before the u64 migration remain reproducible; only
    // ranges whose q bound exceeds u32::MAX (unreachable before this fix)
    // use the wider v2 record encoding.
    if q_list.maximum_q() <= u64::from(u32::MAX) {
        hasher.update(b"A220091-Q-LIST-v1\0");
        for &q in q_list.as_slice() {
            let q32 = u32::try_from(q).expect("q fits in u32 when maximum_q fits in u32");
            hasher.update(q32.to_le_bytes());
        }
    } else {
        hasher.update(b"A220091-Q-LIST-v2\0");
        for &q in q_list.as_slice() {
            hasher.update(q.to_le_bytes());
        }
    }
    hasher.finalize().into()
}

pub fn update_result_hash(hasher: &mut Sha256, record: ResultRecord) {
    hasher.update(record.n.to_le_bytes());
    hasher.update([record.status]);
    hasher.update(record.p.to_le_bytes());
    hasher.update(record.q.to_le_bytes());
    hasher.update(record.q_candidates_tested.to_le_bytes());
}

pub fn hash_result_records<I>(records: I) -> [u8; 32]
where
    I: IntoIterator<Item = ResultRecord>,
{
    let mut hasher = Sha256::new();
    hasher.update(b"A220091-CHUNK-RESULT-v1\0");
    for record in records {
        update_result_hash(&mut hasher, record);
    }
    hasher.finalize().into()
}

pub fn initial_result_root(
    start: u64,
    end: u64,
    mode: &str,
    chunk_size: u64,
    prime_limit: u64,
    q_hash: [u8; 32],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"A220091-RESULT-ROOT-v1\0");
    hasher.update(HASH_SCHEMA_VERSION.to_le_bytes());
    hasher.update(start.to_le_bytes());
    hasher.update(end.to_le_bytes());
    hasher.update((mode.len() as u64).to_le_bytes());
    hasher.update(mode.as_bytes());
    hasher.update(chunk_size.to_le_bytes());
    hasher.update(prime_limit.to_le_bytes());
    hasher.update(q_hash);
    hasher.finalize().into()
}

pub fn extend_result_root(root: [u8; 32], chunk_hash: [u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(root);
    hasher.update(chunk_hash);
    hasher.finalize().into()
}

pub fn hex_digest(digest: [u8; 32]) -> String {
    let mut result = String::with_capacity(64);
    for byte in digest {
        result.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
        result.push(char::from(b"0123456789abcdef"[(byte & 0x0f) as usize]));
    }
    result
}

pub fn parse_hex_digest(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = (hex_value(pair[0])? << 4) | hex_value(pair[1])?;
    }
    Some(digest)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_and_root_hashes_are_deterministic() {
        let records = [
            ResultRecord {
                n: 31,
                status: 1,
                p: 17,
                q: 7,
                q_candidates_tested: 4,
            },
            ResultRecord {
                n: 32,
                status: 1,
                p: 29,
                q: 3,
                q_candidates_tested: 2,
            },
        ];
        assert_eq!(hash_result_records(records), hash_result_records(records));
        let q_hash = [7_u8; 32];
        let first = initial_result_root(1, 32, "full-scan", 1_000_000, 301, q_hash);
        let second = initial_result_root(1, 32, "full-scan", 1_000_000, 301, q_hash);
        assert_eq!(first, second);
        assert_eq!(
            extend_result_root(first, hash_result_records(records)),
            extend_result_root(first, hash_result_records(records))
        );
    }

    #[test]
    fn q_list_hash_stays_on_v1_wire_format_when_maximum_q_fits_u32() {
        let q_list = QList::from_raw_parts_for_test(20, vec![2, 3, 5, 7, 17]);
        let mut expected = Sha256::new();
        expected.update(b"A220091-Q-LIST-v1\0");
        for &q in q_list.as_slice() {
            expected.update((q as u32).to_le_bytes());
        }
        let expected: [u8; 32] = expected.finalize().into();
        assert_eq!(hash_q_list(&q_list), expected);
    }

    #[test]
    fn q_list_hash_switches_to_v2_wire_format_above_u32_max() {
        let above_u32_max = u64::from(u32::MAX) + 5;
        let q_list = QList::from_raw_parts_for_test(above_u32_max, vec![2, 3, above_u32_max]);
        let mut expected = Sha256::new();
        expected.update(b"A220091-Q-LIST-v2\0");
        for &q in q_list.as_slice() {
            expected.update(q.to_le_bytes());
        }
        let expected: [u8; 32] = expected.finalize().into();
        assert_eq!(hash_q_list(&q_list), expected);
        assert_eq!(hash_q_list(&q_list), hash_q_list(&q_list));

        // The v2 record encoding must differ from a naive v1-style truncation.
        let fits_u32 = QList::from_raw_parts_for_test(20, vec![2, 3, 5, 7, 17]);
        assert_ne!(hash_q_list(&q_list), hash_q_list(&fits_u32));
    }
}
