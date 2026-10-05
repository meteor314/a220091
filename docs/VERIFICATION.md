# A220091 Verification and Recovery

> **Superseded:** persistent state is now the SQLite campaign database (see [CAMPAIGN.md](CAMPAIGN.md)). References below to `--checkpoint-dir`, `--resume`, `--no-checkpoint` or `chunk-*.json` describe the retired JSON checkpoint system and no longer apply to the `benchmark` binary.

The single-n verifier remains the mathematical authority for one value. Rayon
only distributes independent contiguous chunks. Each checkpoint worker calls
that verifier for every `n` in its chunk and streams canonical records into the
chunk SHA-256 calculation.

## Checkpoint lifecycle

1. Construct a deterministic chunk filename from its numeric start.
2. Compute every result in ascending `n` order.
3. Hash canonical binary records and collect only chunk statistics and
   zero-solution values.
4. Serialize a typed checkpoint to a temporary file in the checkpoint
   directory.
5. Write, flush, and `sync_all` the temporary file.
6. Rename it to the deterministic final filename.

Temporary and final files are kept in the same directory and therefore assume
the directory is on one filesystem. Rename is atomic for the normal POSIX
same-filesystem case. On Windows, replacing an existing final file is handled
by removing the invalid or stale final file before rename; a valid completed
checkpoint is never rewritten during normal resume.

## Resume

Resume scans expected chunks in numeric order and accepts only a contiguous
validated prefix. A missing file ends the prefix. Malformed, hash-corrupted,
or numerically invalid files are reported as recovery warnings and recomputed
from that frontier. A configuration mismatch is a hard error. Files after a
gap never cause a range to be skipped.

The final global root is reconstructed from checkpoint hashes in numeric chunk
order, never worker completion order.

Checkpoint workers optionally append compact completion records to the file
named by `A220091_LOG_FILE`. The logging failure path is deliberately
non-fatal: checkpoint durability and mathematical verification do not depend
on the log file.

## CLI

Normal development benchmarks do not write checkpoints unless requested:

```text
cargo run --release --bin benchmark -- --start 8070 --end 100000 \
    --threads 4 --chunk-size 10000 --no-checkpoint
```

An explicit resumable run uses:

```text
cargo run --release --bin benchmark -- --start 8070 --end 100000 \
    --threads 4 --chunk-size 10000 \
    --checkpoint-dir results/checkpoints
```

Resume uses the same command with `--resume`. Mathematical configuration
changes, including mode, thread count, chunk size, prime limit, or Q-list,
are rejected rather than silently mixed.

Hashes provide integrity and reproducibility identifiers. They are not proofs
of the A220091 conjecture and do not replace independent verification.

## Optional Discord notifications

Discord is an external observability layer only. The notifier is disabled when
`DISCORD_WEBHOOK_URL` is absent and can also be disabled explicitly with
`--no-discord`. The webhook URL is read only from that environment variable;
it is never stored in source, checkpoints, hashes, manifests, Docker layers,
or diagnostics. Do not commit `.env`; rotate any webhook that has been exposed.

The supported events are `START`, `DAILY_PROGRESS`, `IMPORTANT_ERROR`, and
`COMPLETED`, with recovery/stopped event types available to callers. Daily
state is persisted separately from mathematical checkpoints, normally below
the runtime data directory as `notifications/state.json`, and is sent at most
once per 24 hours when completed-through advances. Restarting therefore does
not reset the daily-notification decision.

Requests use a five-second timeout and at most two attempts. HTTP 429
`Retry-After` values are capped at two seconds. Any configuration, network,
DNS, HTTP, or state failure is non-fatal and is reported without the webhook
URL; verification and checkpoint/hash logic continue independently.

No Discord message is mathematical evidence or part of the result root.

## Independent Python verifier

`independent/verifier.py` is intentionally separate from the Rust production
engine. It does not import or use `PrimeBitmap`, `QList`, the Rust verifier, or
the Rust hash implementation.

For individual representations it uses trial division. For small range
cross-checks it builds its own byte-per-value Python sieve, enumerates integer
`q` values directly, independently applies the parity-specific bounds, and
counts only admissible Q candidates until the first representation is found.

For every supplied result record it independently checks `n`, `status`, `p`,
`q`, the exact equation, `p > q`, primality of `p` and `q`, and primality of
`6*q-1` and `6*q+1`. A zero-status record is checked by independently
recomputing the complete small-range search; it is not accepted merely because
Rust reported it.

Run the independent unit tests with:

```text
python -m unittest independent/test_verifier.py
```

The Rust integration test invokes the Python verifier and compares exact
records against both the Rust reference and optimized implementations for
`1..=10000`, plus `8068..=8072` and `18678..=18682`. The first even domain
value is `8070`; `18680` is the odd-domain threshold parameter, while the first
odd integer at or above it is `18681`.

This implementation is an independent bug detector for small validation
ranges. It is not a proof, does not establish the infinite conjecture, and is
not intended to be the high-performance production engine for large ranges.

## Real process-kill recovery test

On Windows, run:

```text
powershell -ExecutionPolicy Bypass -File scripts/crash_recovery_test.ps1
```

The script builds `target/release/benchmark.exe`, starts the actual executable
for `8070..=100000` with four threads and chunk size `1000`, waits until a
`chunk-*.json` file exists, and forcibly terminates the process with
`Stop-Process -Force`. It then restarts the same executable with `--resume`,
runs a clean uninterrupted execution in a separate directory, and compares
the result root, Q-list hash, candidate total, and processed count.

Finally, it changes one completed checkpoint's result hash, resumes again,
requires a non-empty recovery warning, and checks that recomputation produces
the clean result root. The script reports the checkpoint filename that existed
when the process was killed. It uses temporary directories and removes them
after the comparison; the test does not modify the tracked checkpoint fixtures.