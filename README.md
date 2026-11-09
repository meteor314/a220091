# A220091: Computational Investigation up to 100 Billion

### A large-scale computational investigation of a prime-representation conjecture

An independent computational investigation of [OEIS A220091](https://oeis.org/A220091), a prime-representation conjecture attributed to mathematician **Zhi-Wei Sun**.

The computation successfully covered the interval from 18,681 to 100,000,000,000, using a Lenovo ThinkPad X260 equipped with 8 GB of RAM and an Intel Core i5-6300U processor.

Over approximately 13.06 days of recorded computation, the main campaign evaluated:

**1,493,829,701,180 Q-candidate evaluations (approximately 1.49 trillion).**

The computation used segmented prime sieving, bounded-memory caching, four logical processing threads, deterministic SHA-256 result hashing, and transactional SQLite persistence.

The campaign reached its configured endpoint of 100 billion and was subsequently stopped. Further computation was considered too time-consuming relative to the available hardware.

---

## 1. Mathematical Background

### The conjecture

OEIS sequence [A220091](https://oeis.org/A220091) concerns the number of representations of a positive integer `n` in the form:

```text
n = p + q + (n mod 2) * q
```

where:

- `p` and `q` are prime numbers.
- `p > q`.
- `6q - 1` is prime.
- `6q + 1` is prime.

Let `a(n)` denote the number of valid representations.

The conjecture states that:

```text
a(n) > 0
```

for the following domains:

- Even integers: `n >= 8,070`.
- Odd integers: `n >= 18,680`.

In other words, every integer in these respective domains should have at least one representation satisfying all the prime constraints.

### Connection with Goldbach's conjecture

The problem has a Goldbach-like structure.

The classical Goldbach conjecture states that every even integer greater than 2 can be expressed as the sum of two primes:

```text
n = p + q
```

A220091 has a related but more restrictive structure.

For even integers:

```text
n = p + q
```

For odd integers:

```text
n = p + 2q
```

In both cases, `p` and `q` must be prime, `p > q`, and `q` must satisfy the additional conditions:

```text
6q - 1 is prime
6q + 1 is prime
```

Thus, A220091 can be viewed as a restricted Goldbach-type representation problem, with an additional prime condition on one of the summands.

It is not the classical Goldbach conjecture itself. The additional restrictions make the admissible representations considerably harder to find.

### The special prime condition

The prime `q` must satisfy:

```text
q, 6q - 1, 6q + 1 are all prime.
```

Consequently, `6q - 1` and `6q + 1` form a twin-prime pair centered at `6q`, with `q` itself also prime.

For example:

```text
q = 2
6q - 1 = 11
6q + 1 = 13
```

All three numbers are prime.

The computation therefore begins by identifying the admissible values of `q` before examining representations of individual integers.

---

## 2. Computational Objective

The objective was to investigate the conjecture over a substantially larger interval using a relatively modest personal computer.

The principal campaign covered:

```text
18,681 <= n <= 100,000,000,000
```

A separate completed campaign covered:

```text
1 <= n <= 100,000
```

Together, these campaigns cover the interval from 1 through 100 billion, assuming inclusive endpoints and complete, non-overlapping campaign records.

The original conjectured domains are included in the combined coverage.

### Why stop at 100 billion?

The computation was initially configured to reach 100 billion, and that target was successfully completed.

However, the execution [logs](./logs/discord-log.log) reveal an important practical limitation: the computational workload increased as the campaign progressed.

The number of Q-candidate evaluations in successive 100-million-sized blocks generally increased. Later portions therefore required more candidate processing than earlier portions.

This made continued computation increasingly expensive in terms of elapsed time.

Given the hardware available, extending the campaign beyond 100 billion would have required a substantial additional commitment of computing time.

The decision to stop was therefore deliberate.

The objective was to carry out a meaningful large-scale investigation, preserve its results, and establish a reproducible computational endpoint rather than run indefinitely.

---

## 3. Hardware and Execution Environment

The computation was performed on a Lenovo ThinkPad X260.

This is an older, mobile-class laptop rather than a dedicated workstation or computational server.

### Hardware specifications

| Component           | Specification       |
| ------------------- | ------------------- |
| Manufacturer        | Lenovo              |
| Model               | ThinkPad X260       |
| CPU                 | Intel Core i5-6300U |
| CPU architecture    | x86-64              |
| Physical CPU cores  | 2                   |
| Logical CPU threads | 4                   |
| Installed RAM       | 8 GB                |

### Software environment

| Component           | Version                               |
| ------------------- | ------------------------------------- |
| Operating system    | Ubuntu 26.04.1 LTS (Resolute Raccoon) |
| Kernel              | Linux 7.0.0-43-generic                |
| Architecture        | x86_64                                |
| Shell               | Bash 5.3.9                            |
| Package manager     | dpkg / snap                           |
| Parallel processing | Rayon                                 |
| Database            | SQLite                                |

### Execution configuration

| Parameter                   |       Value |
| --------------------------- | ----------: |
| Worker threads              |           4 |
| Chunk size                  |   1,000,000 |
| Block size                  | 100,000,000 |
| Prime sieve segment size    |        2^20 |
| Prime window cache capacity |           8 |

All four logical CPU threads were used for parallel processing.

The limited memory capacity made memory management an important engineering consideration throughout the project.

Rather than allocating a global prime bitmap, the implementation used segmented sieving and bounded caching.

---

### 4. Installation and Execution

Please refer to the [docs](./docs/) for detailed instructions on how to install and execute the computation.

## 5. Mathematical Reductions and Candidate Filtering

### Admissible Q values

A prime `q` is admissible if and only if:

```text
is_prime(q)
is_prime(6*q - 1)
is_prime(6*q + 1)
```

The admissible values are precomputed and stored in sorted order.

This avoids repeating the same three primality conditions for every integer `n`.

### Modular filtering

For primes greater than 3:

```text
q ≡ 1 or 5 (mod 6)
```

Additional restrictions follow from considering the conditions modulo 5.

For `q > 5`, admissible values must satisfy:

```text
q mod 5 = 2 or 3
```

These modular restrictions reduce the number of candidates that need to be tested.

The precomputation is performed once for the required range of `q`.

For the separate campaign up to 100,000, the recorded admissible Q-list contained 230 values up to the maximum required bound of 49,999.

---

## 6. Prime Lookup Architecture

### The memory problem

For a campaign ending at `N`, the largest integer requiring primality evaluation can reach:

```text
3N + 1
```

At:

```text
N = 100,000,000,000
```

this becomes:

```text
300,000,000,001
```

A global bitmap covering the entire interval would be impractical on a machine with 8 GB of RAM.

The implementation therefore uses a segmented prime lookup architecture called `PrimeOracle`.

### PrimeOracle

The implementation combines:

- A small base-prime list up to the square root of the maximum required integer.
- A segmented sieve operating on fixed-size windows.
- On-demand primality queries.
- A bounded cache of previously generated windows.
- LRU eviction when the cache reaches capacity.

The default configuration uses:

```text
segment_size    = 2^20
cache_capacity  = 8
```

The base-prime sieve is generated once.

Primality queries then operate on the relevant segment rather than requiring a global bitmap.

This keeps the working memory bounded and makes the computation practical on hardware with limited RAM.

The standalone `PrimeBitmap` component remains available and is used internally for the smaller base-prime sieve.

---

## 7. Main Verification Algorithm

For each integer `n`, the computation determines the parity and corresponding upper bound for `q`.

### Even integers

For even `n`:

```text
n = p + q
p = n - q
q < n/2
```

### Odd integers

For odd `n`:

```text
n = p + 2q
p = n - 2q
q < n/3
```

The algorithm iterates through the sorted admissible Q-list.

For each candidate:

1. Check whether `q` is within the relevant bound.
2. Compute the corresponding value of `p`.
3. Query the prime oracle to determine whether `p` is prime.
4. If `p` is prime, record a valid representation.
5. Otherwise, continue to the next admissible `q`.

If no valid representation is found, the integer is recorded as a zero-solution result.

The campaign records the result for every processed integer using a canonical result record.

---

## 8. Parallelization and Persistence

### Parallel processing

The implementation uses Rayon to distribute the computation across four logical CPU threads.

The full numerical range is divided into independent, contiguous chunks.

Each chunk is processed separately, avoiding overlap between workers.

### Deterministic hashing

Each completed chunk produces a SHA-256 hash.

The hash is computed from its canonical result records in ascending numerical order.

The global result root is then calculated by chaining the chunk hashes in ascending numerical order.

This makes the final root independent of:

- Thread scheduling.
- Worker completion order.
- The order in which parallel tasks finish.

The same canonical input records and hashing procedure should produce the same result root.

### SQLite persistence

Completed chunks are stored in a SQLite campaign database.

The database uses:

- Write-Ahead Logging (WAL).
- `synchronous=FULL`.
- Atomic chunk transactions.
- Persistent campaign progress.
- Recovery of previously completed chunks.

This allows computation to resume after an interruption without restarting the entire campaign.

The execution experienced an early failure and a later recovery event. The persistent database allowed the campaign to continue and ultimately complete.

---

## 9. Computational Results

### Main campaign: 18,681 to 100 billion

The following results were reported by the completed campaign.

| Metric                  |                                                             Result |
| ----------------------- | -----------------------------------------------------------------: |
| Status                  |                                                          COMPLETED |
| Start                   |                                                             18,681 |
| End                     |                                                    100,000,000,000 |
| Worker threads          |                                                                  4 |
| Chunk size              |                                                          1,000,000 |
| Block size              |                                                        100,000,000 |
| Q-candidate evaluations |                                                  1,493,829,701,180 |
| Elapsed time            |                                                  1,128,566 seconds |
| Elapsed time in days    |                                                             ~13.06 |
| Result root             | `4166f1e3482d6a74995ff79756e11a33ed49cdd6b1265bb90b5677707b45986d` |

### The scale of the computation

One of the most significant figures from this investigation is the total number of Q-candidate evaluations:

**1,493,829,701,180**

That is approximately:

**1.49 trillion candidate evaluations.**

This is the cumulative number of admissible-Q candidate iterations reported by the campaign, not the number of distinct integers processed or necessarily the number of individual primality tests.

The main campaign covered approximately 100 billion integers, with candidate evaluations performed repeatedly for the admissible values of `q` relevant to each integer.

This illustrates the scale of the computational workload.

The result was obtained on a laptop with 8 GB of installed RAM and an Intel Core i5-6300U mobile processor.

### Separate small-range campaign

A second campaign independently processed the interval from 1 to 100,000.

| Metric                   |                                                             Result |
| ------------------------ | -----------------------------------------------------------------: |
| Status                   |                                                          COMPLETED |
| Range                    |                                                        1 – 100,000 |
| Processed integers       |                                                            100,000 |
| Q candidates             |                                                            630,859 |
| Q-list hash              | `bdae99ec86f8c6d97e69730537279033ae37bfdeecaf4ac979af373baa92c5d5` |
| Reported counterexamples |                                                               None |

This campaign provides an additional recorded computation over the initial interval.
The database was retained from the original campaign execution
and copied from the machine on which the computation was performed.

It is provided to facilitate inspection, reproducibility,
and independent verification of the computational results.

### Verifying the campaign database

Run this audit to check that the campaign completed, contains no
in-conjecture-domain counterexamples, and has no missing, overlapping, or
malformed chunks or blocks:

```bash
sqlite3 -header -column db/campaign-2026-10-22-175549.db "
WITH
cfg AS (
  SELECT range_start, range_end, chunk_size, block_size
  FROM campaign
  WHERE id = 1
),
issues(issue) AS (
  SELECT 'campaign is not COMPLETED'
  FROM campaign
  WHERE id = 1 AND status <> 'COMPLETED'
  UNION ALL
  SELECT 'campaign current_position does not reach range_end + 1'
  FROM campaign
  WHERE id = 1 AND current_position <> range_end + 1
  UNION ALL
  SELECT 'in-conjecture-domain counterexamples: ' || group_concat(n, ', ')
  FROM zero_solutions
  WHERE in_conjecture_domain = 1
  HAVING COUNT(*) > 0
  UNION ALL
  SELECT 'unexpected or malformed chunk starting at ' || c.chunk_start
  FROM chunks c
  CROSS JOIN cfg
  WHERE c.chunk_start < cfg.range_start
     OR c.chunk_end > cfg.range_end
     OR c.chunk_end - c.chunk_start + 1 <>
        MIN(cfg.chunk_size, cfg.range_end - c.chunk_start + 1)
  UNION ALL
  SELECT 'overlapping or gapped chunks near ' || c.chunk_start
  FROM (
    SELECT chunk_start, chunk_end,
           lag(chunk_end) OVER (ORDER BY chunk_start) AS previous_end
    FROM chunks
  ) c
  CROSS JOIN cfg
  WHERE (c.previous_end IS NULL AND c.chunk_start <> cfg.range_start)
     OR (c.previous_end IS NOT NULL AND c.chunk_start <> c.previous_end + 1)
  UNION ALL
  SELECT 'unexpected or malformed block starting at ' || b.block_start
  FROM blocks b
  CROSS JOIN cfg
  WHERE b.block_start < cfg.range_start
     OR b.block_end > cfg.range_end
     OR b.block_end - b.block_start + 1 <>
        MIN(cfg.block_size, cfg.range_end - b.block_start + 1)
  UNION ALL
  SELECT 'no chunks recorded'
  FROM cfg
  WHERE NOT EXISTS (SELECT 1 FROM chunks)
  UNION ALL
  SELECT 'no blocks recorded'
  FROM cfg
  WHERE NOT EXISTS (SELECT 1 FROM blocks)
)
SELECT issue FROM issues
UNION ALL
SELECT 'OK: no counterexamples and no missing, overlapping, or malformed chunks/blocks'
WHERE NOT EXISTS (SELECT 1 FROM issues);
"
```

The command should print only the `OK:` line. Any other output identifies a
specific database inconsistency.

---

## 10. Execution History

The main campaign was monitored through periodic progress reports.

The following milestones were recorded.

| Date        |          Progress | Completed through |
| ----------- | ----------------: | ----------------: |
| 05 Oct 2026 | Initial execution |         1,018,680 |
| 06 Oct 2026 |             27.4% |    27,402,018,680 |
| 07 Oct 2026 |             39.0% |    39,004,018,680 |
| 08 Oct 2026 |             47.9% |    47,901,018,680 |
| 10 Oct 2026 |             55.4% |    55,404,018,680 |
| 11 Oct 2026 |             62.0% |    62,001,018,680 |
| 12 Oct 2026 |             67.9% |    67,901,018,680 |
| 13 Oct 2026 |             73.4% |    73,401,018,680 |
| 14 Oct 2026 |             78.5% |    78,504,018,680 |
| 15 Oct 2026 |             83.3% |    83,302,018,680 |
| 16 Oct 2026 |             87.8% |    87,804,018,680 |
| 17 Oct 2026 |             92.1% |    92,103,018,680 |
| 18 Oct 2026 |             96.2% |    96,203,018,680 |
| 19 Oct 2026 |              100% |   100,000,000,000 |

The campaign initially encountered an execution failure.

It subsequently continued through persistent-state recovery and reached the configured endpoint.

### Increasing computational workload

The execution logs also reveal that the number of Q-candidate evaluations per 100-million-sized block generally increased as the computation progressed.

This is an important practical observation.

Although the numerical chunks had the same configured size, their computational costs were not constant.

The admissible Q-list grew with the relevant upper bound, and more candidate iterations were required in later portions of the campaign.

Consequently, processing later blocks became increasingly expensive.

This was one of the principal reasons for stopping the investigation at 100 billion rather than extending it further.

The decision was based on the practical cost of continuing the computation on the available hardware.

---

## 11. Final Outcome

The main campaign successfully completed its configured range:

```text
18,681 -> 100,000,000,000
```

The separate small-range campaign completed:

```text
1 -> 100,000
```

The main campaign reported:

```text
Q candidates:
1,493,829,701,180

Elapsed time:
1,128,566 seconds

Result root:
4166f1e3482d6a74995ff79756e11a33ed49cdd6b1265bb90b5677707b45986d
```

The result represents a substantial computational investigation of OEIS A220091.

Approximately 1.49 trillion Q-candidate evaluations were carried out over the main campaign, using a machine with only 8 GB of RAM.

The computation reached 100 billion, which was the configured endpoint.

No extension beyond this point is currently planned.

### Mathematical qualification

This work is a computational investigation, not a mathematical proof.

A completed computation provides evidence for the conjecture only over the integers actually processed.

The reported result root provides a deterministic identifier for the campaign results, but a hash alone does not independently establish mathematical correctness.

Independent verification requires access to the implementation, campaign database, canonical result records, and hashing procedure.

I also created a separate small-range campaign covering 1 through 100,000 via naive python code to verify the admissible Q-list and its SHA-256 hash of the canonical result records.

---

## 12. Project Status

**COMPLETED — 100 BILLION**

After approximately 13 days of computation, the main campaign reached its configured endpoint.

The investigation was performed on a Lenovo ThinkPad X260 with:

- Intel Core i5-6300U.
- 8 GB RAM.
- Four logical CPU threads.
- Ubuntu 26.04.1 LTS.

The computation accumulated approximately 1.49 trillion Q-candidate evaluations.

I decided to stop at 100 billion because the workload was becoming increasingly time-consuming as the computation progressed.

Extending the range further would require a substantial additional investment of time, particularly on the available hardware.

The objective was to investigate the conjecture as far as reasonably practical, preserve the computational results, and document the methodology.

For now, **100 billion is the final endpoint of this investigation.**

---

## 13. References

- [OEIS A220091](https://oeis.org/A220091) — Original sequence and conjecture.
- [Zhi-Wei Sun — arXiv:1211.1588](https://arxiv.org/abs/1211.1588) — Research background.
- [Goldbach's conjecture](https://en.wikipedia.org/wiki/Goldbach%27s_conjecture) — Related classical prime-representation problem.

---

## 14. Reproducibility and Verification

The following artifacts should be retained or made available for independent verification:

- Source code and exact software revision.
- Campaign SQLite database.
- Admissible Q-list or its reproducible generation procedure.
- Q-list SHA-256 hash.
- Canonical chunk result records.
- Individual chunk SHA-256 hashes.
- Final global result root.
- Zero-solution records or an independently verified zero-solution count.
- Campaign configuration and execution logs.

The campaign result root is:

```text
4166f1e3482d6a74995ff79756e11a33ed49cdd6b1265bb90b5677707b45986d
```

The intention is to make the investigation reproducible and open to independent examination.
