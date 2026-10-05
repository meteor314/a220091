//! SQLite-backed campaign state for OEIS A220091 verification.
//!
//! The SQLite database is the sole persistent state of a campaign: configuration,
//! completed chunks, completed blocks, per-chunk result hashes, zero-solution
//! findings, cumulative counters, the contiguous frontier (`current_position`) and the
//! runtime timer.
//!
//! Bounded memory: the campaign `[range_start, range_end]` is processed as consecutive
//! verification BLOCKS of `block_size` values (the legacy production campaign used
//! 100,000,000). For each block the prime oracle and Q-list are built for that block's
//! END only (`prime_limit = block_end * 3 + 1`, `maximum_q = (block_end - 1) / 2`,
//! exactly as the benchmark computes them for a run ending at `block_end`) and are
//! dropped before the next block starts. Nothing campaign-wide is held in memory.
//!
//! Durability rules:
//! * A chunk is committed together with every counter it affects in ONE transaction.
//!   The transaction that commits a block's last chunk also records the block row and
//!   its block root; the one that commits the campaign's last block also records the
//!   campaign root and COMPLETED status.
//! * Nothing (log line, notification, in-memory counter) reports a chunk as complete
//!   before that transaction has committed.
//! * `synchronous=FULL` in WAL mode makes a committed transaction durable across
//!   power loss, not only across process crashes.

use std::{
    fmt,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

use crate::{
    hashing,
    prime_oracle::{DEFAULT_CACHE_SEGMENTS, DEFAULT_SEGMENT_SIZE},
    verifier, PrimeOracle, QList,
};

pub const SCHEMA_VERSION: i64 = 1;
pub const DEFAULT_HEARTBEAT: Duration = Duration::from_secs(60);
/// A RUNNING campaign whose last runtime update is younger than this is assumed to
/// still be owned by a live process unless recovery is requested explicitly.
pub const STALE_RUNNING_SECONDS: u64 = 180;
pub const EVEN_CONJECTURE_MIN: u64 = 8_070;
pub const ODD_CONJECTURE_MIN: u64 = 18_680;
/// Block size of the legacy production campaign.
pub const DEFAULT_BLOCK_SIZE: u64 = 100_000_000;

const SCHEMA_SQL: &str = "
CREATE TABLE campaign (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    status TEXT NOT NULL CHECK (status IN ('RUNNING', 'PAUSED', 'COMPLETED')),
    range_start INTEGER NOT NULL,
    range_end INTEGER NOT NULL,
    chunk_size INTEGER NOT NULL CHECK (chunk_size > 0),
    block_size INTEGER NOT NULL CHECK (block_size > 0),
    verification_mode TEXT NOT NULL,
    source_version TEXT,
    current_position INTEGER NOT NULL,
    chunks_completed INTEGER NOT NULL DEFAULT 0,
    blocks_completed INTEGER NOT NULL DEFAULT 0,
    n_processed INTEGER NOT NULL DEFAULT 0,
    total_q_tests INTEGER NOT NULL DEFAULT 0,
    zero_solution_count INTEGER NOT NULL DEFAULT 0,
    result_root TEXT,
    elapsed_seconds INTEGER NOT NULL DEFAULT 0,
    run_started_at INTEGER,
    last_runtime_update INTEGER,
    run_token TEXT,
    run_count INTEGER NOT NULL DEFAULT 0,
    crash_recoveries INTEGER NOT NULL DEFAULT 0,
    pause_requested INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (range_start <= range_end),
    CHECK (current_position >= range_start AND current_position <= range_end + 1)
);

CREATE TABLE chunks (
    chunk_start INTEGER PRIMARY KEY,
    chunk_end INTEGER NOT NULL,
    result_sha256 TEXT NOT NULL,
    n_processed INTEGER NOT NULL,
    q_tests INTEGER NOT NULL,
    zero_solution_n TEXT NOT NULL,
    completed_at INTEGER NOT NULL,
    CHECK (chunk_start <= chunk_end)
);

CREATE TABLE blocks (
    block_start INTEGER PRIMARY KEY,
    block_end INTEGER NOT NULL,
    prime_limit INTEGER NOT NULL,
    q_list_sha256 TEXT NOT NULL,
    q_list_len INTEGER NOT NULL,
    block_root TEXT NOT NULL,
    n_processed INTEGER NOT NULL,
    q_tests INTEGER NOT NULL,
    completed_at INTEGER NOT NULL,
    CHECK (block_start <= block_end)
);

CREATE TABLE zero_solutions (
    n INTEGER PRIMARY KEY,
    chunk_start INTEGER NOT NULL,
    in_conjecture_domain INTEGER NOT NULL CHECK (in_conjecture_domain IN (0, 1))
);
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignError {
    Invalid(String),
    Sqlite(String),
    Io(String),
    Verification(verifier::VerificationError),
    ConfigMismatch {
        field: &'static str,
        stored: String,
        requested: String,
    },
    AlreadyCompleted,
    PossiblyActive {
        seconds_since_update: u64,
    },
    LostOwnership,
    Corrupt(String),
    ArithmeticOverflow,
}

impl fmt::Display for CampaignError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(formatter, "invalid campaign request: {reason}"),
            Self::Sqlite(reason) => write!(formatter, "campaign database error: {reason}"),
            Self::Io(reason) => write!(formatter, "campaign I/O error: {reason}"),
            Self::Verification(error) => write!(formatter, "verification failed: {error}"),
            Self::ConfigMismatch {
                field,
                stored,
                requested,
            } => write!(
                formatter,
                "campaign configuration mismatch for {field}: database has {stored}, requested {requested}"
            ),
            Self::AlreadyCompleted => formatter.write_str("campaign is already COMPLETED"),
            Self::PossiblyActive {
                seconds_since_update,
            } => write!(
                formatter,
                "campaign is RUNNING and was updated {seconds_since_update}s ago; another process may still own it (use --recover only if it is certainly dead)"
            ),
            Self::LostOwnership => formatter.write_str(
                "this process no longer owns the campaign run (status or run token changed)",
            ),
            Self::Corrupt(reason) => write!(formatter, "campaign database inconsistent: {reason}"),
            Self::ArithmeticOverflow => formatter.write_str("campaign arithmetic overflow"),
        }
    }
}

impl std::error::Error for CampaignError {}

impl From<rusqlite::Error> for CampaignError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignStatus {
    Running,
    Paused,
    Completed,
}

impl CampaignStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "RUNNING",
            Self::Paused => "PAUSED",
            Self::Completed => "COMPLETED",
        }
    }

    fn parse(value: &str) -> Result<Self, CampaignError> {
        match value {
            "RUNNING" => Ok(Self::Running),
            "PAUSED" => Ok(Self::Paused),
            "COMPLETED" => Ok(Self::Completed),
            other => Err(CampaignError::Corrupt(format!("unknown status {other:?}"))),
        }
    }
}

/// Immutable campaign definition. Everything here influences the result roots or the
/// chunk/block layout, so a resumed campaign must present exactly the same values.
/// The prime limit and Q-list are NOT part of it: they are per-block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignConfig {
    pub range_start: u64,
    pub range_end: u64,
    pub chunk_size: u64,
    pub block_size: u64,
    pub verification_mode: String,
    pub source_version: Option<String>,
}

impl CampaignConfig {
    pub fn new(
        range_start: u64,
        range_end: u64,
        chunk_size: u64,
        block_size: u64,
        verification_mode: &str,
        source_version: Option<String>,
    ) -> Self {
        Self {
            range_start,
            range_end,
            chunk_size,
            block_size,
            verification_mode: verification_mode.to_owned(),
            source_version,
        }
    }

