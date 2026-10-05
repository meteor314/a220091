# A220091 — LLM Coding Agent Implementation Brief

## Purpose

You are an expert software engineer and computational-number-theory engineer.

Your task is to design and implement a **high-performance, reproducible, independently verifiable computational verifier for OEIS A220091**.

This is not a toy program and not merely a benchmark.

The final software must be suitable for a long-running computation on a small always-on machine (initial target: Intel N100, 4 cores, ~7.5 GiB RAM), with the possibility of extending the verified range substantially beyond the currently published range.

The project must prioritize, in this order:

1. **Mathematical correctness**
2. **Independent verifiability**
3. **Reproducibility and auditability**
4. **Fault tolerance and resumability**
5. **Performance**
6. **Low memory overhead**

Do not sacrifice items 1–4 for a small performance gain.

---

# 1. Mathematical target

The target is **OEIS A220091**.

The sequence counts the number of representations

\[
n = p + q + (n \bmod 2)q
\]

such that:

- `p > q`
- `p` is prime
- `q` is prime
- `6q - 1` is prime
- `6q + 1` is prime.

Therefore:

### Even n

\[
n = p + q
\]

### Odd n

\[
n = p + 2q.
\]

The existence conjecture is:

\[
a(n)>0.
\]

The computational verification target is therefore:

> For every `n` in a specified interval, find at least one valid representation.

A computational verification is **not a mathematical proof** of the conjecture. The software must use precise language such as:

```text
No counterexample was found in the verified interval.
```

Never claim that the conjecture has been mathematically proved.

---

# 2. Current published baseline

Before making any performance or record claim:

1. Verify the current OEIS A220091 page.
2. Record the currently stated verification bound.
3. Check whether newer independent computations exist.
4. Never assume an old bound is still the world record.

At the time this specification was written, OEIS A220091 stated verification through approximately:

```text
10^7
```

This number must be checked again at runtime/project start before publishing a result.

---

# 3. Core mathematical reduction

For a given `n`, we do **not** need to test arbitrary `q`.

## Even n

We have:

\[
n=p+q
\]

and

\[
p>q.
\]

Therefore:

\[
n-q>q
\]

so:

\[
q < n/2.
\]

Thus:

```text
q <= floor((n - 1) / 2)
```

## Odd n

We have:

\[
n=p+2q
\]

and

\[
p>q.
\]

Therefore:

\[
n-2q>q
\]

so:

\[
q<n/3.
\]

Thus:

```text
q <= floor((n - 1) / 3)
```

These bounds must be implemented explicitly and tested.

Do not iterate over impossible `q` values.

---

# 4. Global q prefilter

The conditions

```text
q is prime
6q - 1 is prime
6q + 1 is prime
```

depend only on `q`, not on `n`.

Therefore create the admissible set once:

```text
Q = { q :
      q is prime
      AND 6q-1 is prime
      AND 6q+1 is prime }
```

Sort `Q` in ascending order.

Then the hot verification loop only needs to test whether `p` is prime.

This is one of the most important algorithmic reductions.

---

# 5. Required prime range

Suppose the target verification limit is:

```text
N
```

Then:

```text
q <= N/2
```

and therefore:

```text
6q + 1 <= 3N + 1
```

So the prime lookup structure must support:

```text
0 .. 3N+1
```

For:

```text
N = 1_000_000_000
```

the upper bound is:

```text
3_000_000_001
```

A one-bit-per-integer bitmap needs roughly:

```text
3e9 bits
≈ 375 MB decimal
≈ 358.5 MiB
```

This is acceptable within ~7.5 GiB RAM, provided unnecessary duplicate structures are avoided.

---

# 6. Prime generation

Use a mature prime-generation implementation rather than writing an optimized sieve from scratch unless there is a compelling measured reason.

Preferred component:

```text
primesieve
```

Investigate and use the current, maintained Rust-compatible interface or a small, well-contained FFI layer if that is more robust.

Before coding against the API:

- inspect the current version/API;
- pin the version;
- document it;
- make the build reproducible.

Do not make assumptions about a crate API that has not been checked.

---

