use std::fmt;

use rayon::prelude::*;

use crate::{verifier, PrimeOracle, QList};

/// Errors raised before or during parallel range verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParallelVerificationError {
    InvalidRange,
    InvalidChunkSize,
    InvalidThreadCount,
    ArithmeticOverflow,
    Verification(verifier::VerificationError),
}

impl fmt::Display for ParallelVerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange => formatter.write_str("parallel verification range is invalid"),
            Self::InvalidChunkSize => {
                formatter.write_str("parallel verification chunk size is zero")
            }
            Self::InvalidThreadCount => {
                formatter.write_str("parallel verification thread count is zero")
            }
            Self::ArithmeticOverflow => {
                formatter.write_str("parallel verification arithmetic overflow")
            }
            Self::Verification(error) => write!(formatter, "verification failed: {error}"),
        }
    }
}

impl std::error::Error for ParallelVerificationError {}

/// Deterministic result for one numeric chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkVerificationResult {
    pub start: u64,
    pub end: u64,
    pub n_processed: u64,
    pub total_q_tests: u64,
    pub zero_solution_n: Vec<u64>,
}

/// Deterministic aggregate result for an inclusive range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelVerificationResult {
    pub start: u64,
    pub end: u64,
    pub n_processed: u64,
    pub total_q_tests: u64,
    pub zero_solution_n: Vec<u64>,
    pub chunks: Vec<ChunkVerificationResult>,
}

/// Verify an inclusive range in independent numeric chunks using Rayon.
///
/// `primes` is cloned once per chunk so each worker gets its own mutable
/// segment cache; cloning is cheap (shared base-prime list, empty cache).
pub fn verify_range_parallel(
    start: u64,
    end: u64,
    primes: &PrimeOracle,
    q_list: &QList,
    threads: usize,
    chunk_size: u64,
) -> Result<ParallelVerificationResult, ParallelVerificationError> {
    if start > end {
        return Err(ParallelVerificationError::InvalidRange);
    }
    if threads == 0 {
        return Err(ParallelVerificationError::InvalidThreadCount);
    }
    if chunk_size == 0 {
        return Err(ParallelVerificationError::InvalidChunkSize);
    }

    let ranges = chunk_ranges(start, end, chunk_size)?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|_| ParallelVerificationError::InvalidThreadCount)?;

    let mut chunks = pool.install(|| {
        ranges
            .par_iter()
            .map(|&(chunk_start, chunk_end)| verify_chunk(chunk_start, chunk_end, primes, q_list))
            .collect::<Result<Vec<_>, _>>()
    })?;
    chunks.sort_unstable_by_key(|chunk| chunk.start);

    let mut n_processed = 0_u64;
    let mut total_q_tests = 0_u64;
    let mut zero_solution_n = Vec::new();
    for chunk in &chunks {
        n_processed = n_processed
            .checked_add(chunk.n_processed)
            .ok_or(ParallelVerificationError::ArithmeticOverflow)?;
        total_q_tests = total_q_tests
            .checked_add(chunk.total_q_tests)
            .ok_or(ParallelVerificationError::ArithmeticOverflow)?;
        zero_solution_n.extend_from_slice(&chunk.zero_solution_n);
    }

    Ok(ParallelVerificationResult {
        start,
        end,
        n_processed,
        total_q_tests,
        zero_solution_n,
        chunks,
    })
}

fn chunk_ranges(
    start: u64,
    end: u64,
    chunk_size: u64,
) -> Result<Vec<(u64, u64)>, ParallelVerificationError> {
    let mut ranges = Vec::new();
    let mut chunk_start = start;
    loop {
        let chunk_end = chunk_start
            .checked_add(chunk_size - 1)
            .unwrap_or(end)
            .min(end);
        ranges.push((chunk_start, chunk_end));
        if chunk_end == end {
            break;
        }
        chunk_start = chunk_end
            .checked_add(1)
            .ok_or(ParallelVerificationError::ArithmeticOverflow)?;
    }
    Ok(ranges)
}

fn verify_chunk(
    start: u64,
    end: u64,
    primes: &PrimeOracle,
    q_list: &QList,
) -> Result<ChunkVerificationResult, ParallelVerificationError> {
    let mut primes = primes.clone();
    let mut n_processed = 0_u64;
    let mut total_q_tests = 0_u64;
    let mut zero_solution_n = Vec::new();
    let mut n = start;

    loop {
        let result = verifier::find_representation(n, &mut primes, q_list)
            .map_err(ParallelVerificationError::Verification)?;
        n_processed = n_processed
            .checked_add(1)
            .ok_or(ParallelVerificationError::ArithmeticOverflow)?;
        total_q_tests = total_q_tests
            .checked_add(result.q_candidates_tested)
            .ok_or(ParallelVerificationError::ArithmeticOverflow)?;
        if result.representation.is_none() {
            zero_solution_n.push(n);
        }

        if n == end {
            break;
        }
        n = n
            .checked_add(1)
            .ok_or(ParallelVerificationError::ArithmeticOverflow)?;
    }

    Ok(ChunkVerificationResult {
        start,
        end,
        n_processed,
        total_q_tests,
        zero_solution_n,
    })
}
