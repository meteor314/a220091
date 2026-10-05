# A220091 Reproducibility

> **Superseded:** persistent state is now the SQLite campaign database (see [CAMPAIGN.md](CAMPAIGN.md)). References below to `--checkpoint-dir`, `--resume`, `--no-checkpoint` or `chunk-*.json` describe the retired JSON checkpoint system and no longer apply to the `benchmark` binary.

Build and test with:

```text
cargo fmt
cargo test
```

The project records dependency versions in `Cargo.lock`. Phase 7 uses Rayon
for chunk scheduling, serde/serde_json for typed versioned checkpoint JSON,
and sha2 for SHA-256.

The initial validated Git commit is:

```text
1ab61e469fb9a013d05b09273a82aa59a0d8f712
```

For a deterministic small recovery exercise:

```text
cargo run --release --bin benchmark -- --start 8070 --end 100000 \
    --threads 4 --chunk-size 10000 \
    --checkpoint-dir results/checkpoints
cargo run --release --bin benchmark -- --start 8070 --end 100000 \
    --threads 4 --chunk-size 10000 \
    --checkpoint-dir results/checkpoints --resume
```

The second invocation reuses only validated contiguous checkpoints. The
machine-readable output includes the Q-list hash and global result root.
The optional `A220091_SOURCE_VERSION` environment variable is copied into
checkpoint metadata when set.

## Production Docker image

The production image uses a pinned `rust:1.88.0-bookworm` builder and a
pinned `debian:bookworm-20250630-slim` runtime image. No primesieve system
dependency is used by the current implementation. The final image contains
only the release benchmark executable and its minimal runtime libraries.

Build it from the repository root:

```text
docker build --tag a220091:phase11 .
```

Run a small resumable verification with host-persisted checkpoints under Rootless Docker:

```text
docker run --rm --user 0:0 \
    --mount type=bind,source=/srv/a220091,target=/var/lib/a220091 \
    a220091:phase11 \
    --start 8070 --end 100000 --mode conjecture-domain \
    --threads 4 --chunk-size 10000 --no-discord \
    --checkpoint-dir /var/lib/a220091/checkpoints
```

In Rootless Docker (running under host user `meteor:meteor`), passing `--user 0:0`
maps the container process directly to `meteor:meteor` in the host's unprivileged
user namespace, ensuring secure non-root host execution and clean file ownership
without requiring `chmod 777` or `sudo chown`.

The image does not contain `.env`, local checkpoints, results, webhook URLs,
or credentials. The benchmark JSON can record `A220091_GIT_COMMIT`,
`A220091_DOCKER_IMAGE_ID`, `A220091_SOURCE_HASH`, and `A220091_DIAGNOSTICS`
when those environment variables are supplied by the deployment wrapper.

Optional notifications are supplied only at runtime via external configuration:

```text
docker run --rm --user 0:0 --env-file /srv/a220091/config/.env \
    --mount type=bind,source=/srv/a220091,target=/var/lib/a220091 \
    a220091:phase11 \
    --start 8070 --end 100000 --threads 4 --chunk-size 10000 \
    --checkpoint-dir /var/lib/a220091/checkpoints
```

The `.env` file must be created and protected separately by the operator. It
must contain `DISCORD_WEBHOOK_URL` only on the deployment host and must never
be copied into the build context or image. Use `--no-discord` for development
and validation runs.

## N100 starting point

The previous development benchmark measured four threads as fastest on that
machine, but this is not an N100 result and is not an optimization claim. An
initial N100 run may start with `--threads 4` and `--chunk-size 1000000`, then
benchmark one through four threads and alternative chunk sizes. The prime
bitmap memory requirement is approximately one bit per integer up to `3N+1`,
plus Q-list, process, filesystem-cache, and OS overhead. Actual memory and
thermal behavior must be measured on the target server.

Both values remain runtime-overridable.

## Diagnostics and monitoring

Record host information before a run with:

```text
scripts/system_info.sh > logs/system-info.txt
```

The script reports CPU, logical/core information, RAM, kernel, OS, Rust,
Docker, Git commit, and available temperature/frequency data. Missing sensors
are reported as unavailable. Run the independent monitor separately:

```text
scripts/thermal_monitor.sh 60 logs/thermal.csv
```

Monitoring is observational and must not be a dependency of the mathematical
computation. `scripts/run_production.sh` records startup, command, checkpoint
completion lines, program output/errors, and completion status.

Benchmark runs without `--checkpoint-dir` do not persist checkpoints, avoiding
checkpoint I/O in performance measurements. No runtime, thermal, or large-N
estimate is implied by a small benchmark.

## Windows crash/restart validation

The executable-level recovery test is:

```text
powershell -ExecutionPolicy Bypass -File scripts/crash_recovery_test.ps1
```

It uses `Stop-Process -Force` only after observing a real checkpoint file,
then resumes the same range and compares it with a clean run. It also corrupts
one completed checkpoint and verifies that recovery reports the corruption and
recomputes the same global root. This test depends on Windows PowerShell and
the `target/release/benchmark.exe` produced by Cargo; a POSIX deployment needs
an equivalent process-management script.