# 7. Prime lookup structure

For fast random primality access in the main loop, use a compact bitmap.

Preferred representation:

```rust
Vec<u64>
```

with a deterministic mapping such as:

```text
word_index = n >> 6
bit_index  = n & 63
```

The API should provide:

```rust
is_prime(n: u64) -> bool
```

with minimal overhead.

The implementation must carefully handle:

- `0`
- `1`
- `2`
- odd/even behavior
- upper bounds
- out-of-range access
- integer conversion.

For the initial target range, `u64` should be the mathematical internal type even if some compact storage uses `u32`.

---

# 8. Q storage

For `N <= 10^9`, admissible `q` values satisfy:

```text
q <= 5e8
```

so `u32` is sufficient for storing `q`.

Consider:

```rust
Vec<u32>
```

for the `Q` list to improve cache density.

Convert to `u64` only when necessary for arithmetic.

Do not introduce this optimization until correctness is established, but it should be benchmarked.

---

# 9. Main existence verifier

The existence verifier should conceptually do:

```text
for each n:
    determine q_max from parity

    iterate admissible q <= q_max:
        if n is even:
            p = n - q
        else:
            p = n - 2*q

        if p > q and is_prime(p):
            success
            stop searching this n
```

The moment one valid representation is found, stop.

Do not search for all representations in the existence mode.

---

# 10. Two operating modes

Implement two logically distinct modes.

## Mode A: existence

Purpose:

```text
Does a(n) > 0?
```

This is the performance-critical record-verification mode.

It should stop at the first valid representation.

## Mode B: count

Purpose:

```text
Compute exact a(n)
```

This mode continues through all admissible `q`.

It will be much slower and exists mainly for:

- testing;
- cross-validation;
- small ranges;
- reproducing known sequence values.

---

# 11. Separate reference implementation from optimized implementation

This is mandatory.

## Reference implementation

Requirements:

- simple;
- easy to read;
- mathematically direct;
- not heavily optimized.

Use it on small ranges.

## Optimized implementation

Requirements:

- optimized memory access;
- prefiltered Q;
- compact prime bitmap;
- chunking;
- parallelism;
- checkpointing;
- hashing.

The optimized implementation must continuously be tested against the reference implementation on manageable ranges.

---

# 12. Independent verifier

Create a separate implementation that does not share the same critical logic.

This verifier should be intentionally simpler.

Possible technology:

```text
Python
SageMath
PARI/GP
```

or a second Rust implementation with substantially different internal logic.

The independent verifier must be able to validate:

```text
n
p
q
```

by explicitly checking:

```text
p is prime
q is prime
6q-1 is prime
6q+1 is prime
p > q
n == p + q + (n % 2) * q
```

It must also be able to validate a chunk summary and/or regenerate a deterministic result hash for smaller ranges.

Do not simply copy the optimized code into a second file.

---

# 13. Cross-check strategy

Before long computations:

```text
small brute-force range
        ↓
reference implementation
        ↓
optimized implementation
        ↓
independent implementation
```

All outputs must match.

Recommended progression:

```text
10^3
10^4
10^5
10^6
10^7
```

Then, depending on measured performance:

```text
10^8
```

and only later consider:

```text
10^9
```

---

# 14. Parallelization

Parallelize across independent `n` ranges/chunks.

Do not normally parallelize the `q` loop within one `n`.

Preferred model:

```text
chunk 0 ─ worker 0
chunk 1 ─ worker 1
chunk 2 ─ worker 2
chunk 3 ─ worker 3
```

Use Rayon or an equivalent robust task scheduler.

Start benchmarks at:

```text
1 thread
2 threads
3 threads
4 threads
```

Do not assume that maximum thread count is optimal.

The machine is an Intel N100, so thermal throttling and memory behavior matter.

---

# 15. Chunking

The verifier must process a contiguous range in chunks.

Example:

```text
[1, 1,000,000]
[1,000,001, 2,000,000]
...
```

The exact chunk size must be configurable.

Example initial value:

```text
1,000,000
```

but benchmark alternatives.

Chunk size controls:

