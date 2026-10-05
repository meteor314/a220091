use std::fmt;

/// Errors that can occur while creating a prime bitmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimeBitmapError {
    LimitTooLarge,
    AllocationFailed,
}

impl fmt::Display for PrimeBitmapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitTooLarge => formatter.write_str("prime bitmap limit is too large"),
            Self::AllocationFailed => formatter.write_str("prime bitmap allocation failed"),
        }
    }
}

impl std::error::Error for PrimeBitmapError {}

/// Compact inclusive prime lookup table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimeBitmap {
    limit: u64,
    bits: Vec<u64>,
}

impl PrimeBitmap {
    /// Build a bitmap containing primality information for `0..=limit`.
    pub fn from_limit(limit: u64) -> Result<Self, PrimeBitmapError> {
        let word_count = limit
            .checked_div(64)
            .and_then(|word| word.checked_add(1))
            .and_then(|word_count| usize::try_from(word_count).ok())
            .ok_or(PrimeBitmapError::LimitTooLarge)?;

        let mut bits = Vec::new();
        bits.try_reserve_exact(word_count)
            .map_err(|_| PrimeBitmapError::AllocationFailed)?;
        bits.resize(word_count, u64::MAX);

        let mut bitmap = Self { limit, bits };
        bitmap.clear(0);
        if limit >= 1 {
            bitmap.clear(1);
        }

        let mut candidate = 2;
        while candidate <= limit / candidate {
            if bitmap.contains(candidate) {
                let mut composite = candidate * candidate;
                while composite <= limit {
                    bitmap.clear(composite);
                    composite = match composite.checked_add(candidate) {
                        Some(value) => value,
                        None => break,
                    };
                }
            }
            candidate += 1;
        }

        Ok(bitmap)
    }

    /// Return the inclusive upper bound represented by this bitmap.
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// Return whether `value` is prime and within the represented range.
    pub fn is_prime(&self, value: u64) -> bool {
        value <= self.limit && self.contains(value)
    }

    fn contains(&self, value: u64) -> bool {
        if value > self.limit {
            return false;
        }

        let word_index = usize::try_from(value / 64).expect("bitmap word index is bounded");
        let bit_index = value & 63;
        (self.bits[word_index] & (1u64 << bit_index)) != 0
    }

    fn clear(&mut self, value: u64) {
        let word_index = usize::try_from(value / 64).expect("bitmap word index is bounded");
        let bit_index = value & 63;
        self.bits[word_index] &= !(1u64 << bit_index);
    }
}
