# OEIS A220091 — Mathematical Algorithm & Computational Strategy

## 1. Sequence Definition

OEIS sequence **A220091** defines $a(n)$ as the number of representations of $n$ in the form:

$$n = p + q + (n \bmod 2)q$$

subject to the constraints:
1. $p > q$
2. $p$ is prime
3. $q$ is prime
4. $6q - 1$ is prime
5. $6q + 1$ is prime

### Parity Split

- **Even $n$ ($n \equiv 0 \pmod 2$):**
  $$n = p + q \quad\text{with}\quad p > q \implies q < \frac{n}{2}, \quad p = n - q$$

- **Odd $n$ ($n \equiv 1 \pmod 2$):**
  $$n = p + 2q \quad\text{with}\quad p > q \implies q < \frac{n}{3}, \quad p = n - 2q$$

### Conjecture Domain

The computational target is to verify the existence conjecture $a(n) > 0$ on the conjectured domains:
- **Even $n$:** $n \ge 8\,070$
- **Odd $n$:** $n \ge 18\,680$

Values below these thresholds are known boundary exceptions or small-value absences ($n \in \{11329, 13429, 13829, 18379, 18679\}$) and are reported outside the conjecture domain.

---

## 2. Mathematical Reductions & Q-Filter

A prime $q$ is admissible if and only if:
1. $q$ is prime.
2. $6q - 1$ is prime.
3. $6q + 1$ is prime.

### Properties of Admissible $q$

- For $q = 2$: $6(2) - 1 = 11$ (prime), $6(2) + 1 = 13$ (prime). Valid admissible $q$.
- For $q = 3$: $6(3) - 1 = 17$ (prime), $6(3) + 1 = 19$ (prime). Valid admissible $q$.
- For $q > 3$:
  - Primes must satisfy $q \equiv 1$ or $5 \pmod 6$.
  - Modulo 5 filtering:
    - If $q \equiv 1 \pmod 5$, then $6q - 1 = 6(1) - 1 = 5$, requiring $6q - 1 = 5 \implies q = 1$ (not prime). Hence $q \not\equiv 1 \pmod 5$ for $q > 1$.
    - If $q \equiv 4 \pmod 5$, then $6q + 1 = 6(4) + 1 = 25 \equiv 0 \pmod 5$ (composite). Hence $q \not\equiv 4 \pmod 5$.
    - Therefore, for $q > 5$, $q \bmod 5 \in \{2, 3\}$.

### Global Q-List Precomputation

The Q-list is computed once up to $\max_q(N) = \lfloor (N - 1) / 2 \rfloor$. For $N = 10^5$, $\max_q = 49\,999$, yielding exactly 230 admissible $q$ values.

---

## 3. Prime Lookup: Memory-Disciplined Bit Sieve

The verification requires primality testing for:
- $q \le \max_q(N) \approx N / 2$
- $6q + 1 \le 3N + 1$
- $p \le N - q < N$

The maximum integer tested for primality across the entire computation up to $N$ is $3N + 1$.

### Structure: `PrimeOracle` (segmented, bounded-memory)

A single global `PrimeBitmap` covering $0..3N+1$ requires $O(N)$ memory
(`~7.46 GB` at $N \approx 19.9 \times 10^9$, which caused swap thrashing on an
8 GiB-class host). `PrimeOracle` replaces this with a segmented sieve:

- A small base-prime list up to $\sqrt{3N+1}$ is sieved once with
  `PrimeBitmap` (`~244,000` at $N \approx 19.9 \times 10^9$, a few hundred KB).
- Primality queries are answered by sieving a fixed-size window (`segment_size`
  integers, default `2^20`) containing the queried value, using the base
  primes, and caching a bounded number of windows (`cache_capacity`, default
  8) with LRU eviction.
- Resident memory is therefore `O(sqrt(3N+1) + segment_size * cache_capacity)`
  — independent of $N$ in practice (base primes grow only as $\sqrt{N}$), a
  few MB regardless of how large $N$ grows.
- `PrimeBitmap` itself is unchanged and still used internally for the small
  base-prime sieve, and remains available/tested as a standalone component.

---

## 4. Main Verification Loop

For each $n$ in the chunk interval:
1. Determine parity and maximum admissible $q$ bound:
   - Even $n$: $q_{\max} = \lfloor (n - 1) / 2 \rfloor$
   - Odd $n$: $q_{\max} = \lfloor (n - 1) / 3 \rfloor$
2. Iterate through sorted $q \in Q$ where $q \le q_{\max}$:
   - Increment candidate test counter.
   - Compute $p = n - q$ (even) or $p = n - 2q$ (odd).
   - If `primes.is_prime(p)`:
     - Found representation $(p, q)$.
     - Record 33-byte canonical result record with `status = 1`.
     - Advance to next $n$.
3. If no $q \le q_{\max}$ yields prime $p$:
   - Record canonical result record with `status = 0`.
   - Add $n$ to chunk zero-solution list.

---

## 5. Parallelization & Persistence

- **Parallelization:** Rayon parallel iterator splits the range into independent, non-overlapping contiguous chunks of size `chunk_size`.
- **Determinism:** Each chunk computes its canonical SHA-256 result hash in ascending $n$ order.
- **Result Root:** The global result root is computed by chaining chunk SHA-256 hashes in ascending numerical order, invariant to thread execution order or thread count.
- **Durability:** Each completed chunk is committed to the SQLite campaign database in a single atomic transaction (WAL, `synchronous=FULL`); see `CAMPAIGN.md`.