- checkpoint frequency;
- recovery cost;
- hash overhead;
- scheduling overhead;
- memory usage.

---

# 16. Checkpointing requirements

Every completed chunk must be resumable.

The checkpoint must record at least:

```text
sequence
algorithm version
git commit
range start
range end
completed through
prime limit
Q hash
prime bitmap hash
result hash
statistics
timestamp
```

Checkpoint writing must be atomic.

Preferred pattern:

```text
write temporary file
    ↓
flush/fsync
    ↓
atomic rename
```

Do not overwrite the active checkpoint in place.

The program must support:

```text
--resume
```

and recover automatically from the latest valid checkpoint.

---

# 17. Crash/reboot behavior

Assume:

- power loss;
- reboot;
- process kill;
- kernel restart;
- Docker restart;
- filesystem corruption of the currently written temporary checkpoint.

A valid previous checkpoint must remain usable.

The program must never silently skip a range after restart.

---

# 18. Canonical result representation

Define a deterministic binary representation for result hashing.

For every `n`, encode something conceptually equivalent to:

```text
n
status
q
p
```

with exact:

- integer sizes;
- endian order;
- status encoding.

Do not hash human-readable output whose formatting might change.

Document the binary format.

---

# 19. Hashing

Use SHA-256.

Compute at least:

```text
Q hash
prime-data hash
chunk result hash
global result root
binary SHA-256
configuration hash
```

A possible chained root is:

```text
H0 = SHA256(header)

H1 = SHA256(H0 || chunk_hash_0)

H2 = SHA256(H1 || chunk_hash_1)

...

Hn = final result root
```

The exact construction must be specified in documentation.

Important:

> A cryptographic hash is an integrity mechanism, not a mathematical proof.

---

# 20. Manifest

Produce a final machine-readable manifest.

Suggested fields:

```json
{
  "sequence": "A220091",
  "verification_start": 1,
  "verification_end": 100000000,
  "prime_limit": 300000001,
  "algorithm_version": "...",
  "git_commit": "...",
  "rust_version": "...",
  "compiler_version": "...",
  "primesieve_version": "...",
  "os": "...",
  "kernel": "...",
  "cpu": "...",
  "ram": "...",
  "thread_count": 4,
  "chunk_size": 1000000,
  "q_count": 12345,
  "q_sha256": "...",
  "prime_data_sha256": "...",
  "result_root": "...",
  "counterexamples": 0,
  "runtime_seconds": 0
}
```

Use a versioned schema.

---

# 21. Statistics

Per chunk, record:

```text
n_start
n_end
n_processed
n_success
n_fail
total_q_tests
min_q_tests
max_q_tests
mean_q_tests
median_q_tests (if practical)
runtime
throughput
```

Also record global totals.

These measurements are important because performance assumptions must be validated empirically.

---

# 22. Performance measurement

Do not estimate long runtimes from unrelated CPUs.

The only useful final performance number is the sustained performance on the target machine.

Benchmark:

```text
10^5
10^6
10^7
10^8
```

and record:

```text
n/sec
q-tests/sec
CPU usage
RAM
temperature
clock frequency
```

For long benchmarks, measure after thermal stabilization.

Do not use an initial burst of performance as the final estimate.

---

# 23. Thermal monitoring

The machine is intended for 24/7 operation.

Create a lightweight system monitor that periodically records:

```text
timestamp
CPU temperature
CPU frequency
load
memory usage
```

If available, record:

```text
throttling indicators
```

Performance estimates must use stable post-heating performance.

---

# 24. Do not over-optimize prematurely

First make this sequence work:

```text
reference implementation
        ↓
bitmap
        ↓
Q prefilter
        ↓
optimized existence mode
        ↓
parallel execution
        ↓
checkpointing
        ↓
hashing
```

Only afterward investigate:

- q storage compression;
- separate odd/even loops;
- hand-unrolled loops;
- cache layout;
- custom chunk scheduling;
- CPU-specific flags;
- segmented prime bitmaps;
- branch elimination;
- SIMD.

Every optimization must be justified by a benchmark.

---

# 25. Correctness invariants

The software must enforce/test:

For every accepted representation:

```text
p > q
is_prime(p)
is_prime(q)
is_prime(6*q-1)
is_prime(6*q+1)
n == p + q + (n % 2) * q
```

For even n:

```text
n == p + q
```

For odd n:

```text
n == p + 2*q
```

Also validate:

```text
q <= floor((n-1)/2)    for even n
q <= floor((n-1)/3)    for odd n
```

---

# 26. Overflow policy

Use `u64` for arithmetic.

Explicitly guard expressions such as:

```text
6*q + 1
```

Do not depend on debug-mode overflow behavior.

The code must fail safely if a requested range exceeds the supported arithmetic/storage design.

---

# 27. Project structure

Recommended repository:

```text
a220091/
├── Cargo.toml
├── Cargo.lock
├── Dockerfile
├── README.md
├── LICENSE
│
├── src/
│   ├── main.rs
│   ├── config.rs
│   ├── prime_bitmap.rs
│   ├── q_filter.rs
│   ├── verify.rs
│   ├── parallel.rs
│   ├── checkpoint.rs
│   ├── hashing.rs
│   └── stats.rs
│
├── independent/
│   └── verifier.py
│
├── tests/
│   ├── known_values.rs
│   ├── brute_force.rs
│   └── crosscheck.rs
│
├── scripts/
│   ├── benchmark.sh
│   ├── system_info.sh
│   └── verify_result.sh
│
├── docs/
│   ├── ALGORITHM.md
│   ├── VERIFICATION.md
│   ├── REPRODUCIBILITY.md
│   └── RESULT_FORMAT.md
│
└── results/
    ├── checkpoints/
    ├── hashes/
    ├── logs/
    └── reports/
```

---

# 28. Command-line interface

Provide a clear CLI.

Example:

```bash
a220091 verify --start 1 --end 10000000
```

```bash
a220091 verify --start 1 --end 100000000 \
    --threads 4 \
    --chunk-size 1000000 \
    --resume
```

```bash
a220091 count --start 1 --end 10000
```

```bash
a220091 verify-representation \
    --n 31 \
    --p 17 \
    --q 7
```

```bash
a220091 benchmark --end 10000000
```

Do not require manual editing of source code for normal operation.

---

# 29. Logging

Logs must include:

```text
start
configuration
environment
prime-generation phase
Q-generation phase
verification phase
completed chunks
errors
checkpoint writes
final result
```

Use timestamps.

Avoid printing millions of lines during normal operation.

The default log should remain useful and compact.

Provide a verbose/debug mode separately.

---

# 30. Record-claim protocol

Never automatically print:

```text
WORLD RECORD
```

The software may report:

```text
verified through N
```

A human review must determine whether this actually exceeds all known published bounds.

Before publicizing a result:

1. Check OEIS.
2. Search recent literature.
3. Search recent computational announcements.
4. Compare exact definitions.
5. Confirm the interval is contiguous.
6. Confirm reproducibility.
7. Complete independent verification.

Only then describe it as a possible new computational record.

---

# 31. Publication-quality evidence package

If a substantial new bound is achieved, produce:

```text
source code
Git repository/commit
build instructions
Dockerfile
configuration
system information
prime-data hash
Q hash
per-chunk result hashes
global result root
logs
runtime
thermal data
independent verifier
independent cross-check
final report
```

The final report should explain exactly what was checked.

---

# 32. Required documentation

Before the first serious long run, write:

## `docs/ALGORITHM.md`

Describe:

- mathematical reduction;
- q filtering;
- prime lookup;
- existence logic;
- complexity;
- parallelization.

## `docs/VERIFICATION.md`

Describe:

- tests;
- independent verifier;
- result format;
- hashes;
- checkpoint semantics;
- audit procedure.

## `docs/REPRODUCIBILITY.md`

Describe:

- compiler;
- dependencies;
- versions;
- build flags;
- hardware;
- Docker;
- exact command line;
- expected outputs.

## `docs/RESULT_FORMAT.md`

Specify:

- canonical encoding;
- endianess;
- field sizes;
- statuses;
- SHA-256 calculation.