    fn validate(&self) -> Result<(), CampaignError> {
        if self.range_start > self.range_end {
            return Err(CampaignError::Invalid(
                "range start exceeds range end".to_owned(),
            ));
        }
        if self.chunk_size == 0 || self.block_size == 0 {
            return Err(CampaignError::Invalid(
                "chunk size and block size must be greater than zero".to_owned(),
            ));
        }
        // Chunks must never straddle a block, so each block's Q-list covers whole chunks.
        if self.block_size % self.chunk_size != 0 {
            return Err(CampaignError::Invalid(
                "block size must be a multiple of the chunk size".to_owned(),
            ));
        }
        // current_position may reach range_end + 1, which must fit SQLite's INTEGER.
        if self.range_end >= i64::MAX as u64
            || self.chunk_size > i64::MAX as u64
            || self.block_size > i64::MAX as u64
        {
            return Err(CampaignError::Invalid(
                "range, chunk size or block size exceeds SQLite INTEGER".to_owned(),
            ));
        }
        if required_prime_limit(self.range_end)? > i64::MAX as u64 {
            return Err(CampaignError::Invalid(
                "prime limit exceeds SQLite INTEGER".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Prime limit required to verify `[.., end]` (matches the production benchmark).
pub fn required_prime_limit(end: u64) -> Result<u64, CampaignError> {
    end.checked_mul(3)
        .and_then(|value| value.checked_add(1))
        .ok_or(CampaignError::ArithmeticOverflow)
}

/// Largest Q-list bound for `[.., end]` (matches the production benchmark).
pub fn maximum_q_for_range(end: u64) -> u64 {
    end.saturating_sub(1) / 2
}

pub fn is_in_conjecture_domain(n: u64) -> bool {
    if n % 2 == 0 {
        n >= EVEN_CONJECTURE_MIN
    } else {
        n >= ODD_CONJECTURE_MIN
    }
}

/// Number of blocks in `[range_start, range_end]`.
pub fn total_blocks(range_start: u64, range_end: u64, block_size: u64) -> u64 {
    (range_end - range_start) / block_size + 1
}

/// Inclusive bounds of block `index`; the last block is clipped at `range_end`.
pub fn block_bounds(range_start: u64, range_end: u64, block_size: u64, index: u64) -> (u64, u64) {
    let start = range_start + index * block_size;
    (start, start.saturating_add(block_size - 1).min(range_end))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkRecord {
    pub chunk_start: u64,
    pub chunk_end: u64,
    pub result_sha256: String,
    pub n_processed: u64,
    pub q_tests: u64,
    pub zero_solution_n: Vec<u64>,
    pub completed_at: u64,
}

/// What the verifying process knows about the block a chunk belongs to. It is derived
/// from the block's own oracle and Q-list and is recorded when the block completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockContext {
    pub block_start: u64,
    pub block_end: u64,
    pub prime_limit: u64,
    pub q_list_sha256: String,
    pub q_list_len: u64,
}

impl BlockContext {
    pub fn new(block_start: u64, block_end: u64, primes: &PrimeOracle, q_list: &QList) -> Self {
        Self {
            block_start,
            block_end,
            prime_limit: primes.limit(),
            q_list_sha256: hashing::hex_digest(hashing::hash_q_list(q_list)),
            q_list_len: q_list.len() as u64,
        }
    }
}

/// Notified strictly AFTER a chunk's transaction has committed.
pub trait ChunkProgressSink: Send + Sync {
    fn chunk_committed(&self, chunk: &ChunkRecord);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignState {
    pub status: CampaignStatus,
    pub range_start: u64,
    pub range_end: u64,
    pub chunk_size: u64,
    pub block_size: u64,
    pub verification_mode: String,
    pub source_version: Option<String>,
    pub current_position: u64,
    pub chunks_completed: u64,
    pub blocks_completed: u64,
    pub n_processed: u64,
    pub total_q_tests: u64,
    pub zero_solution_count: u64,
    pub result_root: Option<String>,
    pub elapsed_seconds: u64,
    pub run_started_at: Option<i64>,
    pub last_runtime_update: Option<i64>,
    pub run_count: u64,
    pub crash_recoveries: u64,
    pub pause_requested: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl CampaignState {
    pub fn total_blocks(&self) -> u64 {
        total_blocks(self.range_start, self.range_end, self.block_size)
    }
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

fn int(value: u64) -> Result<i64, CampaignError> {
    i64::try_from(value).map_err(|_| CampaignError::ArithmeticOverflow)
}

fn uint(value: i64) -> Result<u64, CampaignError> {
    u64::try_from(value).map_err(|_| CampaignError::Corrupt(format!("negative value {value}")))
}

pub fn open_connection(path: &Path) -> Result<Connection, CampaignError> {
    let connection = Connection::open(path)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.busy_timeout(Duration::from_millis(10_000))?;
    Ok(connection)
}

/// Create the campaign database, or validate that an existing one has the same
/// immutable configuration. Never modifies an existing campaign's state.
pub fn open_or_create(path: &Path, config: &CampaignConfig, now: i64) -> Result<(), CampaignError> {
    config.validate()?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| CampaignError::Io(error.to_string()))?;
    }
    let mut connection = open_connection(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'campaign')",
        [],
        |row| row.get(0),
    )?;
    if exists {
        let version: i64 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version != SCHEMA_VERSION {
            return Err(CampaignError::Corrupt(format!(
                "unsupported schema version {version}"
            )));
        }
        let state = load_state(&transaction)?;
        compare_config(&state, config)?;
    } else {
        transaction.execute_batch(SCHEMA_SQL)?;
        transaction.execute(
            "INSERT INTO campaign (
                id, status, range_start, range_end, chunk_size, block_size, verification_mode,
                source_version, current_position, created_at, updated_at
             ) VALUES (1, 'PAUSED', ?1, ?2, ?3, ?4, ?5, ?6, ?1, ?7, ?7)",
            params![
                int(config.range_start)?,
                int(config.range_end)?,
                int(config.chunk_size)?,
                int(config.block_size)?,
                config.verification_mode,
                config.source_version,
                now,
            ],
        )?;
        transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    transaction.commit()?;
    Ok(())
}

fn compare_config(state: &CampaignState, config: &CampaignConfig) -> Result<(), CampaignError> {
    fn check<T: PartialEq + fmt::Debug>(
        field: &'static str,
        stored: &T,
        requested: &T,
    ) -> Result<(), CampaignError> {
        if stored == requested {
            Ok(())
        } else {
            Err(CampaignError::ConfigMismatch {
                field,
                stored: format!("{stored:?}"),
                requested: format!("{requested:?}"),
            })
        }
    }
    check("range_start", &state.range_start, &config.range_start)?;
    check("range_end", &state.range_end, &config.range_end)?;
    check("chunk_size", &state.chunk_size, &config.chunk_size)?;
    check("block_size", &state.block_size, &config.block_size)?;
    check(
        "verification_mode",
        &state.verification_mode,
        &config.verification_mode,
    )?;
    check(
        "source_version",
        &state.source_version,
        &config.source_version,
    )
}

pub fn load_state(connection: &Connection) -> Result<CampaignState, CampaignError> {
    struct Raw {
        status: String,
        numbers: [u64; 11],
        mode: String,
        source_version: Option<String>,
        result_root: Option<String>,
        run_started_at: Option<i64>,
        last_update: Option<i64>,
        pause: i64,
        created: i64,
        updated: i64,
    }
    let raw = connection
        .query_row(
            "SELECT status, range_start, range_end, chunk_size, block_size, verification_mode,
                    source_version, current_position, chunks_completed, blocks_completed,
                    n_processed, total_q_tests, zero_solution_count, result_root,
                    elapsed_seconds, run_started_at, last_runtime_update, run_count,
                    crash_recoveries, pause_requested, created_at, updated_at
             FROM campaign WHERE id = 1",
            [],
            |row| {
                let unsigned = |index: usize| -> rusqlite::Result<u64> {
                    let value: i64 = row.get(index)?;
                    u64::try_from(value)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
                };
                Ok(Raw {
                    status: row.get(0)?,
                    numbers: [
                        unsigned(1)?,
                        unsigned(2)?,
                        unsigned(3)?,
                        unsigned(4)?,
                        unsigned(7)?,
                        unsigned(8)?,
                        unsigned(9)?,
                        unsigned(10)?,
                        unsigned(11)?,
                        unsigned(12)?,
                        unsigned(14)?,
                    ],
                    mode: row.get(5)?,
                    source_version: row.get(6)?,
                    result_root: row.get(13)?,
                    run_started_at: row.get(15)?,
                    last_update: row.get(16)?,
                    pause: row.get(19)?,
                    created: row.get(20)?,
                    updated: row.get(21)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| CampaignError::Corrupt("campaign row is missing".to_owned()))?;
    let (run_count, crash_recoveries): (i64, i64) = connection.query_row(
        "SELECT run_count, crash_recoveries FROM campaign WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let n = raw.numbers;
    Ok(CampaignState {
        status: CampaignStatus::parse(&raw.status)?,
        range_start: n[0],
        range_end: n[1],
        chunk_size: n[2],
        block_size: n[3],
        verification_mode: raw.mode,
        source_version: raw.source_version,
        current_position: n[4],
        chunks_completed: n[5],
        blocks_completed: n[6],
        n_processed: n[7],
        total_q_tests: n[8],
        zero_solution_count: n[9],
        result_root: raw.result_root,
        elapsed_seconds: n[10],
        run_started_at: raw.run_started_at,
        last_runtime_update: raw.last_update,
        run_count: uint(run_count)?,
        crash_recoveries: uint(crash_recoveries)?,
        pause_requested: raw.pause != 0,
        created_at: raw.created,
        updated_at: raw.updated,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStart {
    pub token: String,
    /// The previous run died while RUNNING and its state was recovered.
    pub recovered_crash: bool,
}

fn new_run_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = Sha256::new();
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_le_bytes(),
    );
    hasher.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    hasher.update(format!("{:?}", thread::current().id()));
    hashing::hex_digest(hasher.finalize().into())[..32].to_owned()
}

/// Take ownership of the campaign for a new run.
///
/// Crash recovery: if the stored status is RUNNING, the previous process did not shut
/// down cleanly. Only time up to `last_runtime_update` was ever persisted, so nothing
/// after it is counted. A RUNNING row updated less than `STALE_RUNNING_SECONDS` ago is
/// treated as possibly still active and refused unless `recover` is set.
pub fn begin_run(
    connection: &mut Connection,
    now: i64,
    recover: bool,
) -> Result<RunStart, CampaignError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let state = load_state(&transaction)?;
    let mut recovered_crash = false;
    match state.status {
        CampaignStatus::Completed => return Err(CampaignError::AlreadyCompleted),
        CampaignStatus::Running => {
            let last = state.last_runtime_update.unwrap_or(state.updated_at);
            let age = now.saturating_sub(last).max(0) as u64;
            if !recover && age < STALE_RUNNING_SECONDS {
                return Err(CampaignError::PossiblyActive {
                    seconds_since_update: age,
                });
            }
            recovered_crash = true;
        }
        CampaignStatus::Paused => {}
    }
    let token = new_run_token();
    transaction.execute(
        "UPDATE campaign SET status = 'RUNNING', run_started_at = ?1, last_runtime_update = ?1,
                run_token = ?2, run_count = run_count + 1,
                crash_recoveries = crash_recoveries + ?3, pause_requested = 0, updated_at = ?1
         WHERE id = 1",
        params![now, token, i64::from(recovered_crash)],
    )?;
    transaction.commit()?;
    Ok(RunStart {
        token,
        recovered_crash,
    })
}

/// Monotonic runtime accounting for one run. Wall-clock jumps cannot change elapsed
/// time; only whole seconds are persisted and the fractional remainder is carried.
#[derive(Debug)]
pub struct RuntimeClock {
    anchor: Instant,
}

impl RuntimeClock {
    pub fn start() -> Self {
        Self {
            anchor: Instant::now(),
        }
    }

    pub fn pending_seconds(&self) -> u64 {
        self.anchor.elapsed().as_secs()
    }

    pub fn consume(&mut self, seconds: u64) {
        self.anchor += Duration::from_secs(seconds);
    }
}

fn assert_ownership(connection: &Connection, token: &str) -> Result<(), CampaignError> {
    let owned: bool = connection.query_row(
        "SELECT status = 'RUNNING' AND run_token = ?1 FROM campaign WHERE id = 1",
        params![token],
        |row| {
            row.get::<_, Option<bool>>(0)
                .map(|value| value.unwrap_or(false))
        },
    )?;
    if owned {
        Ok(())
    } else {
        Err(CampaignError::LostOwnership)
    }
}

/// Persist `seconds` of accumulated runtime and refresh `last_runtime_update`.
pub fn heartbeat_at(
    connection: &Connection,
    token: &str,
    seconds: u64,
    now: i64,
) -> Result<(), CampaignError> {
    let changed = connection.execute(
        "UPDATE campaign SET elapsed_seconds = elapsed_seconds + ?1, last_runtime_update = ?2,
                updated_at = ?2
         WHERE id = 1 AND status = 'RUNNING' AND run_token = ?3",
        params![int(seconds)?, now, token],
    )?;
    if changed == 1 {
        Ok(())
    } else {
        Err(CampaignError::LostOwnership)
    }
}

/// Stop accumulating time: flush runtime and mark the campaign PAUSED.
/// Returns false when the run no longer owns a RUNNING campaign (for example it just
/// COMPLETED), in which case nothing is changed.
pub fn pause_at(
    connection: &Connection,
    token: &str,
    seconds: u64,
    now: i64,
) -> Result<bool, CampaignError> {
    let changed = connection.execute(
        "UPDATE campaign SET status = 'PAUSED', elapsed_seconds = elapsed_seconds + ?1,
                last_runtime_update = ?2, run_started_at = NULL, run_token = NULL,
                pause_requested = 0, updated_at = ?2
         WHERE id = 1 AND status = 'RUNNING' AND run_token = ?3",
        params![int(seconds)?, now, token],
    )?;
    Ok(changed == 1)
}

/// Ask a running campaign (possibly in another process) to stop after its next commit.
pub fn request_pause(path: &Path) -> Result<bool, CampaignError> {
    let connection = open_connection(path)?;
    let changed = connection.execute(
        "UPDATE campaign SET pause_requested = 1 WHERE id = 1 AND status = 'RUNNING'",
        [],
    )?;
    Ok(changed == 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitOutcome {
    pub current_position: u64,
    /// This commit completed its block (the block row and block root were recorded).
    pub block_completed: bool,
    /// This commit completed the whole campaign.
    pub completed: bool,
    pub pause_requested: bool,
}

fn validate_block(state: &CampaignState, block: &BlockContext) -> Result<(), CampaignError> {
    let offset = block
        .block_start
        .checked_sub(state.range_start)
        .ok_or_else(|| CampaignError::Invalid("block starts before the campaign".to_owned()))?;
    let expected_end = block
        .block_start
        .saturating_add(state.block_size - 1)
        .min(state.range_end);
    if block.block_start > state.range_end
        || offset % state.block_size != 0
        || block.block_end != expected_end
        || hashing::parse_hex_digest(&block.q_list_sha256).is_none()
    {
        return Err(CampaignError::Invalid(format!(
            "block {}..{} is not aligned to the campaign block layout",
            block.block_start, block.block_end
        )));
    }
    Ok(())
}

/// Atomically commit one finished chunk with all state it affects. When this chunk
/// completes its block, the block row and block root are recorded in the same
/// transaction; when that completes the campaign, so are the campaign root and
/// COMPLETED status.
pub fn commit_chunk_at(
    connection: &mut Connection,
    token: &str,
    chunk: &ChunkRecord,
    block: &BlockContext,
    seconds: u64,
    now: i64,
) -> Result<CommitOutcome, CampaignError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    assert_ownership(&transaction, token)?;
    let state = load_state(&transaction)?;
    validate_block(&state, block)?;

    let offset = chunk
        .chunk_start
        .checked_sub(state.range_start)
        .ok_or_else(|| CampaignError::Invalid("chunk starts before the campaign".to_owned()))?;
    let expected_end = chunk
        .chunk_start
        .saturating_add(state.chunk_size - 1)
        .min(state.range_end);
    if chunk.chunk_start > state.range_end
        || offset % state.chunk_size != 0
        || chunk.chunk_end != expected_end
    {
        return Err(CampaignError::Invalid(format!(
            "chunk {}..{} is not aligned to the campaign chunk layout",
            chunk.chunk_start, chunk.chunk_end
        )));
    }
    if chunk.chunk_start < block.block_start || chunk.chunk_end > block.block_end {
        return Err(CampaignError::Invalid(format!(
            "chunk {}..{} is outside block {}..{}",
            chunk.chunk_start, chunk.chunk_end, block.block_start, block.block_end
        )));
    }
    if chunk.n_processed != chunk.chunk_end - chunk.chunk_start + 1
        || hashing::parse_hex_digest(&chunk.result_sha256).is_none()
    {
        return Err(CampaignError::Invalid(format!(
            "chunk {} has an invalid count or result hash",
            chunk.chunk_start
        )));
    }

    let zero_json = serde_json::to_string(&chunk.zero_solution_n)
        .map_err(|error| CampaignError::Io(error.to_string()))?;
    transaction.execute(
        "INSERT INTO chunks (chunk_start, chunk_end, result_sha256, n_processed, q_tests,
                             zero_solution_n, completed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            int(chunk.chunk_start)?,
            int(chunk.chunk_end)?,
            chunk.result_sha256,
            int(chunk.n_processed)?,
            int(chunk.q_tests)?,
            zero_json,
            int(chunk.completed_at)?,
        ],
    )?;
    {
        let mut insert = transaction.prepare_cached(
            "INSERT INTO zero_solutions (n, chunk_start, in_conjecture_domain) VALUES (?1, ?2, ?3)",
        )?;
        for &n in &chunk.zero_solution_n {
            insert.execute(params![
                int(n)?,
                int(chunk.chunk_start)?,
                i64::from(is_in_conjecture_domain(n))
            ])?;
        }
    }

    // Advance the contiguous frontier over every already-committed chunk.
    let mut position = state.current_position;
    {
        let mut next =
            transaction.prepare_cached("SELECT chunk_end FROM chunks WHERE chunk_start = ?1")?;
        while let Some(end) = next
            .query_row(params![int(position)?], |row| row.get::<_, i64>(0))
            .optional()?
        {
            position = uint(end)?
                .checked_add(1)
                .ok_or(CampaignError::ArithmeticOverflow)?;
        }
    }

    // The block is complete once its chunks cover every one of its values.
    let (block_n, block_q): (i64, i64) = transaction.query_row(
        "SELECT COALESCE(SUM(n_processed), 0), COALESCE(SUM(q_tests), 0)
         FROM chunks WHERE chunk_start BETWEEN ?1 AND ?2",
        params![int(block.block_start)?, int(block.block_end)?],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let block_completed = uint(block_n)? == block.block_end - block.block_start + 1;
    let mut blocks_completed = state.blocks_completed;
    if block_completed {
        let root = compute_block_root(
            &transaction,
            &state.verification_mode,
            state.chunk_size,
            block,
        )?;
        transaction.execute(
            "INSERT INTO blocks (block_start, block_end, prime_limit, q_list_sha256, q_list_len,
                                 block_root, n_processed, q_tests, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                int(block.block_start)?,
                int(block.block_end)?,
                int(block.prime_limit)?,
                block.q_list_sha256,
                int(block.q_list_len)?,
                root,
                block_n,
                block_q,
                now,
            ],
        )?;
        blocks_completed += 1;
    }
    let completed = blocks_completed == state.total_blocks() && position > state.range_end;

    transaction.execute(
        "UPDATE campaign SET current_position = ?1, chunks_completed = chunks_completed + 1,
                blocks_completed = ?2,
                n_processed = n_processed + ?3, total_q_tests = total_q_tests + ?4,
                zero_solution_count = zero_solution_count + ?5,
                elapsed_seconds = elapsed_seconds + ?6, last_runtime_update = ?7, updated_at = ?7
         WHERE id = 1",
        params![
            int(position)?,
            int(blocks_completed)?,
            int(chunk.n_processed)?,
            int(chunk.q_tests)?,
            int(chunk.zero_solution_n.len() as u64)?,
            int(seconds)?,
            now,
        ],
    )?;
    if completed {
        let root = compute_campaign_root(&transaction, &state)?;
        transaction.execute(
            "UPDATE campaign SET status = 'COMPLETED', result_root = ?1, run_token = NULL,
                    run_started_at = NULL, pause_requested = 0 WHERE id = 1",
            params![root],
        )?;
    }
    let pause_requested: bool = transaction.query_row(
        "SELECT pause_requested != 0 FROM campaign WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    transaction.commit()?;
    Ok(CommitOutcome {
        current_position: position,
        block_completed,
        completed,
        pause_requested,
    })
}

/// Root of one block: the same construction the benchmark applies to a run covering
/// exactly `[block_start, block_end]`, chained over the block's chunks in `chunk_start`
/// order. Also proves the chunks tile the block.
pub fn compute_block_root(
    connection: &Connection,
    verification_mode: &str,
    chunk_size: u64,
    block: &BlockContext,
) -> Result<String, CampaignError> {
    let q_hash = hashing::parse_hex_digest(&block.q_list_sha256)
        .ok_or_else(|| CampaignError::Corrupt("invalid stored Q-list hash".to_owned()))?;
    let mut root = hashing::initial_result_root(
        block.block_start,
        block.block_end,
        verification_mode,
        chunk_size,
        block.prime_limit,
        q_hash,
    );
    let mut statement = connection.prepare(
        "SELECT chunk_start, chunk_end, result_sha256 FROM chunks
         WHERE chunk_start BETWEEN ?1 AND ?2 ORDER BY chunk_start",
    )?;
    let mut rows = statement.query(params![int(block.block_start)?, int(block.block_end)?])?;
    let mut expected = block.block_start;
    while let Some(row) = rows.next()? {
        let start = uint(row.get(0)?)?;
        let end = uint(row.get(1)?)?;
        if start != expected {
            return Err(CampaignError::Corrupt(format!(
                "chunks are not contiguous: expected start {expected}, found {start}"
            )));
        }
        let hash = hashing::parse_hex_digest(&row.get::<_, String>(2)?)
            .ok_or_else(|| CampaignError::Corrupt(format!("invalid result hash at {start}")))?;
        root = hashing::extend_result_root(root, hash);
        expected = end
            .checked_add(1)
            .ok_or(CampaignError::ArithmeticOverflow)?;
    }
    if expected != block.block_end + 1 {
        return Err(CampaignError::Corrupt(format!(
            "chunks end at {} but the block ends at {}",
            expected.saturating_sub(1),
            block.block_end
        )));
    }
    Ok(hashing::hex_digest(root))
}

/// Seed of the campaign root; thread count is intentionally excluded.
pub fn campaign_root_seed(
    range_start: u64,
    range_end: u64,
    verification_mode: &str,
    chunk_size: u64,
    block_size: u64,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"A220091-CAMPAIGN-ROOT-v1\0");
    hasher.update((SCHEMA_VERSION as u32).to_le_bytes());
    hasher.update(range_start.to_le_bytes());
    hasher.update(range_end.to_le_bytes());
    hasher.update((verification_mode.len() as u64).to_le_bytes());
    hasher.update(verification_mode.as_bytes());
    hasher.update(chunk_size.to_le_bytes());
    hasher.update(block_size.to_le_bytes());
    hasher.finalize().into()
}

/// Campaign root: the block roots folded in ascending block order with the same
/// `H(i+1) = SHA256(H(i) || block_root_i)` chaining as chunk hashes. Independent of
/// thread count and of the order in which chunks finished. Also proves the blocks
/// tile the campaign range.
pub fn compute_campaign_root(
    connection: &Connection,
    state: &CampaignState,
) -> Result<String, CampaignError> {
    let mut root = campaign_root_seed(
        state.range_start,
        state.range_end,
        &state.verification_mode,
        state.chunk_size,
        state.block_size,
    );
    let mut statement = connection
        .prepare("SELECT block_start, block_end, block_root FROM blocks ORDER BY block_start")?;
    let mut rows = statement.query([])?;
    let mut expected = state.range_start;
    while let Some(row) = rows.next()? {
        let start = uint(row.get(0)?)?;
        let end = uint(row.get(1)?)?;
        if start != expected {
            return Err(CampaignError::Corrupt(format!(
                "blocks are not contiguous: expected start {expected}, found {start}"
            )));
        }
        let hash = hashing::parse_hex_digest(&row.get::<_, String>(2)?)
            .ok_or_else(|| CampaignError::Corrupt(format!("invalid block root at {start}")))?;
        root = hashing::extend_result_root(root, hash);
        expected = end
            .checked_add(1)
            .ok_or(CampaignError::ArithmeticOverflow)?;
    }
    if expected != state.range_end + 1 {
        return Err(CampaignError::Corrupt(format!(
            "blocks end at {} but the campaign ends at {}",
            expected.saturating_sub(1),
            state.range_end
        )));
    }
    Ok(hashing::hex_digest(root))
}

pub fn compute_chunk(
    start: u64,
    end: u64,
    primes: &mut PrimeOracle,
    q_list: &QList,
) -> Result<ChunkRecord, CampaignError> {
    let mut hasher = Sha256::new();
    hasher.update(b"A220091-CHUNK-RESULT-v1\0");
    let mut n_processed = 0_u64;
    let mut q_tests = 0_u64;
    let mut zero_solution_n = Vec::new();
    let mut n = start;
    loop {
        let result = verifier::find_representation(n, primes, q_list)
            .map_err(CampaignError::Verification)?;
        let record = hashing::ResultRecord {
            n,
            status: u8::from(result.representation.is_some()),
            p: result.representation.map_or(0, |value| value.p),
            q: result.representation.map_or(0, |value| value.q),
            q_candidates_tested: result.q_candidates_tested,
        };
        hashing::update_result_hash(&mut hasher, record);
        n_processed += 1;
        q_tests = q_tests
            .checked_add(result.q_candidates_tested)
            .ok_or(CampaignError::ArithmeticOverflow)?;
        if result.representation.is_none() {
            zero_solution_n.push(n);
        }
        if n == end {
            break;
        }
        n += 1;
    }
    Ok(ChunkRecord {
        chunk_start: start,
        chunk_end: end,
        result_sha256: hashing::hex_digest(hasher.finalize().into()),
        n_processed,
        q_tests,
        zero_solution_n,
        completed_at: unix_now().max(0) as u64,
    })
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub threads: usize,
    pub heartbeat_interval: Duration,
    pub recover: bool,
    /// Stop gracefully (PAUSED) after committing this many chunks in this run.
    pub max_chunks: Option<u64>,
    /// Prime-oracle window size and number of resident windows, per worker.
    pub segment_size: u64,
    pub cache_segments: usize,
}

impl RunOptions {
    pub fn new(threads: usize) -> Self {
        Self {
            threads,
            heartbeat_interval: DEFAULT_HEARTBEAT,
            recover: false,
            max_chunks: None,
            segment_size: DEFAULT_SEGMENT_SIZE,
            cache_segments: DEFAULT_CACHE_SEGMENTS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub recovered_crash: bool,
    pub chunks_committed: u64,
    pub in_domain_counterexamples: Vec<u64>,
    pub state: CampaignState,
}

/// Build the oracle and Q-list for one block exactly as the benchmark does for a run
/// ending at `block_end`.
fn build_block_structures(
    block_end: u64,
    options: &RunOptions,
) -> Result<(PrimeOracle, QList), CampaignError> {
    let mut primes = PrimeOracle::new(
        required_prime_limit(block_end)?,
        options.segment_size,
        options.cache_segments,
    )
    .map_err(|error| {
        CampaignError::Invalid(format!("prime oracle construction failed: {error}"))
    })?;
    let q_list = crate::build_q_list(maximum_q_for_range(block_end), &mut primes)
        .map_err(|error| CampaignError::Invalid(format!("Q-list construction failed: {error}")))?;
    Ok((primes, q_list))
}

/// Run (or resume) the campaign stored in `path`, one bounded block at a time, until it
/// completes, a pause is requested, `max_chunks` is reached, an in-domain counterexample
/// is committed, or an error occurs. The database must already exist (see
/// `open_or_create`). Each block's oracle and Q-list are dropped before the next block.
pub fn run_campaign(
    path: &Path,
    options: &RunOptions,
    sink: Option<&dyn ChunkProgressSink>,
) -> Result<RunOutcome, CampaignError> {
    if options.threads == 0 || options.segment_size == 0 || options.cache_segments == 0 {
        return Err(CampaignError::Invalid(
            "thread, segment and cache values must be greater than zero".to_owned(),
        ));
    }
    let mut connection = open_connection(path)?;
    let config = load_state(&connection)?;
    let completed_blocks: Vec<u64> = {
        let mut statement =
            connection.prepare("SELECT block_start FROM blocks ORDER BY block_start")?;
        let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
        let mut starts = Vec::new();
        for row in rows {
            starts.push(uint(row?)?);
        }
        starts
    };

    let run = begin_run(&mut connection, unix_now(), options.recover)?;
    let clock = Mutex::new(RuntimeClock::start());

    let stop = AtomicBool::new(false);
    let finished = AtomicBool::new(false);
    let heartbeat_error: Mutex<Option<CampaignError>> = Mutex::new(None);

    let mut run_error: Option<CampaignError> = None;
    let mut chunks_committed = 0_u64;
    let mut counterexamples = Vec::new();

    thread::scope(|outer| {
        outer.spawn(|| {
            let connection = match open_connection(path) {
                Ok(connection) => connection,
                Err(error) => {
                    *heartbeat_error.lock().unwrap() = Some(error);
                    stop.store(true, Ordering::SeqCst);
                    return;
                }
            };
            let mut last_beat = Instant::now();
            while !finished.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(50));
                if last_beat.elapsed() < options.heartbeat_interval {
                    continue;
                }
                last_beat = Instant::now();
                let mut clock = clock.lock().unwrap();
                let seconds = clock.pending_seconds();
                match heartbeat_at(&connection, &run.token, seconds, unix_now()) {
                    Ok(()) => clock.consume(seconds),
                    Err(error) => {
                        *heartbeat_error.lock().unwrap() = Some(error);
                        stop.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            }
        });

        for index in 0..config.total_blocks() {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let (block_start, block_end) = block_bounds(
                config.range_start,
                config.range_end,
                config.block_size,
                index,
            );
            if completed_blocks.binary_search(&block_start).is_ok() {
                continue;
            }

            let completed: Vec<u64> = match (|| -> Result<Vec<u64>, CampaignError> {
                let mut statement = connection.prepare(
                    "SELECT chunk_start FROM chunks WHERE chunk_start BETWEEN ?1 AND ?2
                     ORDER BY chunk_start",
                )?;
                let rows = statement
                    .query_map(params![int(block_start)?, int(block_end)?], |row| {
                        row.get::<_, i64>(0)
                    })?;
                let mut starts = Vec::new();
                for row in rows {
                    starts.push(uint(row?)?);
                }
                Ok(starts)
            })() {
                Ok(starts) => starts,
                Err(error) => {
                    run_error = Some(error);
                    break;
                }
            };
            let chunks_in_block = (block_end - block_start) / config.chunk_size + 1;
            if completed.len() as u64 == chunks_in_block {
                run_error = Some(CampaignError::Corrupt(format!(
                    "block {block_start}..{block_end} has every chunk but no block record"
                )));
                break;
            }

            let (primes, q_list) = match build_block_structures(block_end, options) {
                Ok(structures) => structures,
                Err(error) => {
                    run_error = Some(error);
                    break;
                }
            };
            let block = BlockContext::new(block_start, block_end, &primes, &q_list);

            let next_index = AtomicU64::new(0);
            let (sender, receiver) = mpsc::sync_channel::<Result<ChunkRecord, CampaignError>>(
                options.threads.saturating_mul(2).max(2),
            );
            thread::scope(|inner| {
                for _ in 0..options.threads {
                    let sender = sender.clone();
                    let completed = &completed;
                    let next_index = &next_index;
                    let stop = &stop;
                    let primes = &primes;
                    let q_list = &q_list;
                    let chunk_size = config.chunk_size;
                    inner.spawn(move || {
                        let mut local_primes = primes.clone();
                        while !stop.load(Ordering::SeqCst) {
                            let chunk_index = next_index.fetch_add(1, Ordering::SeqCst);
                            if chunk_index >= chunks_in_block {
                                break;
                            }
                            let start = block_start + chunk_index * chunk_size;
                            if completed.binary_search(&start).is_ok() {
                                continue;
                            }
                            let end = start.saturating_add(chunk_size - 1).min(block_end);
                            let message = compute_chunk(start, end, &mut local_primes, q_list);
                            let failed = message.is_err();
                            if sender.send(message).is_err() || failed {
                                stop.store(true, Ordering::SeqCst);
                                break;
                            }
                        }
                    });
                }
                drop(sender);

                for message in receiver.iter() {
                    let chunk = match message {
                        Ok(chunk) => chunk,
                        Err(error) => {
                            run_error.get_or_insert(error);
                            stop.store(true, Ordering::SeqCst);
                            continue;
                        }
                    };
                    if run_error.is_some() {
                        continue;
                    }
                    let outcome = {
                        let mut clock = clock.lock().unwrap();
                        let seconds = clock.pending_seconds();
                        let outcome = commit_chunk_at(
                            &mut connection,
                            &run.token,
                            &chunk,
                            &block,
                            seconds,
                            unix_now(),
                        );
                        if outcome.is_ok() {
                            clock.consume(seconds);
                        }
                        outcome
                    };
                    match outcome {
                        Ok(outcome) => {
                            chunks_committed += 1;
                            if let Some(sink) = sink {
                                sink.chunk_committed(&chunk);
                            }
                            let in_domain = chunk
                                .zero_solution_n
                                .iter()
                                .copied()
                                .filter(|&n| is_in_conjecture_domain(n));
                            let before = counterexamples.len();
                            counterexamples.extend(in_domain);
                            if outcome.pause_requested
                                || counterexamples.len() > before
                                || options
                                    .max_chunks
                                    .is_some_and(|limit| chunks_committed >= limit)
                            {
                                stop.store(true, Ordering::SeqCst);
                            }
                        }
                        Err(error) => {
                            run_error = Some(error);
                            stop.store(true, Ordering::SeqCst);
                        }
                    }
                }
            });
            // Release this block's oracle and Q-list before the next block is built.
            drop(q_list);
            drop(primes);
        }
        finished.store(true, Ordering::SeqCst);
    });

    // Flush runtime and release ownership unless the campaign just completed.
    {
        let mut clock = clock.lock().unwrap();
        let seconds = clock.pending_seconds();
        if pause_at(&connection, &run.token, seconds, unix_now()).unwrap_or(false) {
            clock.consume(seconds);
        }
    }
    if let Some(error) = run_error {
        return Err(error);
    }
    if let Some(error) = heartbeat_error.lock().unwrap().take() {
        return Err(error);
    }
    Ok(RunOutcome {
        recovered_crash: run.recovered_crash,
        chunks_committed,
        in_domain_counterexamples: counterexamples,
        state: load_state(&connection)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn config(start: u64, end: u64, chunk_size: u64, block_size: u64) -> CampaignConfig {
        CampaignConfig::new(start, end, chunk_size, block_size, "full-scan", None)
    }

    fn database() -> (TempDir, std::path::PathBuf) {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("campaign").join("campaign.db");
        (directory, path)
    }

    fn options(threads: usize, max_chunks: Option<u64>) -> RunOptions {
        let mut options = RunOptions::new(threads);
        options.max_chunks = max_chunks;
        options.segment_size = 64;
        options.cache_segments = 4;
        options
    }

    fn run(path: &Path, threads: usize, max_chunks: Option<u64>) -> RunOutcome {
        run_campaign(path, &options(threads, max_chunks), None).unwrap()
    }

    /// Oracle, Q-list and context of the block containing `n`, built per block.
    fn block_of(config: &CampaignConfig, n: u64) -> (PrimeOracle, QList, BlockContext) {
        let index = (n - config.range_start) / config.block_size;
        let (start, end) = block_bounds(
            config.range_start,
            config.range_end,
            config.block_size,
            index,
        );
        let (primes, q_list) = build_block_structures(end, &options(1, None)).unwrap();
        let context = BlockContext::new(start, end, &primes, &q_list);
        (primes, q_list, context)
    }

    fn chunk(config: &CampaignConfig, start: u64) -> (ChunkRecord, BlockContext) {
        let end = (start + config.chunk_size - 1).min(config.range_end);
        let (mut primes, q_list, context) = block_of(config, start);
        (
            compute_chunk(start, end, &mut primes, &q_list).unwrap(),
            context,
        )
    }

    fn commit(
        connection: &mut Connection,
        token: &str,
        config: &CampaignConfig,
        start: u64,
        seconds: u64,
        now: i64,
    ) -> Result<CommitOutcome, CampaignError> {
        let (record, context) = chunk(config, start);
        commit_chunk_at(connection, token, &record, &context, seconds, now)
    }

    fn chunk_hashes(path: &Path) -> Vec<(i64, String)> {
        let connection = open_connection(path).unwrap();
        let mut statement = connection
            .prepare("SELECT chunk_start, result_sha256 FROM chunks ORDER BY chunk_start")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn block_rows(path: &Path) -> Vec<(i64, i64, i64, String, i64, String)> {
        let connection = open_connection(path).unwrap();
        let mut statement = connection
            .prepare(
                "SELECT block_start, block_end, prime_limit, q_list_sha256, q_list_len, block_root
                 FROM blocks ORDER BY block_start",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn creates_schema_in_paused_state_with_zero_counters() {
        let (_directory, path) = database();
        open_or_create(&path, &config(1, 100, 10, 50), 1000).unwrap();
        let connection = open_connection(&path).unwrap();
        let state = load_state(&connection).unwrap();
        assert_eq!(state.status, CampaignStatus::Paused);
        assert_eq!(state.current_position, 1);
        assert_eq!(state.block_size, 50);
        assert_eq!(state.total_blocks(), 2);
        assert_eq!(state.blocks_completed, 0);
        assert_eq!(state.elapsed_seconds, 0);
        assert_eq!(state.total_q_tests, 0);
        assert_eq!(state.chunks_completed, 0);
        assert_eq!((state.created_at, state.run_started_at), (1000, None));
        let mode: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let sync: i64 = connection
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sync, 2); // FULL
        let tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
                 AND name IN ('campaign', 'chunks', 'blocks', 'zero_solutions')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 4);
    }

    #[test]
    fn invalid_layouts_are_rejected() {
        let (_directory, path) = database();
        assert!(open_or_create(&path, &config(1, 100, 10, 25), 1).is_err());
        assert!(open_or_create(&path, &config(1, 100, 10, 0), 1).is_err());
        assert!(open_or_create(&path, &config(1, 100, 0, 10), 1).is_err());
        assert!(!path.exists() || load_state(&open_connection(&path).unwrap()).is_err());
    }

    #[test]
    fn full_run_uses_per_block_structures_and_matches_independent_roots() {
        let (_directory, path) = database();
        let config = config(1, 95, 10, 40);
        open_or_create(&path, &config, 1).unwrap();
        let state = run(&path, 3, None).state;
        assert_eq!(state.status, CampaignStatus::Completed);
        assert_eq!(state.current_position, 96);
        assert_eq!(state.chunks_completed, 10);
        assert_eq!(state.blocks_completed, 3);
        assert_eq!(state.n_processed, 95);
        assert_eq!(state.run_started_at, None);

        // Independent reference: every block gets its OWN oracle and Q-list built for the
        // block end only, then chunk hashes are folded with the production root functions.
        let mut campaign_root = campaign_root_seed(1, 95, "full-scan", 10, 40);
        let mut total_q = 0;
        let mut zeros = 0;
        let mut expected_blocks = Vec::new();
        for (block_start, block_end) in [(1, 40), (41, 80), (81, 95)] {
            let (mut primes, q_list, context) = block_of(&config, block_start);
            assert_eq!(context.prime_limit, block_end * 3 + 1);
            assert_eq!(q_list.maximum_q(), (block_end - 1) / 2);
            let mut root = hashing::initial_result_root(
                block_start,
                block_end,
                "full-scan",
                10,
                context.prime_limit,
                hashing::parse_hex_digest(&context.q_list_sha256).unwrap(),
            );
            let mut start = block_start;
            while start <= block_end {
                let end = (start + 9).min(block_end);
                let record = compute_chunk(start, end, &mut primes, &q_list).unwrap();
                root = hashing::extend_result_root(
                    root,
                    hashing::parse_hex_digest(&record.result_sha256).unwrap(),
                );
                total_q += record.q_tests;
                zeros += record.zero_solution_n.len() as u64;
                start += 10;
            }
            campaign_root = hashing::extend_result_root(campaign_root, root);
            expected_blocks.push((
                block_start as i64,
                block_end as i64,
                context.prime_limit as i64,
                context.q_list_sha256.clone(),
                context.q_list_len as i64,
                hashing::hex_digest(root),
            ));
        }
        assert_eq!(block_rows(&path), expected_blocks);
        // Later blocks use larger (but still block-scoped) prime limits and Q-lists.
        assert!(expected_blocks[0].2 < expected_blocks[2].2);
        assert!(expected_blocks[0].4 <= expected_blocks[2].4);
        assert_eq!(state.result_root, Some(hashing::hex_digest(campaign_root)));
        assert_eq!(state.total_q_tests, total_q);
        assert_eq!(state.zero_solution_count, zeros);
    }

    #[test]
    fn thread_count_does_not_change_chunks_blocks_or_root() {
        let config = config(1, 120, 7, 35);
        let (_d1, one) = database();
        let (_d2, four) = database();
        open_or_create(&one, &config, 1).unwrap();
        open_or_create(&four, &config, 1).unwrap();
        let first = run(&one, 1, None).state;
        let second = run(&four, 4, None).state;
        assert_eq!(first.result_root, second.result_root);
        assert_eq!(chunk_hashes(&one), chunk_hashes(&four));
        assert_eq!(block_rows(&one), block_rows(&four));
        assert_eq!(first.total_q_tests, second.total_q_tests);
    }

    #[test]
    fn resume_in_the_middle_of_a_block_matches_uninterrupted_run() {
        let config = config(1, 200, 10, 100);
        let (_d1, whole) = database();
        let (_d2, split) = database();
        open_or_create(&whole, &config, 1).unwrap();
        open_or_create(&split, &config, 1).unwrap();
        let reference = run(&whole, 2, None).state;

        // Stop early inside the first 10-chunk block (a few in-flight chunks may still commit).
        let partial = run(&split, 1, Some(2));
        assert_eq!(partial.state.status, CampaignStatus::Paused);
        assert_eq!(partial.state.blocks_completed, 0);
        assert!(block_rows(&split).is_empty());
        assert!(partial.state.chunks_completed >= 2 && partial.state.chunks_completed < 10);
        let finished = run(&split, 2, None);
        assert_eq!(finished.state.status, CampaignStatus::Completed);
        assert_eq!(finished.state.result_root, reference.result_root);
        assert_eq!(finished.state.run_count, 2);
        assert_eq!(chunk_hashes(&split), chunk_hashes(&whole));
        assert_eq!(block_rows(&split), block_rows(&whole));
        assert_eq!(finished.state.total_q_tests, reference.total_q_tests);
        assert_eq!(finished.state.chunks_completed, 20);
        assert_eq!(finished.state.blocks_completed, 2);
    }
    #[test]
    fn resume_starts_at_the_first_incomplete_block() {
        let config = config(1, 90, 10, 30);
        let (_directory, path) = database();
        open_or_create(&path, &config, 1).unwrap();
        // Stop right after block 1 (3 chunks), single thread for determinism.
        let first = run(&path, 1, Some(3));
        assert_eq!(first.state.blocks_completed, 1);
        assert_eq!(first.state.current_position, 31);
        let rows = block_rows(&path);
        assert_eq!((rows[0].0, rows[0].1), (1, 30));
        let second = run(&path, 1, None);
        assert_eq!(second.state.status, CampaignStatus::Completed);
        assert_eq!(second.chunks_committed, 6);
        // The first block was not recomputed.
        assert_eq!(block_rows(&path)[0], rows[0]);
    }

    #[test]
    fn zero_solutions_are_recorded_with_domain_flag_and_counts_match() {
        let (_directory, path) = database();
        open_or_create(&path, &config(1, 60, 20, 40), 1).unwrap();
        let outcome = run(&path, 2, None);
        assert!(outcome.in_domain_counterexamples.is_empty());
        let connection = open_connection(&path).unwrap();
        let rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM zero_solutions", [], |row| row.get(0))
            .unwrap();
        assert!(rows > 0);
        assert_eq!(rows as u64, outcome.state.zero_solution_count);
        let in_domain: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM zero_solutions WHERE in_conjecture_domain = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(in_domain, 0);
    }

    #[test]
    fn crash_recovery_does_not_count_time_after_last_persisted_update() {
        let (_directory, path) = database();
        open_or_create(&path, &config(1, 40, 10, 40), 100).unwrap();
        let mut connection = open_connection(&path).unwrap();
        let first = begin_run(&mut connection, 1_000, false).unwrap();
        assert!(!first.recovered_crash);
        heartbeat_at(&connection, &first.token, 60, 1_060).unwrap();
        // The process dies here (no pause). Hours later the next process starts.
        let mut second_connection = open_connection(&path).unwrap();
        let second = begin_run(&mut second_connection, 50_000, false).unwrap();
        assert!(second.recovered_crash);
        let state = load_state(&second_connection).unwrap();
        assert_eq!(state.elapsed_seconds, 60);
        assert_eq!(state.crash_recoveries, 1);
        assert_eq!(state.status, CampaignStatus::Running);
        assert_eq!(state.run_started_at, Some(50_000));
        // The dead run's token has lost ownership.
        assert_eq!(
            heartbeat_at(&connection, &first.token, 5, 50_001),
            Err(CampaignError::LostOwnership)
        );
        assert_eq!(load_state(&connection).unwrap().elapsed_seconds, 60);
    }

    #[test]
    fn crash_in_the_middle_of_a_block_recomputes_only_uncommitted_work() {
        let config = config(1, 80, 10, 40);
        let (_d1, whole) = database();
        let (_d2, crashed) = database();
        open_or_create(&whole, &config, 1).unwrap();
        open_or_create(&crashed, &config, 1).unwrap();
        let reference = run(&whole, 2, None).state;

        open_or_create(&crashed, &config, 1).unwrap();
        let mut connection = open_connection(&crashed).unwrap();
        let dead = begin_run(&mut connection, 1_000, false).unwrap();
        commit(&mut connection, &dead.token, &config, 1, 4, 1_004).unwrap();
        commit(&mut connection, &dead.token, &config, 11, 4, 1_008).unwrap();
        // The process dies mid-block: no pause, no block row.
        drop(connection);
        assert!(block_rows(&crashed).is_empty());

        let mut recovering = options(2, None);
        recovering.recover = true;
        let outcome = run_campaign(&crashed, &recovering, None).unwrap();
        assert!(outcome.recovered_crash);
        assert_eq!(outcome.chunks_committed, 6);
        assert_eq!(outcome.state.status, CampaignStatus::Completed);
        assert_eq!(outcome.state.crash_recoveries, 1);
        assert_eq!(outcome.state.result_root, reference.result_root);
        assert_eq!(chunk_hashes(&crashed), chunk_hashes(&whole));
        assert_eq!(block_rows(&crashed), block_rows(&whole));
        assert!(outcome.state.elapsed_seconds >= 8);
    }

    #[test]
    fn fresh_running_state_is_refused_unless_recovery_is_explicit() {
        let (_directory, path) = database();
        open_or_create(&path, &config(1, 40, 10, 40), 100).unwrap();
        let mut connection = open_connection(&path).unwrap();
        begin_run(&mut connection, 1_000, false).unwrap();
        assert!(matches!(
            begin_run(&mut connection, 1_030, false),
            Err(CampaignError::PossiblyActive {
                seconds_since_update: 30
            })
        ));
        let forced = begin_run(&mut connection, 1_030, true).unwrap();
        assert!(forced.recovered_crash);
    }

    #[test]
    fn timer_accumulates_only_while_running() {
        let (_directory, path) = database();
        let config = config(1, 40, 10, 40);
        open_or_create(&path, &config, 100).unwrap();
        let mut connection = open_connection(&path).unwrap();
        let run_one = begin_run(&mut connection, 1_000, false).unwrap();
        heartbeat_at(&connection, &run_one.token, 60, 1_060).unwrap();
        commit(&mut connection, &run_one.token, &config, 1, 7, 1_067).unwrap();
        assert!(pause_at(&connection, &run_one.token, 3, 1_070).unwrap());
        let paused = load_state(&connection).unwrap();
        assert_eq!(paused.status, CampaignStatus::Paused);
        assert_eq!(paused.elapsed_seconds, 70);
        assert_eq!(paused.run_started_at, None);
        // A week passes while paused: nothing accumulates.
        let run_two = begin_run(&mut connection, 700_000, false).unwrap();
        assert_eq!(load_state(&connection).unwrap().elapsed_seconds, 70);
        heartbeat_at(&connection, &run_two.token, 30, 700_030).unwrap();
        assert_eq!(load_state(&connection).unwrap().elapsed_seconds, 100);
        // Pausing twice does not double count.
        assert!(pause_at(&connection, &run_two.token, 0, 700_030).unwrap());
        assert!(!pause_at(&connection, &run_two.token, 50, 700_100).unwrap());
        assert_eq!(load_state(&connection).unwrap().elapsed_seconds, 100);
    }

    #[test]
    fn completed_campaign_cannot_be_restarted_and_stops_the_timer() {
        let (_directory, path) = database();
        open_or_create(&path, &config(1, 30, 10, 20), 1).unwrap();
        let finished = run(&path, 2, None).state;
        assert_eq!(finished.status, CampaignStatus::Completed);
        let mut connection = open_connection(&path).unwrap();
        assert_eq!(
            begin_run(&mut connection, 9_999_999, true),
            Err(CampaignError::AlreadyCompleted)
        );
        assert_eq!(load_state(&connection).unwrap(), finished);
    }

    #[test]
    fn rejected_commits_leave_no_partial_state() {
        let (_directory, path) = database();
        let config = config(1, 40, 10, 20);
        open_or_create(&path, &config, 1).unwrap();
        let mut connection = open_connection(&path).unwrap();
        let run = begin_run(&mut connection, 1_000, false).unwrap();
        let before = load_state(&connection).unwrap();

        let (good, context) = chunk(&config, 1);
        let mut misaligned = good.clone();
        misaligned.chunk_start = 2;
        misaligned.chunk_end = 11;
        assert!(
            commit_chunk_at(&mut connection, &run.token, &misaligned, &context, 5, 1_005).is_err()
        );
        assert_eq!(
            commit_chunk_at(&mut connection, "not-the-token", &good, &context, 5, 1_005),
            Err(CampaignError::LostOwnership)
        );
        let mut bad_hash = good.clone();
        bad_hash.result_sha256 = "zz".to_owned();
        assert!(
            commit_chunk_at(&mut connection, &run.token, &bad_hash, &context, 5, 1_005).is_err()
        );
        // A chunk of block 2 presented with block 1's context, and a mis-sized block.
        let (second_block_chunk, _) = chunk(&config, 21);
        assert!(commit_chunk_at(
            &mut connection,
            &run.token,
            &second_block_chunk,
            &context,
            5,
            1_005
        )
        .is_err());
        let mut wrong_block = context.clone();
        wrong_block.block_end = 30;
        assert!(
            commit_chunk_at(&mut connection, &run.token, &good, &wrong_block, 5, 1_005).is_err()
        );
        assert_eq!(load_state(&connection).unwrap(), before);

        commit_chunk_at(&mut connection, &run.token, &good, &context, 5, 1_005).unwrap();
        let committed = load_state(&connection).unwrap();
        // Duplicate chunk: the INSERT fails and the whole transaction rolls back.
        assert!(commit_chunk_at(&mut connection, &run.token, &good, &context, 9, 1_010).is_err());
        assert_eq!(load_state(&connection).unwrap(), committed);
        assert_eq!(before.chunks_completed, 0);
        assert_eq!(committed.chunks_completed, 1);
        assert_eq!(committed.elapsed_seconds, 5);
        assert_eq!(chunk_hashes(&path).len(), 1);
    }

    #[test]
    fn block_row_is_committed_atomically_with_its_last_chunk() {
        let (_directory, path) = database();
        let config = config(1, 40, 10, 20);
        open_or_create(&path, &config, 1).unwrap();
        let mut connection = open_connection(&path).unwrap();
        let run = begin_run(&mut connection, 1_000, false).unwrap();

        let first = commit(&mut connection, &run.token, &config, 1, 0, 1_001).unwrap();
        assert!(!first.block_completed);
        assert!(block_rows(&path).is_empty());
        assert_eq!(load_state(&connection).unwrap().blocks_completed, 0);

        let second = commit(&mut connection, &run.token, &config, 11, 0, 1_002).unwrap();
        assert!(second.block_completed && !second.completed);
        let state = load_state(&connection).unwrap();
        assert_eq!(state.blocks_completed, 1);
        assert_eq!(state.current_position, 21);
        assert_eq!(block_rows(&path).len(), 1);
        assert_eq!(state.status, CampaignStatus::Running);
        assert_eq!(state.result_root, None);
    }

    #[test]
    fn out_of_order_commits_advance_frontier_only_over_contiguous_chunks() {
        let (_directory, path) = database();
        let config = config(1, 40, 10, 20);
        open_or_create(&path, &config, 1).unwrap();
        let mut connection = open_connection(&path).unwrap();
        let run = begin_run(&mut connection, 1_000, false).unwrap();
        let position = |connection: &Connection| load_state(connection).unwrap().current_position;

        commit(&mut connection, &run.token, &config, 21, 0, 1_001).unwrap();
        assert_eq!(position(&connection), 1);
        commit(&mut connection, &run.token, &config, 1, 0, 1_002).unwrap();
        assert_eq!(position(&connection), 11);
        commit(&mut connection, &run.token, &config, 11, 0, 1_003).unwrap();
        assert_eq!(position(&connection), 31);
        let last = commit(&mut connection, &run.token, &config, 31, 0, 1_004).unwrap();
        assert!(last.completed && last.block_completed);
        let state = load_state(&connection).unwrap();
        assert_eq!(state.status, CampaignStatus::Completed);
        assert!(state.result_root.is_some());
        assert_eq!(state.blocks_completed, 2);
    }

    #[test]
    fn reopening_with_a_different_configuration_fails_and_changes_nothing() {
        let (_directory, path) = database();
        let original = config(1, 40, 10, 20);
        open_or_create(&path, &original, 1).unwrap();
        let error = open_or_create(&path, &config(1, 40, 5, 20), 2).unwrap_err();
        assert!(matches!(
            error,
            CampaignError::ConfigMismatch {
                field: "chunk_size",
                ..
            }
        ));
        let error = open_or_create(&path, &config(1, 40, 10, 40), 2).unwrap_err();
        assert!(matches!(
            error,
            CampaignError::ConfigMismatch {
                field: "block_size",
                ..
            }
        ));
        assert!(open_or_create(&path, &config(1, 50, 10, 20), 2).is_err());
        open_or_create(&path, &original, 3).unwrap();
        let connection = open_connection(&path).unwrap();
        assert_eq!(load_state(&connection).unwrap().created_at, 1);
    }

    #[test]
    fn pause_request_stops_a_run_gracefully() {
        let (_directory, path) = database();
        open_or_create(&path, &config(1, 400, 10, 100), 1).unwrap();
        struct PauseAfterFirst<'a>(&'a Path);
        impl ChunkProgressSink for PauseAfterFirst<'_> {
            fn chunk_committed(&self, _chunk: &ChunkRecord) {
                request_pause(self.0).unwrap();
            }
        }
        let sink = PauseAfterFirst(&path);
        let outcome = run_campaign(&path, &options(1, None), Some(&sink)).unwrap();
        assert_eq!(outcome.state.status, CampaignStatus::Paused);
        assert!(outcome.state.chunks_completed < 40);
        assert!(!outcome.state.pause_requested);
    }

    #[test]
    fn heartbeat_thread_keeps_ownership_and_updates_last_runtime_update() {
        let (_directory, path) = database();
        open_or_create(&path, &config(1, 40, 10, 20), 1).unwrap();
        let mut options = options(2, None);
        options.heartbeat_interval = Duration::from_millis(60);
        let outcome = run_campaign(&path, &options, None).unwrap();
        assert_eq!(outcome.state.status, CampaignStatus::Completed);
        assert!(outcome.state.last_runtime_update.is_some());
    }
}
