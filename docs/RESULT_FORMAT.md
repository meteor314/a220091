# A220091 Result and Campaign State Format

## Scope

This document specifies the Phase 7 integrity format. SHA-256 values are
integrity identifiers, not mathematical proofs and not evidence by themselves
that a computation was executed correctly.

All integer fields in canonical result bytes are unsigned little-endian.

## Q-list hash

`q` is represented internally as `u64` because the mathematical q bound
(`N / 2` for even `N`) can exceed `u32::MAX` once `N` grows beyond roughly
`8.6e9`. The wire format for the Q-list hash selects its element width from
the Q-list's `maximum_q` bound so that ranges which fit in `u32` keep the
original byte-identical hash:

```text
if maximum_q <= u32::MAX:
    ASCII bytes: A220091-Q-LIST-v1\0
    for each q in ascending order:
        q as u32 little-endian
else:
    ASCII bytes: A220091-Q-LIST-v2\0
    for each q in ascending order:
        q as u64 little-endian
```

The hexadecimal digest is stored as lowercase 64-character text.

## Per-n result record

Each record is exactly 33 bytes after the chunk domain prefix:

| Bytes | Field | Encoding |
|---:|---|---|
| 0..7 | `n` | `u64` little-endian |
| 8 | `status` | `0` = no representation, `1` = representation |
| 9..16 | `p` | `u64` little-endian; zero when status is `0` |
| 17..24 | `q` | `u64` little-endian; zero when status is `0` |
| 25..32 | `q_candidates_tested` | `u64` little-endian |

The chunk hash is SHA-256 of:

```text
ASCII bytes: A220091-CHUNK-RESULT-v1\0
records in ascending n order
```

No human-readable output is hashed.

## Global result root

The initial root is SHA-256 of:

```text
ASCII bytes: A220091-RESULT-ROOT-v1\0
schema version as u32 little-endian
range start as u64 little-endian
range end as u64 little-endian
mode byte length as u64 little-endian
mode UTF-8 bytes
chunk size as u64 little-endian
prime limit as u64 little-endian
Q-list hash as 32 raw bytes
```

Thread count is intentionally excluded, so changing Rayon worker count does
not change the mathematical result root. For each chunk in ascending numeric
range order:

```text
H(i+1) = SHA256(H(i) || chunk_hash_i)
```

Changing chunk size changes the ordered chunk sequence and therefore normally
changes the root. The mathematical results remain comparable independently.

## SQLite campaign state

Persistent campaign state lives in a single SQLite database (default
`/var/lib/a220091/campaign/campaign.db`), opened in WAL mode with
`synchronous=FULL` and a 10 s `busy_timeout`. No per-chunk files are written.
The chunk hash and result root above are unchanged; they are stored, not
re-derived from any file format.

```sql
-- schema version stored in PRAGMA user_version (currently 1)
campaign(id=1 singleton, status RUNNING|PAUSED|COMPLETED,
         range_start, range_end, chunk_size, block_size, verification_mode,
         source_version,                                  -- immutable config
         blocks_completed,
         current_position,            -- next n of the contiguous verified prefix
         chunks_completed, n_processed, total_q_tests, zero_solution_count,
         result_root,                 -- set in the committing transaction of the last chunk
         elapsed_seconds, run_started_at, last_runtime_update,
         run_token, run_count, crash_recoveries, pause_requested,
         created_at, updated_at)
chunks(chunk_start PRIMARY KEY, chunk_end, result_sha256,
       n_processed, q_tests, zero_solution_n /* JSON array */, completed_at)
blocks(block_start PRIMARY KEY, block_end,
       prime_limit /* block_end*3+1 */, q_list_sha256, q_list_len,
       block_root, n_processed, q_tests, completed_at)
zero_solutions(n PRIMARY KEY, chunk_start, in_conjecture_domain)
```

A chunk is committed in one `BEGIN IMMEDIATE` transaction that inserts the
chunk row, inserts its zero-solution rows, advances `current_position` and the
cumulative counters, and updates the runtime timer. A range is never reported
as complete (including to Discord) before that transaction has committed.

`result_root` is computed from the `chunks` table in ascending `chunk_start`
order inside the transaction that commits the final chunk, so it is identical
to an uninterrupted run and independent of thread count.