---

# 33. Testing matrix

Implement automated tests for:

```text
primality
Q generation
even n
odd n
p > q
boundary q
overflow handling
checkpoint recovery
hash determinism
result serialization
reference vs optimized
optimized vs independent
```

Use known small values from OEIS where applicable.

For example, verify known representations such as:

```text
31 = 17 + 2*7
32 = 29 + 3
```

with:

```text
q=7: 6q-1=41, 6q+1=43
q=3: 6q-1=17, 6q+1=19
```

---

# 34. Reproducibility test

The project must contain a command that verifies its own reproducibility on a small range.

Example:

```bash
./scripts/reproducibility_test.sh
```

Expected behavior:

1. build;
2. run small reference calculation;
3. run optimized calculation;
4. run independent verifier;
5. compare outputs;
6. compare hashes;
7. return nonzero on any mismatch.

---

# 35. Recovery test

Simulate interruption.

Example test:

```text
start computation
kill process during chunk k
restart with --resume
verify final result == uninterrupted run
```

This must be automated.

Checkpointing that has never been tested under failure should not be trusted for a months-long computation.

---

# 36. Determinism

The final result representation must be deterministic regardless of worker execution order.

Parallel workers may finish chunks in different orders, but the final manifest/result root must follow a fixed chunk ordering:

```text
chunk 0
chunk 1
chunk 2
...
```

Never concatenate results according to completion order.

---

# 37. Fault tolerance

If a worker fails:

- do not silently mark the chunk completed;
- retry or fail explicitly;
- preserve the previous valid checkpoint;
- report the failure.

If corrupted result data is detected:

```text
FAIL CLOSED
```

not:

```text
continue anyway
```

---

# 38. Memory discipline

Avoid keeping unnecessary copies.

At large N, the approximate memory budget should explicitly account for:

```text
prime bitmap
Q
worker state
buffers
runtime
OS/cache overhead
```

Do not allocate a second full prime bitmap accidentally.

Do not build enormous vectors of per-n text results.

Stream data and hashes.

---

# 39. Prime generation strategy

A practical first implementation is:

1. generate primes up to `3N+1`;
2. populate the prime bitmap;
3. scan primes up to `N/2`;
4. construct `Q`;
5. run the verifier.

The project should also document a future alternative:

```text
segmented prime data
```

if the full bitmap becomes a performance or storage limitation.

Do not implement the more complicated architecture unless measurements justify it.

---

# 40. Performance-critical loop

The final optimized loop should be as close as practical to:

```rust
for &q32 in q_prefix {
    let q = q32 as u64;

    let p = if even {
        n - q
    } else {
        n - 2 * q
    };

    if p <= q {
        continue;
    }

    if primes.is_prime(p) {
        return Some((p, q));
    }
}
```

But do not assume this exact code is optimal.

Profile first.

Then optimize based on actual measurements on the N100.

---

# 41. Performance metrics that matter

Do not optimize solely for:

```text
primes/second
```

The real metric is:

```text
verified n / second
```

and secondarily:

```text
q tests / second
```

The prime-generation phase is preprocessing.

The final project should report:

```text
prime generation time
Q generation time
verification time
total wall time
```

separately.

---

# 42. Important distinction: preprocessing vs verification

It is acceptable if prime preprocessing takes substantial time.

What matters is:

```text
total verified range
/
total wall-clock time
```

The program must not claim that a fast sieve benchmark means the A220091 computation itself is fast.

---

# 43. Avoid unjustified mathematical assumptions

Do not hard-code assumptions such as:

```text
average q tests ≈ log n
```

or:

```text
Q contains approximately X values
```

unless measured.

The software should collect the statistics needed to test these hypotheses empirically.

---

# 44. Benchmark report

Generate a machine-readable and human-readable benchmark report.

Example:

```text
N
threads
chunk_size
Q_count
prime_limit
prime_generation_seconds
Q_generation_seconds
verification_seconds
total_seconds
n_per_second
q_tests_per_second
RAM_peak
temperature_min
temperature_max
frequency_min
frequency_max
```

---

# 45. Decision gate for large N

