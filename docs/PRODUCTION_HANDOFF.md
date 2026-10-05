# Production Handoff & Phase Status

> **Superseded:** persistent state is now the SQLite campaign database (see [CAMPAIGN.md](CAMPAIGN.md)). References below to `--checkpoint-dir`, `--resume`, `--no-checkpoint` or `chunk-*.json` describe the retired JSON checkpoint system and no longer apply to the `benchmark` binary.

## Project Status Overview

- **Current State:** Production campaign in progress on Intel N100; completed through $n = 19{,}800{,}000{,}000$. The next block ($19{,}800{,}000{,}001..19{,}900{,}000{,}000$) was blocked by unbounded prime-bitmap memory and is fixed but **not yet run**.
- **Target Platform:** Intel N100 (4C/4T, ~7.5 GiB RAM, Ubuntu 24.04 LTS, Rootless Docker under user `meteor`).
- **Mathematical Scope:** OEIS A220091 existence verifier on conjecture domain ($n \ge 8070$ even, $n \ge 18680$ odd).

---

## Phase Execution History

| Phase | Description | Status | Verification Artifacts |
|---|---|---|---|
| **Phase 1** | Mathematical definition, reference verifier, core tests | Completed | `tests/reference.rs` |
| **Phase 2** | `PrimeBitmap` bit-sieve and global `build_q_list` | Completed | `src/prime_bitmap.rs`, `src/q_filter.rs` |
| **Phase 3** | Optimized existence verifier (`find_representation`) | Completed | `src/verifier.rs` |
| **Phase 4** | Independent Python cross-verifier and cross-checks | Completed | `independent/verifier.py`, `tests/independent_crosscheck.rs` |
| **Phase 5** | Rayon multi-threaded chunking verification | Completed | `src/parallel.rs` |
| **Phase 6** | Canonical binary hashing format & deterministic result root | Completed | `src/hashing.rs`, `docs/RESULT_FORMAT.md` |
| **Phase 7** | Atomic crash-resilient chunk checkpointing & resume | Completed | `src/checkpoint.rs` |
| **Phase 8** | Crash recovery test suites (Linux bash & Windows PowerShell) | Completed | `scripts/crash_recovery_test.sh`, `scripts/crash_recovery_test.ps1` |
| **Phase 9** | Multi-stage Docker packaging (`rust:1.88.0` builder, `debian:bookworm-slim`) | Completed | `Dockerfile`, `scripts/image_metadata.sh` |
| **Phase 10** | Discord notification sink with 24h throttling & failure isolation | Completed | `src/notifier.rs` |
| **Phase 11** | Rootless Docker deployment hardening on Ubuntu 24.04 / N100 (`--user 0:0`) | Completed | `docs/N100_DEPLOYMENT.md`, `scripts/crash_recovery_test.sh` |
| **Phase 12** | Production handoff documentation & algorithm specification | Completed | `docs/ALGORITHM.md`, `.github/copilot-instructions.md`, `docs/PRODUCTION_HANDOFF.md` |
| **Phase 13** | `u32` -> `u64` migration for `q` (fixes overflow above `n \approx 8.6e9`) | Completed | `src/q_filter.rs`, `src/verifier.rs`, `src/hashing.rs` |
| **Phase 14** | Segmented, bounded-memory `PrimeOracle` (fixes `O(N)` bitmap swap thrashing above `n \approx 1.99e10`) | Completed, **not yet run in production** | `src/prime_oracle.rs` |

---

## Technical Invariants & Verification Reference

### Reference Hashes ($N = 100\,000$, range $8070..=100000$, chunk size $10\,000$)
- **Q-list SHA-256:** `bdae99ec86f8c6d97e69730537279033ae37bfdeecaf4ac979af373baa92c5d5`
- **Result Root SHA-256:** `0d3c157ccd38a0f5fc6eee59069b9bf88e639e07f50c67543752e203c8dcf419`
- **Processed Count:** $91\,931$
- **Total Q-tests:** $592\,326$
- **Zero Solutions (Conjecture Domain):** `[]` (None)
- **Zero Solutions (All):** `[11329, 13429, 13829, 18379, 18679]`

### Rootless Docker Deployment Configuration
- Host User: `meteor:meteor` (`UID 1000`, `GID 1000`)
- Bind Mount: `/srv/a220091` -> `/var/lib/a220091`
- Docker Flag: `--user 0:0` (maps to `meteor:meteor` in rootless user namespace)
- Debug Override: `--entrypoint /bin/sh`
- Runtime Secret: `/srv/a220091/config/.env` (`chmod 600`)

---

## Next Steps for Operator

1. **Deploy to Target Intel N100 Host:**
   - Follow instructions in `docs/N100_DEPLOYMENT.md`.
2. **Execute Validation Smoke Test:**
   - Run the smoke test with `--no-discord`.
   - Verify checkpoint files appear in `/srv/a220091/checkpoints/` and match reference root `0d3c...`.
3. **Run Thermal & Baseline Measurement:**
   - Execute `scripts/thermal_monitor.sh` and `scripts/system_info.sh`.
   - Benchmark 1, 2, 4 threads and chunk sizes on N100 hardware.
4. **Decision Gate for Large Intervals:**
   - Do not launch large runs ($10^7$, $10^8$, $10^9$) until initial N100 baseline and thermal benchmarks are recorded.
5. **Segmented `PrimeOracle` rollout (Phase 14):**
   - Completed campaign checkpoints through $19{,}800{,}000{,}000$ are untouched; hashing semantics are preserved for ranges already supported.
   - The next block ($19{,}800{,}000{,}001..19{,}900{,}000{,}000$) has **not** been run yet. Resume it only on explicit instruction, starting with `--segment-size`/`--cache-segments` defaults and a memory-monitored smoke test before committing to the full block.
