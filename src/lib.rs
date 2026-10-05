//! Simple reference implementation for OEIS A220091.

pub mod campaign;
pub mod hashing;
pub mod parallel;
pub mod prime_bitmap;
pub mod prime_oracle;
pub mod q_filter;
pub mod verifier;

pub use parallel::{
    verify_range_parallel, ChunkVerificationResult, ParallelVerificationError,
    ParallelVerificationResult,
};
pub use prime_bitmap::{PrimeBitmap, PrimeBitmapError};
pub use prime_oracle::{PrimeOracle, PrimeOracleError};
pub use q_filter::{build_q_list, QFilterError, QList};

pub mod notifier;
/// A valid representation of `n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Representation {
    pub n: u64,
    pub p: u64,
    pub q: u64,
}

/// Return whether `value` is prime.
pub fn is_prime(value: u64) -> bool {
    if value < 2 {
        return false;
    }
    if value == 2 {
        return true;
    }
    if value % 2 == 0 {
        return false;
    }

    let mut divisor = 3;
    while divisor <= value / divisor {
        if value % divisor == 0 {
            return false;
        }
        divisor += 2;
    }
    true
}

/// Return the largest mathematically possible `q` for `n`.
pub fn q_upper_bound(n: u64) -> u64 {
    if n % 2 == 0 {
        (n.saturating_sub(1)) / 2
    } else {
        (n.saturating_sub(1)) / 3
    }
}

/// Return all admissible `q` values up to and including `max_q`.
pub fn admissible_q(max_q: u64) -> Vec<u64> {
    (2..=max_q).filter(|&q| is_admissible_q(q)).collect()
}

/// Return whether `q` satisfies the three prime conditions independent of `n`.
pub fn is_admissible_q(q: u64) -> bool {
    let six_q = match q.checked_mul(6) {
        Some(value) => value,
        None => return false,
    };
    let lower = match six_q.checked_sub(1) {
        Some(value) => value,
        None => return false,
    };
    let upper = match six_q.checked_add(1) {
        Some(value) => value,
        None => return false,
    };

    is_prime(q) && is_prime(lower) && is_prime(upper)
}

/// Return every valid representation of `n`, in ascending `q` order.
pub fn representations(n: u64) -> Vec<Representation> {
    let mut result = Vec::new();
    for q in admissible_q(q_upper_bound(n)) {
        let p = if n % 2 == 0 { n - q } else { n - 2 * q };

        if p > q && is_prime(p) {
            result.push(Representation { n, p, q });
        }
    }
    result
}

/// Return the first valid representation of `n`, if one exists.
pub fn find_representation(n: u64) -> Option<Representation> {
    representations(n).into_iter().next()
}

/// Return the exact number of valid representations of `n`.
pub fn count_representations(n: u64) -> usize {
    representations(n).len()
}
