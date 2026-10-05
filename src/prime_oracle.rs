use std::fmt;
use std::sync::Arc;

use crate::PrimeBitmap;

/// Default number of consecutive integers covered by one cached segment.
/// ~131 KiB per segment (`DEFAULT_SEGMENT_SIZE / 64 * 8` bytes).
pub const DEFAULT_SEGMENT_SIZE: u64 = 1 << 20;

/// Default number of segments kept resident at once (LRU-evicted).
pub const DEFAULT_CACHE_SEGMENTS: usize = 8;

/// Errors that can occur while constructing a `PrimeOracle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimeOracleError {
    InvalidSegmentSize,
    InvalidCacheCapacity,
    BasePrimeSieveFailed,
}

impl fmt::Display for PrimeOracleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSegmentSize => {
                formatter.write_str("segment size must be greater than zero")
            }
            Self::InvalidCacheCapacity => {
                formatter.write_str("cache capacity must be greater than zero")
            }
            Self::BasePrimeSieveFailed => {
                formatter.write_str("failed to sieve base primes up to sqrt(limit)")
            }
        }
    }
}

impl std::error::Error for PrimeOracleError {}

/// A bit-packed primality record for one contiguous `[lo, hi]` window.
#[derive(Debug, Clone)]
struct Segment {
    lo: u64,
    hi: u64,
    bits: Vec<u64>,
    last_used: u64,
}

impl Segment {
    fn contains(&self, value: u64) -> bool {
        let offset = (value - self.lo) as usize;
        let word = offset / 64;
        let bit = offset % 64;
        (self.bits[word] >> bit) & 1 == 1
    }

    fn clear(&mut self, value: u64) {
        let offset = (value - self.lo) as usize;
        let word = offset / 64;
        let bit = offset % 64;
        self.bits[word] &= !(1_u64 << bit);
    }
}

/// A segmented, bounded-memory primality oracle over `2..=limit`.
///
/// Unlike a single global bitmap (which needs `O(limit)` memory), this
/// sieves and caches only a fixed number of fixed-size windows on demand,
/// using a small precomputed base-prime list (`O(sqrt(limit))`, itself tiny
/// compared to `limit`). Total resident memory is therefore bounded by
/// `segment_size * cache_capacity`, independent of `limit`/`END`.
///
/// Cloning is cheap: the base-prime list is shared (`Arc`) and the cache
/// starts empty, so each parallel worker can hold an independent, mutable
/// working copy without re-sieving base primes.
#[derive(Debug, Clone)]
pub struct PrimeOracle {
    base_primes: Arc<Vec<u64>>,
    limit: u64,
    segment_size: u64,
    cache_capacity: usize,
    cache: Vec<Segment>,
    clock: u64,
}

impl PrimeOracle {
    /// Build an oracle covering `2..=limit` using fixed-size segments.
    pub fn new(
        limit: u64,
        segment_size: u64,
        cache_capacity: usize,
    ) -> Result<Self, PrimeOracleError> {
        if segment_size == 0 {
            return Err(PrimeOracleError::InvalidSegmentSize);
        }
        if cache_capacity == 0 {
            return Err(PrimeOracleError::InvalidCacheCapacity);
        }

        let sqrt_limit = integer_sqrt(limit);
        let base_bitmap = PrimeBitmap::from_limit(sqrt_limit)
            .map_err(|_| PrimeOracleError::BasePrimeSieveFailed)?;
        let base_primes: Vec<u64> = (2..=sqrt_limit)
            .filter(|&value| base_bitmap.is_prime(value))
            .collect();

        Ok(Self {
            base_primes: Arc::new(base_primes),
            limit,
            segment_size,
            cache_capacity,
            cache: Vec::with_capacity(cache_capacity),
            clock: 0,
        })
    }

    /// Build an oracle with the default segment size and cache capacity.
    pub fn with_defaults(limit: u64) -> Result<Self, PrimeOracleError> {
        Self::new(limit, DEFAULT_SEGMENT_SIZE, DEFAULT_CACHE_SEGMENTS)
    }

