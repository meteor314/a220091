# Intel N100 Deployment

> **Superseded:** persistent state is now the SQLite campaign database (see [CAMPAIGN.md](CAMPAIGN.md)). References below to `--checkpoint-dir`, `--resume`, `--no-checkpoint` or `chunk-*.json` describe the retired JSON checkpoint system and no longer apply to the `benchmark` binary.

This document describes the production deployment configuration for the
Intel N100 host running Ubuntu 24.04 LTS with Rootless Docker.

## Requirements

- Ubuntu 24.04 LTS, 64-bit.
- Rootless Docker (systemd user service running under host user `meteor`).
- Persistent storage for checkpoints, logs, notifications state, and reports.
- No source checkout or secrets in the checkpoint directory.
- Restrictive `.env` file for optional Discord notification credentials.

Verify Docker status on the host with:

```text
docker version
docker info
```

## Rootless Docker User & Permission Model

### Why writes fail with default UID 10001 or `--user "$(id -u):$(id -g)"`

Rootless Docker runs inside a Linux user namespace created by the unprivileged
host user (`meteor`, UID 1000, GID 1000). The namespace maps user IDs via
`/etc/subuid` and `/etc/subgid`:

1. **In-container UID 0 (root)** maps directly to **host UID 1000 (`meteor`)**.
2. **In-container UIDs 1..65535** map to **subordinate host UIDs (e.g. 100000..165534)**.

When host directories in `/srv/a220091` are owned by `meteor:meteor`:

- **Image default UID 10001 (`a220091`)**: Inside the container namespace,
  UID 10001 translates to host UID `110000`. This subordinate UID does not own
  `/srv/a220091`, causing write operations to fail with
  `Permission denied (os error 13)`.
- **`--user "$(id -u):$(id -g)"`**: On the host, `id -u` evaluates to `1000`.
  Passing `--user 1000:1000` to rootless Docker instructs the container to run
  as *in-container* UID 1000, which maps to host subordinate UID `100999`.
  Because host UID 100999 is not `meteor` (1000), write operations also fail
  with `Permission denied (os error 13)`.

### The Rootless Solution: `--user 0:0`

Inside rootless Docker, running the container with `--user 0:0` (or `--user 0`):

- Maps the container process directly to host user `meteor:meteor` (UID 1000, GID 1000).
- **Keeps the container non-root from the host's perspective**: The process
  has zero host root privileges, cannot access host files outside `meteor`'s
  permissions, and cannot elevate privileges.
- Matches host file ownership on `/srv/a220091` seamlessly without requiring
  `chmod 777` or `sudo chown`.
- Checkpoints, logs, and notification state created by the container are written
  as `meteor:meteor` with standard secure file modes (`0644`/`0750`).

## Directory Layout & Host Preparation

As the host user `meteor`:

```bash
mkdir -p /srv/a220091/{checkpoints,logs,notifications,results,config}
chmod 750 /srv/a220091 /srv/a220091/{checkpoints,logs,notifications,results,config}
```

Host layout:

```text
/srv/a220091/
    checkpoints/        # Chunk checkpoint JSON records
    logs/               # Execution and thermal logs
    notifications/      # Daily notification state (state.json)
    results/            # Final summary reports
    config/             # Optional runtime configuration (.env)
```

Mount `/srv/a220091` at `/var/lib/a220091` in the container. Keep `.env` inside
`/srv/a220091/config/` with `chmod 600` permissions.

## Build and Image Metadata

From the repository checkout:

```bash
docker build --tag a220091:phase11 .
scripts/image_metadata.sh a220091:phase11
```

The metadata command records the local Git commit, image ID, creation time, and
repository digests. The image contains only the compiled binary and required
system libraries; it does not contain `.env`, checkpoints, logs, or secrets.

## CLI & Debug Usage

The Docker image defines the benchmark binary as its `ENTRYPOINT`.

To run an interactive shell or debug command inside the container, explicitly
override the entrypoint using `--entrypoint /bin/sh`. Do **not** pass `sh` as
an application argument:

```bash
docker run --rm -it --user 0:0 --entrypoint /bin/sh \
  --mount type=bind,source=/srv/a220091,target=/var/lib/a220091 \
  a220091:phase11
```

## Development Smoke Run

Run the smoke test with Discord disabled to validate bind mount write access,
checkpoint generation, and mathematical integrity:

```bash
docker run --rm --user 0:0 \
  --mount type=bind,source=/srv/a220091,target=/var/lib/a220091 \
  a220091:phase11 \
  --start 8070 --end 100000 --mode conjecture-domain \
  --threads 4 --chunk-size 10000 --no-discord \
  --checkpoint-dir /var/lib/a220091/checkpoints
```

Expected output:
- `Discord disabled`
- `n processed: 91931`
- `zero-solution n (conjecture domain): []`
- `Q-list SHA-256: bdae99ec86f8c6d97e69730537279033ae37bfdeecaf4ac979af373baa92c5d5`
- `result root: 0d3c157ccd38a0f5fc6eee59069b9bf88e639e07f50c67543752e203c8dcf419`
- 10 checkpoint chunk files created in `/srv/a220091/checkpoints/`

## Optional Discord Injection

Discord notifications are optional and isolated from the core computation.
To enable notifications, create `/srv/a220091/config/.env` with:

```text
DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/...
```

Ensure restrictive file permissions (`chmod 600 /srv/a220091/config/.env`).

Run production benchmark:

```bash
docker run --rm --user 0:0 \
  --env-file /srv/a220091/config/.env \
  --mount type=bind,source=/srv/a220091,target=/var/lib/a220091 \
  a220091:phase11 \
  --start 8070 --end 100000 --threads 4 --chunk-size 10000 \
  --checkpoint-dir /var/lib/a220091/checkpoints
```

If network or webhook delivery fails, a warning is printed to stderr and the
computation continues normally without disruption.

## Initial Runtime Settings

Start with:

```text
--threads 4
--chunk-size 1000000
```

These are starting values only. Benchmark one through four threads and several
chunk sizes on the N100. Primality is answered by a segmented `PrimeOracle`
(`--segment-size`, `--cache-segments`), not a single bitmap sized to `3N+1`;
resident prime-oracle memory is bounded by segment size times cache capacity
(a few MB by default) regardless of `N`, in addition to Q-list, process,
filesystem cache, and OS memory. Do not infer N100 thermal behavior from
another CPU.

## Resume and Stopping

Resume with the identical mathematical/runtime configuration:

```bash
docker run --rm --user 0:0 \
  --env-file /srv/a220091/config/.env \
  --mount type=bind,source=/srv/a220091,target=/var/lib/a220091 \
  a220091:phase11 \
  --start 8070 --end 100000 --threads 4 --chunk-size 10000 \
  --checkpoint-dir /var/lib/a220091/checkpoints --resume
```

Stop normally with SIGTERM when possible and preserve the checkpoint directory.
If the process or container is forcibly stopped, restart with `--resume` and
inspect recovery warnings before accepting the result.

The Linux executable recovery procedure is:

```bash
scripts/crash_recovery_test.sh
```

Set `A220091_IMAGE` to the image under test. It waits for an actual checkpoint,
terminates the actual container, resumes, performs a clean comparison, corrupts
a checkpoint, and verifies corruption reporting and recomputation.

## Logs and Diagnostics

Collect host information:

```bash
scripts/system_info.sh > /srv/a220091/logs/system-info.txt
```

Run thermal monitoring independently:

```bash
scripts/thermal_monitor.sh 60 /srv/a220091/logs/thermal.csv
```

To start production logging with persistent stdout/stderr capture:

```bash
scripts/run_production.sh docker run --rm --user 0:0 \
  --env-file /srv/a220091/config/.env \
  --mount type=bind,source=/srv/a220091,target=/var/lib/a220091 \
  a220091:phase11 \
  --start 8070 --end 100000 --threads 4 --chunk-size 10000 \
  --checkpoint-dir /var/lib/a220091/checkpoints
```

## Safe Image Updates

1. Stop the active run safely and preserve `/srv/a220091/checkpoints`.
2. Save the old image ID/digest and current manifest output.
3. Build or pull the new image and record its metadata.
4. Run the small smoke range with a separate validation directory.
5. Compare Q hash, per-chunk hashes, root, counts, and zero classifications.
6. Resume only when checkpoint configuration and source compatibility match.

Never silently mix checkpoints from incompatible mathematical, hashing, or
configuration versions. A result root is an integrity identifier, not a proof
of the infinite conjecture.
