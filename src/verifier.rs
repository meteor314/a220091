use std::fmt;

use crate::{q_upper_bound, PrimeOracle, QList, Representation};

/// Errors caused by inputs that cannot fully cover the requested verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationError {
    PrimeBitmapLimitTooSmall { required: u64, available: u64 },
    QListLimitTooSmall { required: u64, available: u64 },
    ArithmeticOverflow,
}

impl fmt::Display for VerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PrimeBitmapLimitTooSmall {
                required,
                available,
            } => write!(
                formatter,
                "prime bitmap limit {available} is below required limit {required}"
            ),
            Self::QListLimitTooSmall {
                required,
                available,
            } => write!(
                formatter,
                "Q-list limit {available} is below required limit {required}"
            ),
            Self::ArithmeticOverflow => formatter.write_str("verification arithmetic overflow"),
        }
    }
}

impl std::error::Error for VerificationError {}

/// The result and candidate count for one existence verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerificationResult {
    pub representation: Option<Representation>,
    pub q_candidates_tested: u64,
}

/// Find the first valid representation of `n` using the bitmap and Q-list.
pub fn find_representation(
    n: u64,
    primes: &mut PrimeOracle,
    q_list: &QList,
) -> Result<VerificationResult, VerificationError> {
    let q_max = q_upper_bound(n);
    if q_max > q_list.maximum_q() {
        return Err(VerificationError::QListLimitTooSmall {
            required: q_max,
            available: q_list.maximum_q(),
        });
    }

    let q_prefix = q_list.prefix_up_to(q_max);
    if q_prefix.is_empty() {
        return Ok(VerificationResult {
            representation: None,
            q_candidates_tested: 0,
        });
    }

    let first_q = q_prefix[0];
    let first_q_term = if n % 2 == 0 {
        first_q
    } else {
        first_q
            .checked_mul(2)
            .ok_or(VerificationError::ArithmeticOverflow)?
    };
    let largest_candidate_p = n
        .checked_sub(first_q_term)
        .ok_or(VerificationError::ArithmeticOverflow)?;
    if largest_candidate_p > primes.limit() {
        return Err(VerificationError::PrimeBitmapLimitTooSmall {
            required: largest_candidate_p,
            available: primes.limit(),
        });
    }

    if n % 2 == 0 {
        verify_even(n, q_prefix, primes)
    } else {
        verify_odd(n, q_prefix, primes)
    }
}

fn verify_even(
    n: u64,
    q_prefix: &[u64],
    primes: &mut PrimeOracle,
) -> Result<VerificationResult, VerificationError> {
    let mut q_candidates_tested = 0;
    for &q in q_prefix {
        q_candidates_tested += 1;
        let p = n
            .checked_sub(q)
            .ok_or(VerificationError::ArithmeticOverflow)?;

        if p > q && primes.is_prime_bounded(p) {
            return Ok(VerificationResult {
                representation: Some(Representation { n, p, q }),
                q_candidates_tested,
            });
        }
    }

    Ok(VerificationResult {
        representation: None,
        q_candidates_tested,
    })
}

fn verify_odd(
    n: u64,
    q_prefix: &[u64],
    primes: &mut PrimeOracle,
) -> Result<VerificationResult, VerificationError> {
    let mut q_candidates_tested = 0;
    for &q in q_prefix {
        q_candidates_tested += 1;
        let q_term = q
            .checked_mul(2)
            .ok_or(VerificationError::ArithmeticOverflow)?;
        let p = n
            .checked_sub(q_term)
            .ok_or(VerificationError::ArithmeticOverflow)?;

        if p > q && primes.is_prime_bounded(p) {
            return Ok(VerificationResult {
                representation: Some(Representation { n, p, q }),
                q_candidates_tested,
            });
        }
    }

    Ok(VerificationResult {
        representation: None,
        q_candidates_tested,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_representation_handles_q_max_above_u32_max_without_overflow() {
        // n just above 2 * u32::MAX forces q_upper_bound(n) past u32::MAX for
        // even n. The Q-list and prime-source size checks must still report
        // legitimate "too small" errors instead of panicking or overflowing.
        let above_u32_max = u64::from(u32::MAX) + 11;
        let n = above_u32_max
            .checked_mul(2)
            .and_then(|value| value.checked_add(2))
            .unwrap();
        assert_eq!(q_upper_bound(n), above_u32_max);

        let q_list = QList::from_raw_parts_for_test(above_u32_max, vec![2, 3, above_u32_max]);
        let mut small_oracle = PrimeOracle::new(12, 32, 2).unwrap();

        match find_representation(n, &mut small_oracle, &q_list) {
            Err(VerificationError::PrimeBitmapLimitTooSmall {
                required,
                available,
            }) => {
                assert_eq!(available, 12);
                assert_eq!(required, n - 2); // n minus the smallest admissible q (2)
            }
            other => panic!("expected PrimeBitmapLimitTooSmall, got {other:?}"),
        }
    }

    #[test]
    fn find_representation_reports_q_list_limit_too_small_above_u32_max() {
        let above_u32_max = u64::from(u32::MAX) + 11;
        let n = above_u32_max
            .checked_mul(2)
            .and_then(|value| value.checked_add(2))
            .unwrap();

        let undersized_q_list = QList::from_raw_parts_for_test(above_u32_max - 1, vec![2, 3]);
        let mut small_oracle = PrimeOracle::new(12, 32, 2).unwrap();

        assert_eq!(
            find_representation(n, &mut small_oracle, &undersized_q_list),
            Err(VerificationError::QListLimitTooSmall {
                required: above_u32_max,
                available: above_u32_max - 1,
            })
        );
    }
}