    /// Return the inclusive upper bound this oracle can answer queries for.
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// Return the number of integers covered by one cached segment.
    pub fn segment_size(&self) -> u64 {
        self.segment_size
    }

    /// Return the maximum number of segments kept resident at once.
    pub fn cache_capacity(&self) -> usize {
        self.cache_capacity
    }

    /// Return the worst-case resident memory for base primes plus a fully
    /// populated segment cache, in bytes. This bound does not grow with
    /// `limit` beyond the `O(sqrt(limit))` base-prime list.
    pub fn approximate_memory_bytes(&self) -> u64 {
        let base_primes_bytes = (self.base_primes.len() as u64) * 8;
        let words_per_segment = self.segment_size.div_ceil(64);
        let cache_bytes = words_per_segment * 8 * self.cache_capacity as u64;
        base_primes_bytes + cache_bytes
    }

    /// Return whether `value` is prime, treating values outside `2..=limit`
    /// as not prime (mirrors `PrimeBitmap::is_prime`).
    pub fn is_prime(&mut self, value: u64) -> bool {
        if value < 2 || value > self.limit {
            return false;
        }
        self.is_prime_bounded(value)
    }

    /// Return whether `value` is prime. The caller must ensure
    /// `value <= limit()`.
    pub fn is_prime_bounded(&mut self, value: u64) -> bool {
        debug_assert!(value <= self.limit);
        if value < 2 {
            return false;
        }
        if let Some(index) = self
            .cache
            .iter()
            .position(|segment| segment.lo <= value && value <= segment.hi)
        {
            self.clock += 1;
            self.cache[index].last_used = self.clock;
            return self.cache[index].contains(value);
        }

        let segment = self.sieve_segment_containing(value);
        let result = segment.contains(value);
        self.insert_segment(segment);
        result
    }

    fn sieve_segment_containing(&self, value: u64) -> Segment {
        let segment_index = value / self.segment_size;
        let lo = segment_index * self.segment_size;
        let hi = lo.saturating_add(self.segment_size - 1).min(self.limit);
        self.sieve_segment(lo, hi)
    }

    fn sieve_segment(&self, lo: u64, hi: u64) -> Segment {
        let word_count = ((hi - lo) / 64 + 1) as usize;
        let mut segment = Segment {
            lo,
            hi,
            bits: vec![u64::MAX; word_count],
            last_used: 0,
        };

        if lo == 0 {
            segment.clear(0);
            if hi >= 1 {
                segment.clear(1);
            }
        } else if lo == 1 {
            segment.clear(1);
        }

        for &p in self.base_primes.iter() {
            let square = match p.checked_mul(p) {
                Some(value) => value,
                None => break,
            };
            if square > hi {
                break;
            }
            let start = if square >= lo {
                square
            } else {
                let remainder = lo % p;
                if remainder == 0 {
                    lo
                } else {
                    lo + (p - remainder)
                }
            };
            let mut multiple = start;
            while multiple <= hi {
                segment.clear(multiple);
                multiple = match multiple.checked_add(p) {
                    Some(next) => next,
                    None => break,
                };
            }
        }

        segment
    }

    fn insert_segment(&mut self, mut segment: Segment) {
        self.clock += 1;
        segment.last_used = self.clock;
        if self.cache.len() >= self.cache_capacity {
            let evict_index = self
                .cache
                .iter()
                .enumerate()
                .min_by_key(|(_, cached)| cached.last_used)
                .map(|(index, _)| index)
                .expect("cache_capacity is greater than zero");
            self.cache.remove(evict_index);
        }
        self.cache.push(segment);
    }
}

