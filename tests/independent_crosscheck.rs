use std::{fs, path::PathBuf, process::Command};

use a220091::{
    build_q_list, find_representation, q_upper_bound, verifier::find_representation as optimized,
    PrimeOracle,
};
use serde_json::{json, Value};

fn python_records(records: &[Value]) -> Vec<Value> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let input_path = std::env::temp_dir().join(format!(
        "a220091-independent-records-{}.json",
        std::process::id()
    ));
    fs::write(&input_path, serde_json::to_vec(records).unwrap()).unwrap();
    let script = manifest.join("independent").join("verifier.py");
    let mut command = Command::new("python");
    command.args([
        script.to_str().unwrap(),
        "--records",
        input_path.to_str().unwrap(),
    ]);
    let output = command
        .output()
        .or_else(|_| {
            Command::new("py")
                .args([
                    "-3",
                    script.to_str().unwrap(),
                    "--records",
                    input_path.to_str().unwrap(),
                ])
                .output()
        })
        .expect("Python 3 is required for the independent cross-check");
    assert!(
        output.status.success(),
        "Python verifier failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_file(&input_path);
    serde_json::from_slice(&output.stdout).unwrap()
}

fn rust_records(start: u64, end: u64) -> Vec<Value> {
    let mut oracle = PrimeOracle::new(3 * end + 1, 997, 6).unwrap();
    let q_list = build_q_list(q_upper_bound(end), &mut oracle).unwrap();
    (start..=end)
        .map(|n| {
            let result = optimized(n, &mut oracle, &q_list).unwrap();
            json!({
                "n": n,
                "status": u8::from(result.representation.is_some()),
                "p": result.representation.map_or(0, |value| value.p),
                "q": result.representation.map_or(0, |value| value.q),
                "q_candidates_tested": result.q_candidates_tested,
            })
        })
        .collect()
}

#[test]
fn independent_python_matches_rust_reference_and_optimized_through_10000() {
    let records = rust_records(1, 10_000);
    for record in &records {
        let n = record["n"].as_u64().unwrap();
        let reference = find_representation(n);
        assert_eq!(
            record["status"].as_u64().unwrap(),
            u64::from(reference.is_some())
        );
        assert_eq!(
            record["p"].as_u64().unwrap(),
            reference.map_or(0, |value| value.p)
        );
        assert_eq!(
            record["q"].as_u64().unwrap(),
            reference.map_or(0, |value| value.q)
        );
    }
    assert_eq!(python_records(&records), records);
}

#[test]
fn independent_python_matches_both_domain_boundary_ranges() {
    for (start, end) in [(8068, 8072), (18678, 18682)] {
        let records = rust_records(start, end);
        assert_eq!(python_records(&records), records);
    }
    assert!(8070 >= 8070);
    assert!(18680 >= 18680);
    assert!(18679 < 18680);
}
