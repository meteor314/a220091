use a220091::verifier::{find_representation as optimized_find_representation, VerificationError};
use a220091::{
    admissible_q, build_q_list, count_representations, find_representation, is_admissible_q,
    is_prime, q_upper_bound, representations, verify_range_parallel, ParallelVerificationError,
    PrimeBitmap, PrimeOracle, QFilterError, Representation,
};

#[test]
fn primality_handles_boundaries_and_small_values() {
    assert!(!is_prime(0));
    assert!(!is_prime(1));
    assert!(is_prime(2));
    assert!(is_prime(3));
    assert!(!is_prime(4));
    assert!(is_prime(97));
    assert!(!is_prime(99));
}

#[test]
fn q_bounds_follow_n_parity() {
    assert_eq!(q_upper_bound(0), 0);
    assert_eq!(q_upper_bound(1), 0);
    assert_eq!(q_upper_bound(10), 4);
    assert_eq!(q_upper_bound(11), 3);
}

#[test]
fn admissible_q_is_independent_of_n() {
    assert_eq!(admissible_q(20), vec![2, 3, 5, 7, 17]);
    assert_eq!(admissible_q(7), vec![2, 3, 5, 7]);
}

#[test]
fn admissible_q_rejects_arithmetic_overflow() {
    assert!(!is_admissible_q(u64::MAX));
}

#[test]
fn known_even_representation_is_valid() {
    assert_eq!(
        find_representation(32),
        Some(Representation { n: 32, p: 29, q: 3 })
    );
}

#[test]
fn known_odd_representation_is_valid() {
    assert_eq!(
        find_representation(31),
        Some(Representation { n: 31, p: 17, q: 7 })
    );
}

#[test]
fn representations_enforce_p_greater_than_q_and_parity_formula() {
    for n in 1..=200 {
        for representation in representations(n) {
            assert!(representation.p > representation.q);
            assert!(is_prime(representation.p));
            assert!(is_prime(representation.q));
            assert!(is_prime(6 * representation.q - 1));
            assert!(is_prime(6 * representation.q + 1));
            assert_eq!(
                n,
                representation.p + representation.q + (n % 2) * representation.q
            );
            assert!(representation.q <= q_upper_bound(n));
        }
    }
}

#[test]
fn count_mode_counts_all_representations() {
    assert_eq!(count_representations(31), 1);
    assert_eq!(count_representations(32), 1);
    assert_eq!(count_representations(1), 0);
}

#[test]
fn existence_mode_returns_the_first_q() {
    let all = representations(101);
    assert!(!all.is_empty());
    assert_eq!(find_representation(101), all.first().copied());
}

#[test]
fn bitmap_handles_zero_one_and_small_primes() {
    let bitmap = PrimeBitmap::from_limit(17).unwrap();

    assert!(!bitmap.is_prime(0));
    assert!(!bitmap.is_prime(1));
    assert!(bitmap.is_prime(2));
    assert!(bitmap.is_prime(3));
    assert!(bitmap.is_prime(5));
    assert!(bitmap.is_prime(17));
}

#[test]
fn bitmap_handles_small_composites_and_prime_boundaries() {
    let bitmap = PrimeBitmap::from_limit(17).unwrap();

    assert!(!bitmap.is_prime(4));
    assert!(bitmap.is_prime(5));
    assert!(!bitmap.is_prime(6));
    assert!(bitmap.is_prime(7));
    assert!(!bitmap.is_prime(8));
    assert!(!bitmap.is_prime(9));
    assert!(!bitmap.is_prime(10));
    assert!(bitmap.is_prime(11));
    assert!(!bitmap.is_prime(12));
    assert!(bitmap.is_prime(13));
    assert!(!bitmap.is_prime(14));
    assert!(!bitmap.is_prime(15));
    assert!(!bitmap.is_prime(16));
}

#[test]
fn bitmap_is_explicitly_bounded() {
    let bitmap = PrimeBitmap::from_limit(17).unwrap();

    assert_eq!(bitmap.limit(), 17);
    assert!(!bitmap.is_prime(18));
    assert!(!bitmap.is_prime(u64::MAX));
}

#[test]
fn bitmap_matches_reference_primality_on_a_substantial_small_range() {
    let limit = 10_000;
    let bitmap = PrimeBitmap::from_limit(limit).unwrap();

    for value in 0..=limit {
        assert_eq!(bitmap.is_prime(value), is_prime(value), "value={value}");
    }
}

