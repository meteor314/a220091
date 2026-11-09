# A220091 SQLite Campaign

SQLite is the sole persistent state of a campaign. The legacy JSON checkpoint
system (`src/checkpoint.rs`, `chunk-*.json`, `checkpoint_sha256`) and the
checkpoint migration utility have been retired. Existing legacy data on the
N100 (`/var/lib/a220091/checkpoints`, `/var/lib/a220091/campaign-30d`) is **not
read, imported, modified or deleted** by the new binary; a new campaign starts
from scratch in `/var/lib/a220091/campaign/campaign.db`.

## Commands (`a220091-campaign`)

```text
a220091-campaign run [--db PATH] --start N --end N [--chunk-size N] [--block-size N] [--threads N]
                     [--mode conjecture-domain|full-scan] [--recover] [--max-chunks N]
                     [--heartbeat-seconds N] [--no-discord] [--notification-state PATH]
a220091-campaign status [--db PATH]
a220091-campaign pause  [--db PATH]
```

`run` creates the database on first use and resumes it afterwards. Range, chunk
size, block size, mode and source version are immutable once the database exists;
a mismatch is refused. `--block-size` (default 100,000,000) must be a multiple of
`--chunk-size`.

## Bounded verification blocks

The campaign `[start, end]` is split into blocks of `block_size` n values (the last
one clipped at `end`), exactly like the legacy 100M-block campaign. For each block:

1. `prime_limit = block_end * 3 + 1` and the Q-list covers `q <= (block_end - 1) / 2`
   (the per-block `benchmark --start --end` construction); only this block's
   oracle and Q-list exist in memory.
2. The block is verified by the parallel verifier in chunks of `chunk_size`; each
   chunk commits in its own SQLite transaction.
3. The transaction of the block's **last** chunk also inserts the `blocks` row
   (`prime_limit`, `q_list_sha256`, `q_list_len`, `block_root`). A block with all
   chunks but no block row cannot exist; if found it is reported as corruption.
4. The oracle and Q-list are dropped before the next block is built.

Resume reads SQLite, skips completed blocks, skips committed chunks of a partly
done block (its Q-list is rebuilt deterministically) and continues.

Roots (independent of thread count): `block_root` equals the legacy per-block
benchmark root (`initial_result_root(block_start, block_end, mode, chunk_size,
prime_limit, q_list_sha256)` extended with each chunk hash in order). The campaign
`result_root` starts from a seed over (range, mode, chunk_size, block_size) and
folds the block roots in order.

## Memory per block

Resident memory is the current block only: the Q-list (8 bytes per q where q,
6q-1, 6q+1 are all prime, up to `(block_end-1)/2`), the shared base primes
(O(sqrt(3 \* block_end))) and per worker one segment cache (`cache_segments` x
`segment_size/8` bytes, default 8 x 128 KiB). The Q-list grows with the block's
absolute end, not its size. Measured with `benchmark` on the dev machine: block
end 1e8 -> 51,325 q, oracle 1.06 MB; block end 1e9 -> 352,904 q (2.8 MB), oracle
1.09 MB, peak working set about 10 MiB for the whole process. The Q-list build is
sequential: about 1.1 s at end 1e8 and 12 s at end 1e9 (dev machine).
Compose (values are required, nothing is defaulted for the range):

```sh
CAMPAIGN_START=1 CAMPAIGN_END=... docker compose up -d campaign
docker compose run --rm --entrypoint /usr/local/bin/a220091-campaign campaign status
```

To import an existing database from `data/db` into the Docker volume, follow
the backup, copy, and verification steps in the repository [README](../README.md#import-an-existing-database).

## Timer

`elapsed_seconds` accumulates only while status is RUNNING, measured with a
monotonic clock (not loop iterations). It is persisted by a heartbeat about
once per minute and inside every chunk-commit transaction. A clean stop
(completion, `--max-chunks`, `pause`, error) flushes the time and sets PAUSED;
completion sets COMPLETED. Time is never reset.

## Crash / restart behavior

- A chunk (and a block row) exists in SQLite only after its transaction commits; `current_position`
  is the end of the contiguous committed prefix. Uncommitted work is recomputed.
- If the previous process died while RUNNING, startup counts time only up to the
  last persisted `last_runtime_update`, increments `crash_recoveries`, and issues a
  new `run_token` (the dead run can no longer commit).
- If the RUNNING row was updated less than 180 s ago, startup is refused unless
  `--recover` is given, because the old process might still be alive.
- There is no signal handler: `docker stop` is treated as a crash and loses at most
  about one heartbeat interval of runtime, never any verified chunk. For a clean
  stop use `a220091-campaign pause`.
- A COMPLETED campaign cannot be restarted.

## Local crash test

`scripts\crash_recovery_test.ps1` kills a real run mid-flight on a temporary database, recovers it with `--recover`, and compares the result root with an uninterrupted run.
