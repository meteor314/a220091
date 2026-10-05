use std::fmt;

use crate::PrimeOracle;

/// Errors that can occur while constructing the admissible Q-list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QFilterError {
    ArithmeticOverflow,
    BitmapLimitTooSmall { required: u64, available: u64 },
}

impl fmt::Display for QFilterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow => formatter.write_str("Q arithmetic overflow"),
            Self::BitmapLimitTooSmall {
                required,
                available,
            } => write!(
                formatter,
                "prime bitmap limit {available} is below required limit {required}"
            ),
        }
    }
}

impl std::error::Error for QFilterError {}

/// Sorted admissible q values.
///
/// Stored as `u64` because the mathematical q bound (`N / 2` for even `N`) can
/// exceed `u32::MAX` once `N` grows beyond roughly `8.6e9`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QList {
    maximum_q: u64,
    values: Vec<u64>,
}

impl QList {
    /// Return all stored q values in strictly increasing order.
    pub fn as_slice(&self) -> &[u64] {
        &self.values
    }

    /// Return the stored q prefix satisfying `q <= max_q` without allocating.
    pub fn prefix_up_to(&self, max_q: u64) -> &[u64] {
        let end = self.values.partition_point(|&q| q <= max_q);
        &self.values[..end]
    }

    /// Return the number of stored q values.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Return whether the list contains no q values.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Return the maximum q range covered by this list construction.
    pub fn maximum_q(&self) -> u64 {
        self.maximum_q
    }

    /// Build a `QList` directly from precomputed values without a prime
    /// bitmap, so tests can exercise `q > u32::MAX` behavior without
    /// allocating a bitmap covering billions of bits.
    #[cfg(test)]
    pub(crate) fn from_raw_parts_for_test(maximum_q: u64, values: Vec<u64>) -> Self {
        Self { maximum_q, values }
    }
}

/// Construct the admissible Q-list for `2..=max_q` using `primes`.
pub fn build_q_list(max_q: u64, primes: &mut PrimeOracle) -> Result<QList, QFilterError> {
    let six_max_q = max_q
        .checked_mul(6)
        .ok_or(QFilterError::ArithmeticOverflow)?;
    let required_limit = six_max_q
        .checked_add(1)
        .ok_or(QFilterError::ArithmeticOverflow)?;

    if max_q >= 2 && primes.limit() < required_limit {
        return Err(QFilterError::BitmapLimitTooSmall {
            required: required_limit,
            available: primes.limit(),
        });
    }

    let mut values = Vec::new();
    let mut q = 2_u64;
    while q <= max_q {
        let six_q = q.checked_mul(6).ok_or(QFilterError::ArithmeticOverflow)?;
        let lower = six_q
            .checked_sub(1)
            .ok_or(QFilterError::ArithmeticOverflow)?;
        let upper = six_q
            .checked_add(1)
            .ok_or(QFilterError::ArithmeticOverflow)?;

        if primes.is_prime(q) && primes.is_prime(lower) && primes.is_prime(upper) {
            values.push(q);
        }

        q = match q.checked_add(1) {
            Some(next) => next,
            None => break,
        };
    }

    Ok(QList {
        maximum_q: max_q,
        values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_q_list_rejects_bitmap_too_small_instead_of_u32_ceiling_beyond_u32_max() {
        // max_q above u32::MAX (4_294_967_295) previously failed with
        // QFilterError::MaximumQTooLarge before this could even consult the
        // bitmap. It must now fail only due to the (legitimate, unrelated)
        // bitmap size limitation, proving the u32 ceiling is gone.
        let max_q = u64::from(u32::MAX) + 1_000;
        let mut small_oracle = PrimeOracle::new(12, 32, 2).unwrap();
        let required = max_q * 6 + 1;
        assert_eq!(
            build_q_list(max_q, &mut small_oracle),
            Err(QFilterError::BitmapLimitTooSmall {
                required,
                available: 12,
            })
        );
    }

    #[test]
    fn q_list_stores_and_returns_values_above_u32_max() {
        let above_u32_max = u64::from(u32::MAX) + 5;
        let q_list = QList::from_raw_parts_for_test(above_u32_max, vec![2, 3, above_u32_max]);

        assert_eq!(q_list.as_slice(), &[2, 3, above_u32_max]);
        assert_eq!(q_list.maximum_q(), above_u32_max);
        assert_eq!(q_list.prefix_up_to(above_u32_max), &[2, 3, above_u32_max]);
        assert_eq!(q_list.prefix_up_to(above_u32_max - 1), &[2, 3]);
    }
}