#[test]
fn q_list_contains_the_first_admissible_values_and_q_two() {
    let mut oracle = PrimeOracle::new(121, 32, 4).unwrap();
    let q_list = build_q_list(20, &mut oracle).unwrap();

    assert_eq!(q_list.as_slice(), &[2, 3, 5, 7, 17]);
    assert_eq!(q_list.as_slice().first(), Some(&2));
}

#[test]
fn q_list_is_strictly_sorted() {
    let mut oracle = PrimeOracle::new(301, 32, 4).unwrap();
    let q_list = build_q_list(50, &mut oracle).unwrap();

    assert!(q_list
        .as_slice()
        .windows(2)
        .all(|window| window[0] < window[1]));
}

#[test]
fn every_q_in_the_list_matches_the_reference_definition() {
    let mut oracle = PrimeOracle::new(601, 32, 4).unwrap();
    let q_list = build_q_list(100, &mut oracle).unwrap();

    assert!(q_list.as_slice().iter().all(|&q| is_admissible_q(q)));
}

#[test]
fn q_list_matches_simple_reference_construction() {
    let max_q = 100;
    let mut oracle = PrimeOracle::new(6 * max_q + 1, 32, 4).unwrap();
    let q_list = build_q_list(max_q, &mut oracle).unwrap();
    let reference: Vec<u64> = (2..=max_q).filter(|&q| is_admissible_q(q)).collect();

    let optimized: Vec<u64> = q_list.as_slice().to_vec();
    assert_eq!(optimized, reference);
}

#[test]
fn q_list_respects_maximum_and_borrowed_prefix_bounds() {
    let mut oracle = PrimeOracle::new(301, 32, 4).unwrap();
    let q_list = build_q_list(20, &mut oracle).unwrap();

    assert_eq!(q_list.as_slice(), &[2, 3, 5, 7, 17]);
    assert_eq!(q_list.prefix_up_to(2), &[2]);
    assert_eq!(q_list.prefix_up_to(6), &[2, 3, 5]);
    assert_eq!(q_list.prefix_up_to(1), &[] as &[u64]);
}

#[test]
fn q_list_checks_bitmap_boundary_and_arithmetic_errors() {
    let mut insufficient_oracle = PrimeOracle::new(12, 32, 4).unwrap();
    assert_eq!(
        build_q_list(2, &mut insufficient_oracle),
        Err(QFilterError::BitmapLimitTooSmall {
            required: 13,
            available: 12,
        })
    );

    let mut oracle = PrimeOracle::new(0, 32, 4).unwrap();
    assert_eq!(
        build_q_list(u64::MAX, &mut oracle),
        Err(QFilterError::ArithmeticOverflow)
    );
}

#[test]
fn optimized_verifier_finds_known_even_and_odd_representations() {
    let mut oracle = PrimeOracle::new(301, 32, 4).unwrap();
    let q_list = build_q_list(50, &mut oracle).unwrap();

    let even = optimized_find_representation(32, &mut oracle, &q_list).unwrap();
    assert_eq!(
        even.representation,
        Some(Representation { n: 32, p: 29, q: 3 })
    );
    assert_eq!(even.q_candidates_tested, 2);

    let odd = optimized_find_representation(31, &mut oracle, &q_list).unwrap();
    assert_eq!(
        odd.representation,
        Some(Representation { n: 31, p: 17, q: 7 })
    );
    assert_eq!(odd.q_candidates_tested, 4);
}

#[test]
fn optimized_verifier_reports_no_representation_and_candidate_count() {
    let mut oracle = PrimeOracle::new(121, 32, 4).unwrap();
    let q_list = build_q_list(20, &mut oracle).unwrap();
    let result = optimized_find_representation(6, &mut oracle, &q_list).unwrap();

    assert_eq!(result.representation, None);
    assert_eq!(result.q_candidates_tested, 1);
}

#[test]
fn optimized_verifier_rejects_insufficient_q_and_prime_ranges() {
    let mut oracle = PrimeOracle::new(121, 32, 4).unwrap();
    let short_q_list = build_q_list(2, &mut oracle).unwrap();
    assert_eq!(
        optimized_find_representation(31, &mut oracle, &short_q_list),
        Err(VerificationError::QListLimitTooSmall {
            required: 10,
            available: 2,
        })
    );

    let q_list = build_q_list(20, &mut oracle).unwrap();
    let mut short_oracle = PrimeOracle::new(10, 32, 4).unwrap();
    assert_eq!(
        optimized_find_representation(32, &mut short_oracle, &q_list),
        Err(VerificationError::PrimeBitmapLimitTooSmall {
            required: 30,
            available: 10,
        })
    );
}