fn integer_sqrt(value: u64) -> u64 {
    if value < 2 {
        return value;
    }
    let mut x = (value as f64).sqrt() as u64;
    while x > 0 && x.checked_mul(x).is_none_or(|square| square > value) {
        x -= 1;
    }
    while x
        .checked_add(1)
        .and_then(|next| next.checked_mul(next))
        .is_some_and(|square| square <= value)
    {
        x += 1;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_sqrt_matches_expected_boundaries() {
        assert_eq!(integer_sqrt(0), 0);
        assert_eq!(integer_sqrt(1), 1);
        assert_eq!(integer_sqrt(3), 1);
        assert_eq!(integer_sqrt(4), 2);
        assert_eq!(integer_sqrt(15), 3);
        assert_eq!(integer_sqrt(16), 4);
        assert_eq!(integer_sqrt(u64::from(u32::MAX)), 65535);
    }

    #[test]
    fn matches_reference_primality_on_a_substantial_small_range() {
        let limit = 10_000;
        let mut oracle = PrimeOracle::new(limit, 97, 3).unwrap();
        for value in 0..=limit {
            assert_eq!(
                oracle.is_prime(value),
                crate::is_prime(value),
                "value={value}"
            );
        }
    }

    #[test]
    fn matches_reference_primality_across_the_u32_max_boundary() {
        // Regression test for q > u32::MAX: sieve a real window straddling
        // 2^32 - 1 and cross-check every value against the independent
        // trial-division reference, without allocating a bitmap over the
        // full range (only ~65536 base primes plus a handful of segments).
        let above_u32_max = u64::from(u32::MAX) + 40;
        let below_u32_max = u64::from(u32::MAX) - 40;
        let mut oracle = PrimeOracle::new(above_u32_max, 1024, 4).unwrap();
        for value in below_u32_max..=above_u32_max {
            assert_eq!(
                oracle.is_prime(value),
                crate::is_prime(value),
                "value={value}"
            );
        }
        // 4_294_967_311 is a well-known prime just above u32::MAX.
        assert!(oracle.is_prime(4_294_967_311));
    }

    #[test]
    fn small_segment_size_forces_repeated_eviction_and_stays_correct() {
        let limit = 5_000;
        let mut oracle = PrimeOracle::new(limit, 16, 1).unwrap();
        for value in (0..=limit).rev() {
            assert_eq!(
                oracle.is_prime(value),
                crate::is_prime(value),
                "value={value}"
            );
        }
    }

    #[test]
    fn out_of_range_values_are_not_prime() {
        let mut oracle = PrimeOracle::new(100, 32, 2).unwrap();
        assert!(!oracle.is_prime(101));
        assert!(!oracle.is_prime(u64::MAX));
    }

    #[test]
    fn memory_estimate_is_bounded_by_segment_size_and_cache_capacity() {
        let oracle = PrimeOracle::new(1_000_000_000, 1 << 20, 8).unwrap();
        // 8 segments of 1<<20 bits (131_072 bytes each) plus a small base-prime list.
        assert!(oracle.approximate_memory_bytes() < 2 * 1024 * 1024);
    }

    #[test]
    fn memory_stays_bounded_at_the_production_scale_that_previously_swapped() {
        // 3 * 19_900_000_000 + 1: the prime limit that made the old global
        // bitmap require ~7.46 GB and start swapping on the N100. Only the
        // base-prime list (O(sqrt(limit))) plus a fixed segment cache are
        // built here; no allocation is proportional to the limit itself.
        let production_scale_limit = 3 * 19_900_000_000_u64 + 1;
        let oracle = PrimeOracle::with_defaults(production_scale_limit).unwrap();
        assert!(
            oracle.approximate_memory_bytes() < 16 * 1024 * 1024,
            "expected a few MB, got {} bytes",
            oracle.approximate_memory_bytes()
        );
    }

    #[test]
    fn rejects_invalid_configuration() {
        assert_eq!(
            PrimeOracle::new(100, 0, 4).unwrap_err(),
            PrimeOracleError::InvalidSegmentSize
        );
        assert_eq!(
            PrimeOracle::new(100, 32, 0).unwrap_err(),
            PrimeOracleError::InvalidCacheCapacity
        );
    }
}
