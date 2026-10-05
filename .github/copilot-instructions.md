# Instructions for GitHub Copilot in A220091

## Project Identity & Mission

This repository develops a high-performance, mathematically rigorous, reproducible computational verifier for **OEIS A220091**.

Core Priorities (in strict order):
1. **Mathematical correctness**
2. **Independent verifiability**
3. **Reproducibility and auditability**
4. **Fault tolerance and resumability**
5. **Performance**
6. **Low memory overhead**

## Key Invariants & Rules

1. **Do not modify the mathematical verifier** or formula without explicit requirement.
2. **Mathematical reduction:**
   - Even $n$: $n = p + q$, $p > q$, $q < n/2$, $p = n - q$
   - Odd $n$: $n = p + 2q$, $p > q$, $q < n/3$, $p = n - 2q$
   - $q$ prime, $6q - 1$ prime, $6q + 1$ prime
3. **Conjecture Domain:**
   - Even $n \ge 8\,070$
   - Odd $n \ge 18\,680$
   - Known zero solutions outside domain: $\{11329, 13429, 13829, 18379, 18679\}$
4. **Determinism & Hashing:**
   - Q-list SHA-256 and global Result Root SHA-256 must remain deterministic and independent of thread count.
5. **Never expose secrets:**
   - Webhook URLs must never be committed, embedded in images, logged, or printed to the console.
6. **Rootless Docker deployment:**
   - Host user `meteor:meteor` mounts `/srv/a220091` to container `/var/lib/a220091`.
   - Container must be executed with `--user 0:0` to map cleanly inside user namespace.
   - Interactive shell debug commands must use `--entrypoint /bin/sh`.
7. **Testing:**
   - Ensure `cargo fmt --check` and `cargo test` pass before and after changes.
8. **No fabricated benchmarks:**
   - Do not claim unmeasured performance or invent numbers.