#[test]
fn optimized_verifier_matches_reference_for_every_n_through_10000() {
    let maximum_n = 10_000;
    let maximum_q = q_upper_bound(maximum_n);
    let mut oracle = PrimeOracle::new(6 * maximum_q + 1, 997, 6).unwrap();
    let q_list = build_q_list(maximum_q, &mut oracle).unwrap();

    for n in 1..=maximum_n {
        let optimized = optimized_find_representation(n, &mut oracle, &q_list)
            .unwrap()
            .representation;
        let reference = find_representation(n);
        assert_eq!(optimized, reference, "n={n}");

        if let Some(representation) = optimized {
            assert!(representation.p > representation.q);
            assert!(is_prime(representation.p));
            assert!(is_prime(representation.q));
            assert!(is_prime(6 * representation.q - 1));
            assert!(is_prime(6 * representation.q + 1));
            assert_eq!(
                n,
                representation.p + representation.q + (n % 2) * representation.q
            );
        }
    }
}

fn parallel_test_inputs() -> (PrimeOracle, a220091::QList) {
    let maximum_n = 10_000;
    let maximum_q = q_upper_bound(maximum_n);
    let mut oracle = PrimeOracle::new(6 * maximum_q + 1, 997, 6).unwrap();
    let q_list = build_q_list(maximum_q, &mut oracle).unwrap();
    (oracle, q_list)
}

#[test]
fn parallel_matches_single_thread_aggregate_over_10000_values() {
    let (oracle, q_list) = parallel_test_inputs();
    let parallel = verify_range_parallel(1, 10_000, &oracle, &q_list, 2, 257).unwrap();

    let mut n_processed = 0;
    let mut total_q_tests = 0;
    let mut zero_solution_n = Vec::new();
    let mut sequential_oracle = oracle.clone();
    for n in 1..=10_000 {
        let result = optimized_find_representation(n, &mut sequential_oracle, &q_list).unwrap();
        n_processed += 1;
        total_q_tests += result.q_candidates_tested;
        if result.representation.is_none() {
            zero_solution_n.push(n);
        }
    }

    assert_eq!(parallel.n_processed, n_processed);
    assert_eq!(parallel.total_q_tests, total_q_tests);
    assert_eq!(parallel.zero_solution_n, zero_solution_n);
}

#[test]
fn parallel_one_through_four_threads_are_identical() {
    let (oracle, q_list) = parallel_test_inputs();
    let baseline = verify_range_parallel(1, 10_000, &oracle, &q_list, 1, 257).unwrap();

    for threads in 2..=4 {
        let result = verify_range_parallel(1, 10_000, &oracle, &q_list, threads, 257).unwrap();
        assert_eq!(result, baseline, "threads={threads}");
    }
}

#[test]
fn parallel_chunks_are_contiguous_without_gaps_or_overlaps() {
    let (oracle, q_list) = parallel_test_inputs();
    let result = verify_range_parallel(7, 23, &oracle, &q_list, 3, 4).unwrap();

    assert_eq!(
        result
            .chunks
            .iter()
            .map(|chunk| (chunk.start, chunk.end))
            .collect::<Vec<_>>(),
        vec![(7, 10), (11, 14), (15, 18), (19, 22), (23, 23)]
    );
    assert_eq!(result.n_processed, 17);
}

#[test]
fn parallel_handles_odd_even_boundaries_and_invalid_inputs() {
    let (oracle, q_list) = parallel_test_inputs();
    let result = verify_range_parallel(30, 35, &oracle, &q_list, 2, 3).unwrap();
    assert_eq!(result.n_processed, 6);

    assert_eq!(
        verify_range_parallel(35, 30, &oracle, &q_list, 2, 3),
        Err(ParallelVerificationError::InvalidRange)
    );
    assert_eq!(
        verify_range_parallel(30, 35, &oracle, &q_list, 0, 3),
        Err(ParallelVerificationError::InvalidThreadCount)
    );
    assert_eq!(
        verify_range_parallel(30, 35, &oracle, &q_list, 2, 0),
        Err(ParallelVerificationError::InvalidChunkSize)
    );
}
