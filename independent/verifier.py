#!/usr/bin/env python3
"""Independent, intentionally simple verifier for OEIS A220091."""

from __future__ import annotations

import argparse
import json
import sys
from typing import Any

EVEN_CONJECTURE_MIN = 8070
ODD_CONJECTURE_MIN = 18680


def is_prime(value: int) -> bool:
    """Independent trial-division primality test for individual values."""
    if value < 2:
        return False
    if value == 2:
        return True
    if value % 2 == 0:
        return False
    divisor = 3
    while divisor * divisor <= value:
        if value % divisor == 0:
            return False
        divisor += 2
    return True


def simple_sieve(limit: int) -> bytearray:
    """Build a separate byte-per-value sieve for small range validation."""
    flags = bytearray(b"\x01") * (limit + 1)
    if limit >= 0:
        flags[0] = 0
    if limit >= 1:
        flags[1] = 0
    candidate = 2
    while candidate * candidate <= limit:
        if flags[candidate]:
            first = candidate * candidate
            flags[first : limit + 1 : candidate] = b"\x00" * (((limit - first) // candidate) + 1)
        candidate += 1
    return flags


def q_upper_bound(n: int) -> int:
    if n % 2 == 0:
        return max(0, (n - 1) // 2)
    return max(0, (n - 1) // 3)


def in_conjecture_domain(n: int) -> bool:
    if n % 2 == 0:
        return n >= EVEN_CONJECTURE_MIN
    return n >= ODD_CONJECTURE_MIN


def validate_representation(n: int, p: int, q: int) -> bool:
    """Independently validate every A220091 condition using trial division."""
    if not is_prime(q):
        return False
    if not is_prime(6 * q - 1):
        return False
    if not is_prime(6 * q + 1):
        return False
    if not is_prime(p):
        return False
    if p <= q:
        return False
    return n == p + q + (n % 2) * q


def verify_n(n: int, flags: bytearray | None = None) -> dict[str, int]:
    """Enumerate all integer q candidates independently and stop at first hit."""
    if flags is None:
        flags = simple_sieve(max(3 * n + 1, n))

    tested = 0
    for q in range(2, q_upper_bound(n) + 1):
        if not (flags[q] and flags[6 * q - 1] and flags[6 * q + 1]):
            continue
        tested += 1
        p = n - q if n % 2 == 0 else n - 2 * q
        if p > q and flags[p]:
            return {
                "n": n,
                "status": 1,
                "p": p,
                "q": q,
                "q_candidates_tested": tested,
            }

    return {"n": n, "status": 0, "p": 0, "q": 0, "q_candidates_tested": tested}


def verify_record(record: dict[str, Any], flags: bytearray | None = None) -> bool:
    """Validate a Rust-style canonical result record independently."""
    required = {"n", "status", "p", "q", "q_candidates_tested"}
    if not required.issubset(record):
        return False
    n = int(record["n"])
    status = int(record["status"])
    p = int(record["p"])
    q = int(record["q"])
    tested = int(record["q_candidates_tested"])
    if status not in (0, 1) or tested < 0:
        return False

    expected = verify_n(n, flags)
    if expected["q_candidates_tested"] != tested or expected["status"] != status:
        return False
    if status == 0:
        return p == 0 and q == 0
    return p == expected["p"] and q == expected["q"] and validate_representation(n, p, q)


def verify_range(start: int, end: int) -> list[dict[str, int]]:
    if start > end:
        raise ValueError("start must not exceed end")
    flags = simple_sieve(max(3 * end + 1, end))
    return [verify_n(n, flags) for n in range(start, end + 1)]


def run_self_test() -> None:
    assert validate_representation(31, 17, 7)
    assert not validate_representation(31, 19, 7)
    assert not validate_representation(31, 17, 13)
    assert not validate_representation(31, 17, 19)
    assert not validate_representation(31, 7, 7)
    assert not validate_representation(32, 17, 7)
    assert verify_n(91)["status"] == 0
    assert not in_conjecture_domain(8068)
    assert in_conjecture_domain(8070)
    assert not in_conjecture_domain(18679)
    assert in_conjecture_domain(18681)
    assert ODD_CONJECTURE_MIN == 18680
    assert verify_record(verify_n(31))
    assert verify_record(verify_n(91))
    print("independent verifier self-test: PASS")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--verify-n", type=int)
    parser.add_argument("--range", nargs=2, type=int, metavar=("START", "END"))
    parser.add_argument("--records")
    parser.add_argument("--verify-record", type=json.loads)
    args = parser.parse_args()

    if args.self_test:
        run_self_test()
        return 0
    if args.verify_n is not None:
        print(json.dumps(verify_n(args.verify_n), sort_keys=True))
        return 0
    if args.range is not None:
        print(json.dumps(verify_range(args.range[0], args.range[1]), separators=(",", ":")))
        return 0
    if args.records is not None:
        with open(args.records, encoding="utf-8") as input_file:
            records = json.load(input_file)
        maximum_n = max((int(record["n"]) for record in records), default=0)
        flags = simple_sieve(max(3 * maximum_n + 1, maximum_n))
        if not all(verify_record(record, flags) for record in records):
            return 1
        print(json.dumps([verify_n(int(record["n"]), flags) for record in records], separators=(",", ":")))
        return 0
    if args.verify_record is not None:
        if not verify_record(args.verify_record):
            return 1
        print("valid")
        return 0

    parser.error("choose --self-test, --verify-n, --range, --records, or --verify-record")
    return 2


if __name__ == "__main__":
    sys.exit(main())