Do not jump directly from a small test to `10^9`.

Use a gate:

```text
10^7
    ↓
correctness + independent verification
    ↓
10^8
    ↓
sustained thermal benchmark
    ↓
performance analysis
    ↓
decide whether 10^9 is justified
```

The decision must be based on measured N100 performance.

No optimistic duration promises.

---

# 46. Code quality requirements

The code should be:

- safe Rust wherever practical;
- documented;
- tested;
- modular;
- explicit about integer types;
- explicit about error handling;
- free of unnecessary `unsafe`;
- deterministic in output;
- easy for another engineer to audit.

Unsafe code, if required for FFI, must be isolated and documented.

---

# 47. External dependencies

Keep dependencies minimal.

For each dependency:

- pin a compatible version;
- record its license;
- record its version in the manifest;
- avoid unreviewed dependencies in the mathematical core.

Before finalizing dependencies, check their current APIs and maintenance status.

---

# 48. Security/integrity mindset

Treat result files as scientific artifacts.

Use:

```text
SHA-256
atomic writes
versioned schema
Git commit
immutable archived outputs
```

A copied result without its manifest and code version should not be considered a complete result.

---

# 49. Final acceptance criteria

The project is not complete until all of the following are true:

### Mathematical

- A220091 is implemented exactly.
- Even and odd formulas are correct.
- q bounds are correct.
- p > q is enforced.

### Engineering

- optimized Rust implementation exists;
- reference implementation exists;
- independent verifier exists;
- checkpoints work;
- resume works;
- hashes are deterministic;
- result format is documented.

### Validation

- small-range brute-force comparison passes;
- reference and optimized implementations agree;
- independent verifier agrees;
- interruption/recovery test passes;
- deterministic hash test passes.

### Performance

- benchmarked on the actual N100;
- thread count benchmarked;
- chunk size benchmarked;
- thermal behavior measured;
- memory usage measured.

### Publication readiness

- exact code version recorded;
- exact environment recorded;
- exact command recorded;
- chunk hashes available;
- global result root available;
- independent verification procedure documented.

---

# 50. Expected first deliverable from the coding agent

Do **not** immediately launch a huge computation.

The first deliverable should be a complete repository containing:

```text
1. reference implementation
2. optimized implementation
3. independent verifier
4. tests
5. checkpoint mechanism
6. hash mechanism
7. CLI
8. documentation
9. benchmark command
10. Dockerfile
```

Then execute:

```text
small correctness tests
        ↓
cross-validation
        ↓
benchmark
```

Only after these pass should the agent recommend a large verification run.

---

# 51. How the coding agent should behave

When implementing this project:

- do not fabricate benchmark numbers;
- do not claim an optimization works without measuring it;
- do not claim a record without checking current sources;
- do not silently change the mathematical definition;
- do not remove verification mechanisms because they are inconvenient;
- do not discard intermediate evidence;
- do not make runtime guarantees from unrelated hardware;
- clearly distinguish measured facts, assumptions, and estimates.

If an optimization conflicts with auditability, prefer auditability unless the benefit is substantial and demonstrable.

---

# 52. Final engineering objective

The intended final artifact is not merely:

```text
a220091 → OK
```

It is something closer to:

```text
A220091 Computational Verification

Range:
    [1, N]

Result:
    No counterexample found

Algorithm:
    documented

Source:
    Git commit SHA

Environment:
    documented

Prime data:
    SHA-256

Q list:
    SHA-256

Chunks:
    individually hashed

Global result:
    deterministic SHA-256 root

Verification:
    independently reproduced

Recovery:
    tested

Hardware:
    documented

Thermal behavior:
    documented

Runtime:
    measured
```

That is the standard the implementation should target.

---

# 53. First instruction to the coding agent

Before writing substantial code:

1. Inspect the current OEIS A220091 definition.
2. Inspect the current `primesieve`/Rust integration API.
3. Confirm current dependency versions.
4. Produce the repository architecture.
5. Implement the small reference verifier first.
6. Add tests.
7. Only then implement the optimized engine.

Do not skip directly to a speculative high-performance implementation.

