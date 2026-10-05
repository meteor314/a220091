use std::{env, fmt, path::PathBuf, process, sync::Arc, time::Instant};

use a220091::{
    build_q_list,
    hashing::{hash_q_list, hex_digest},
    notifier::{DiscordNotifier, NotificationEvent},
    parallel::verify_range_parallel,
    prime_oracle::{DEFAULT_CACHE_SEGMENTS, DEFAULT_SEGMENT_SIZE},
    verifier::{find_representation, VerificationError},
    PrimeOracle,
};

const DEFAULT_START: u64 = 1;
const DEFAULT_END: u64 = 100_000;
const DEFAULT_THREADS: usize = 1;
const DEFAULT_CHUNK_SIZE: u64 = 1_000_000;
const EVEN_CONJECTURE_MIN: u64 = 8_070;
const ODD_CONJECTURE_MIN: u64 = 18_680;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BenchmarkMode {
    FullScan,
    ConjectureDomain,
}

impl BenchmarkMode {
    fn label(self) -> &'static str {
        match self {
            Self::FullScan => "full-scan",
            Self::ConjectureDomain => "conjecture-domain",
        }
    }
}

#[derive(Debug)]
struct Config {
    start: u64,
    end: u64,
    threads: usize,
    chunk_size: u64,
    mode: BenchmarkMode,
    json_only: bool,
    no_discord: bool,
    segment_size: u64,
    cache_segments: usize,
}

#[derive(Debug)]
struct BenchmarkReport {
    start: u64,
    end: u64,
    mode: BenchmarkMode,
    threads: usize,
    chunk_size: u64,
    q_list_sha256: String,
    prime_limit: u64,
    q_count: usize,
    prime_generation_seconds: f64,
    q_generation_seconds: f64,
    verification_seconds: f64,
    total_seconds: f64,
    n_processed: u64,
    total_q_tests: u64,
    zero_solution_n: Vec<u64>,
    zero_solution_in_domain: Vec<u64>,
    zero_solution_outside_domain: Vec<u64>,
    prime_oracle_bytes: u64,
}

#[derive(Debug)]
enum BenchmarkError {
    Usage(String),
    ArithmeticOverflow,
    PrimeOracle(String),
    QFilter(String),
    Verification(VerificationError),
    Parallel(String),
    Counterexamples(Vec<u64>),
}

impl fmt::Display for BenchmarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => formatter.write_str(message),
            Self::ArithmeticOverflow => formatter.write_str("benchmark arithmetic overflow"),
            Self::PrimeOracle(message) => {
                write!(formatter, "prime oracle construction failed: {message}")
            }
            Self::QFilter(message) => write!(formatter, "Q-list construction failed: {message}"),
            Self::Verification(error) => write!(formatter, "verification failed: {error}"),
            Self::Parallel(error) => write!(formatter, "parallel verification failed: {error}"),
            Self::Counterexamples(values) => {
                write!(
                    formatter,
                    "in-domain counterexamples found for n values: {values:?}"
                )
            }
        }
    }
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(error) => {
            eprintln!("benchmark failed: {error}");
            process::exit(1);
        }
    }
}

fn run() -> Result<(), BenchmarkError> {
    let config = parse_args()?;
    let notification_state = PathBuf::from("notifications").join("state.json");
    let notifier = match DiscordNotifier::from_env(config.no_discord, notification_state) {
        Ok(value) => value.map(Arc::new),
        Err(_) => {
            eprintln!("Discord configuration unavailable; computation continues");
            None
        }
    };
    if notifier.is_none() {
        eprintln!("Discord disabled");
    }
    if let Some(notifier) = &notifier {
        let start_message = format!(
            "Status: RUNNING\nRange: {} -> {}\nThreads: {}\nChunk size: {}",
            config.start, config.end, config.threads, config.chunk_size
        );
        if notifier
            .notify(NotificationEvent::Start, &start_message)
            .is_err()
        {
            eprintln!("Discord notification failed; computation continues");
        }
    }
    let report = match measure(&config) {
        Ok(report) => report,
        Err(error) => {
            if let Some(notifier) = &notifier {
                let message = format!("Status: ERROR\nDetails: {error}");
                if notifier
                    .notify(NotificationEvent::ImportantError, &message)
                    .is_err()
                {
                    eprintln!("Discord notification failed; computation continues");
                }
            }
            return Err(error);
        }
    };

    if !config.json_only {
        print_human_summary(&report);
    }
    println!("{}", report_json(&report));

    if counterexamples_for_mode(&report.zero_solution_n, report.mode).is_empty() {
        if let Some(notifier) = &notifier {
            let message = format!(
                "Status: COMPLETED\nRange: {} -> {}\nProcessed: {}\nQ candidates: {}\nQ hash: {}\nNo counterexample found in the verified interval.",
                report.start,
                report.end,
                report.n_processed,
                report.total_q_tests,
                report.q_list_sha256,
            );
            if notifier
                .notify(NotificationEvent::Completed, &message)
                .is_err()
            {
                eprintln!("Discord notification failed; computation continues");
            }
        }
        Ok(())
    } else {
        if let Some(notifier) = &notifier {
            let message = format!(
                "Status: ERROR\nIn-domain counterexamples: {:?}",
                counterexamples_for_mode(&report.zero_solution_n, report.mode)
            );
            if notifier
                .notify(NotificationEvent::ImportantError, &message)
                .is_err()
            {
                eprintln!("Discord notification failed; computation continues");
            }
        }
        Err(BenchmarkError::Counterexamples(counterexamples_for_mode(
            &report.zero_solution_n,
            report.mode,
        )))
    }
}

fn parse_args() -> Result<Config, BenchmarkError> {
    let mut start = DEFAULT_START;
    let mut end = DEFAULT_END;
    let mut threads = DEFAULT_THREADS;
    let mut chunk_size = DEFAULT_CHUNK_SIZE;
    let mut mode = BenchmarkMode::ConjectureDomain;
    let mut json_only = false;
    let mut no_discord = false;
    let mut segment_size = DEFAULT_SEGMENT_SIZE;
    let mut cache_segments = DEFAULT_CACHE_SEGMENTS;
    let mut arguments = env::args().skip(1);

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--start" => start = parse_value("--start", arguments.next())?,
            "--end" => end = parse_value("--end", arguments.next())?,
            "--threads" => threads = parse_usize("--threads", arguments.next())?,
            "--chunk-size" => chunk_size = parse_value("--chunk-size", arguments.next())?,
            "--mode" => mode = parse_mode(arguments.next())?,
            "--json" => json_only = true,
            "--no-discord" => no_discord = true,
            "--segment-size" => segment_size = parse_value("--segment-size", arguments.next())?,
            "--cache-segments" => {
                cache_segments = parse_usize("--cache-segments", arguments.next())?
            }
            "--help" | "-h" => {
                println!(
                    "Usage: cargo run --release --bin benchmark -- [--start N] [--end N] [--threads N] [--chunk-size N]                     [--mode full-scan|conjecture-domain] [--json] [--segment-size N] [--cache-segments N]"
                );
                println!(
                                        "In-memory benchmark only; persistent campaigns use the `campaign` binary (SQLite). Both modes process every n in the requested interval. The conjecture-domain mode is the normal A220091 verification scope; values outside the parity-specific domain are reported but never treated as counterexamples."
                );
                process::exit(0);
            }
            unknown => {
                return Err(BenchmarkError::Usage(format!(
                    "unknown argument {unknown}; use --help for usage"
                )));
            }
        }
    }

    if start > end {
        return Err(BenchmarkError::Usage(
            "--start must be less than or equal to --end".to_owned(),
        ));
    }
    if threads == 0 {
        return Err(BenchmarkError::Usage(
            "--threads must be greater than zero".to_owned(),
        ));
    }
    if chunk_size == 0 {
        return Err(BenchmarkError::Usage(
            "--chunk-size must be greater than zero".to_owned(),
        ));
    }
    if segment_size == 0 {
        return Err(BenchmarkError::Usage(
            "--segment-size must be greater than zero".to_owned(),
        ));
    }
    if cache_segments == 0 {
        return Err(BenchmarkError::Usage(
            "--cache-segments must be greater than zero".to_owned(),
        ));
    }

    Ok(Config {
        start,
        end,
        threads,
        chunk_size,
        mode,
        json_only,
        no_discord,
        segment_size,
        cache_segments,
    })
}

fn parse_value(name: &str, value: Option<String>) -> Result<u64, BenchmarkError> {
    let value = value.ok_or_else(|| BenchmarkError::Usage(format!("{name} requires a value")))?;
    value
        .parse::<u64>()
        .map_err(|_| BenchmarkError::Usage(format!("{name} value must be an unsigned integer")))
}

fn parse_mode(value: Option<String>) -> Result<BenchmarkMode, BenchmarkError> {
    match value.as_deref() {
        Some("full-scan") => Ok(BenchmarkMode::FullScan),
        Some("conjecture-domain") => Ok(BenchmarkMode::ConjectureDomain),
        Some(value) => Err(BenchmarkError::Usage(format!(
            "unknown benchmark mode {value}; use full-scan or conjecture-domain"
        ))),
        None => Err(BenchmarkError::Usage(
            "--mode requires full-scan or conjecture-domain".to_owned(),
        )),
    }
}

fn parse_usize(name: &str, value: Option<String>) -> Result<usize, BenchmarkError> {
    let value = value.ok_or_else(|| BenchmarkError::Usage(format!("{name} requires a value")))?;
    value
        .parse::<usize>()
        .map_err(|_| BenchmarkError::Usage(format!("{name} value must be a positive integer")))
}

fn measure(config: &Config) -> Result<BenchmarkReport, BenchmarkError> {
    let total_start = Instant::now();
    let maximum_q = maximum_q_for_range(config.end);
    let prime_limit = config
        .end
        .checked_mul(3)
        .and_then(|value| value.checked_add(1))
        .ok_or(BenchmarkError::ArithmeticOverflow)?;

    let prime_start = Instant::now();
    let mut primes = PrimeOracle::new(prime_limit, config.segment_size, config.cache_segments)
        .map_err(|error| BenchmarkError::PrimeOracle(error.to_string()))?;
    let prime_oracle_bytes = primes.approximate_memory_bytes();
    let prime_generation_seconds = prime_start.elapsed().as_secs_f64();

    let q_start = Instant::now();
    let q_list = build_q_list(maximum_q, &mut primes)
        .map_err(|error| BenchmarkError::QFilter(error.to_string()))?;
    let q_generation_seconds = q_start.elapsed().as_secs_f64();

    let q_list_sha256 = hex_digest(hash_q_list(&q_list));
    let verification_start = Instant::now();
    let (n_processed, total_q_tests, zero_solution_n) = if config.threads == 1 {
        verify_sequential(config.start, config.end, &mut primes, &q_list)?
    } else {
        let result = verify_range_parallel(
            config.start,
            config.end,
            &primes,
            &q_list,
            config.threads,
            config.chunk_size,
        )
        .map_err(|error| BenchmarkError::Parallel(error.to_string()))?;
        (
            result.n_processed,
            result.total_q_tests,
            result.zero_solution_n,
        )
    };
    let mut zero_solution_in_domain = Vec::new();
    let mut zero_solution_outside_domain = Vec::new();
    for n in &zero_solution_n {
        if is_in_conjecture_domain(*n) {
            zero_solution_in_domain.push(*n);
        } else {
            zero_solution_outside_domain.push(*n);
        }
    }

    let verification_seconds = verification_start.elapsed().as_secs_f64();
    let total_seconds = total_start.elapsed().as_secs_f64();

    Ok(BenchmarkReport {
        start: config.start,
        end: config.end,
        mode: config.mode,
        threads: config.threads,
        chunk_size: config.chunk_size,
        q_list_sha256,
        prime_limit,
        q_count: q_list.len(),
        prime_generation_seconds,
        q_generation_seconds,
        verification_seconds,
        total_seconds,
        n_processed,
        total_q_tests,
        zero_solution_n,
        zero_solution_in_domain,
        zero_solution_outside_domain,
        prime_oracle_bytes,
    })
}

fn verify_sequential(
    start: u64,
    end: u64,
    primes: &mut PrimeOracle,
    q_list: &a220091::QList,
) -> Result<(u64, u64, Vec<u64>), BenchmarkError> {
    let mut n_processed = 0_u64;
    let mut total_q_tests = 0_u64;
    let mut zero_solution_n = Vec::new();
    let mut n = start;
    while n <= end {
        let result =
            find_representation(n, primes, q_list).map_err(BenchmarkError::Verification)?;
        n_processed = n_processed
            .checked_add(1)
            .ok_or(BenchmarkError::ArithmeticOverflow)?;
        total_q_tests = total_q_tests
            .checked_add(result.q_candidates_tested)
            .ok_or(BenchmarkError::ArithmeticOverflow)?;
        if result.representation.is_none() {
            zero_solution_n.push(n);
        }
        n = match n.checked_add(1) {
            Some(next) => next,
            None => break,
        };
    }
    Ok((n_processed, total_q_tests, zero_solution_n))
}

fn maximum_q_for_range(end: u64) -> u64 {
    end.saturating_sub(1) / 2
}

fn is_in_conjecture_domain(n: u64) -> bool {
    if n % 2 == 0 {
        n >= EVEN_CONJECTURE_MIN
    } else {
        n >= ODD_CONJECTURE_MIN
    }
}

fn counterexamples_for_mode(values: &[u64], _mode: BenchmarkMode) -> Vec<u64> {
    values
        .iter()
        .copied()
        .filter(|&n| is_in_conjecture_domain(n))
        .collect()
}

fn print_human_summary(report: &BenchmarkReport) {
    println!("A220091 verification benchmark");
    println!("mode: {}", report.mode.label());
    println!("threads: {}", report.threads);
    println!("chunk size: {}", report.chunk_size);
    println!("Q-list SHA-256: {}", report.q_list_sha256);
    println!(
        "range: [{}..={}], prime limit: {}",
        report.start, report.end, report.prime_limit
    );
    println!("Q-list size: {}", report.q_count);
    println!("n processed: {}", report.n_processed);
    println!("total q candidates tested: {}", report.total_q_tests);
    println!("prime generation: {:.6} s", report.prime_generation_seconds);
    println!("Q-list construction: {:.6} s", report.q_generation_seconds);
    println!("verification: {:.6} s", report.verification_seconds);
    println!("total: {:.6} s", report.total_seconds);
    println!(
        "n/sec: {:.6}",
        rate(report.n_processed, report.verification_seconds)
    );
    println!(
        "q-tests/sec: {:.6}",
        rate(report.total_q_tests, report.verification_seconds)
    );
    println!(
        "prime oracle storage (bounded, segmented): {} bytes (process-wide memory unavailable)",
        report.prime_oracle_bytes
    );
    println!("zero-solution n (all): {:?}", report.zero_solution_n);
    println!(
        "zero-solution n (conjecture domain): {:?}",
        report.zero_solution_in_domain
    );
    println!(
        "zero-solution n (outside conjecture domain): {:?}",
        report.zero_solution_outside_domain
    );
}

fn rate(count: u64, seconds: f64) -> f64 {
    if seconds > 0.0 {
        count as f64 / seconds
    } else {
        0.0
    }
}

fn report_json(report: &BenchmarkReport) -> String {
    format!(
        concat!(
            "{{",
            "\"start\":{},",
            "\"end\":{},",
            "\"mode\":\"{}\",",
            "\"threads\":{},",
            "\"chunk_size\":{},",
            "\"git_commit\":{},",
            "\"docker_image_id\":{},",
            "\"source_hash\":{},",
            "\"diagnostics\":{},",
            "\"q_list_sha256\":\"{}\",",
            "\"prime_limit\":{},",
            "\"q_count\":{},",
            "\"prime_generation_seconds\":{:.9},",
            "\"q_generation_seconds\":{:.9},",
            "\"verification_seconds\":{:.9},",
            "\"total_seconds\":{:.9},",
            "\"n_processed\":{},",
            "\"total_q_tests\":{},",
            "\"n_per_second\":{:.9},",
            "\"q_tests_per_second\":{:.9},",
            "\"zero_solution_n\":{},",
            "\"zero_solution_in_domain\":{},",
            "\"zero_solution_outside_domain\":{},",
            "\"prime_oracle_bytes\":{},",
            "\"process_memory_bytes\":null",
            "}}"
        ),
        report.start,
        report.end,
        report.mode.label(),
        report.threads,
        report.chunk_size,
        json_optional_env("A220091_GIT_COMMIT"),
        json_optional_env("A220091_DOCKER_IMAGE_ID"),
        json_optional_env("A220091_SOURCE_HASH"),
        json_optional_env("A220091_DIAGNOSTICS"),
        report.q_list_sha256,
        report.prime_limit,
        report.q_count,
        report.prime_generation_seconds,
        report.q_generation_seconds,
        report.verification_seconds,
        report.total_seconds,
        report.n_processed,
        report.total_q_tests,
        rate(report.n_processed, report.verification_seconds),
        rate(report.total_q_tests, report.verification_seconds),
        json_u64_list(&report.zero_solution_n),
        json_u64_list(&report.zero_solution_in_domain),
        json_u64_list(&report.zero_solution_outside_domain),
        report.prime_oracle_bytes,
    )
}

fn json_u64_list(values: &[u64]) -> String {
    let body = values
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!("[{body}]")
}

fn json_optional_env(name: &str) -> String {
    std::env::var(name).map_or_else(
        |_| "null".to_owned(),
        |value| format!("\"{}\"", value.replace('"', "\\\"")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conjecture_domain_uses_parity_specific_thresholds() {
        assert!(!is_in_conjecture_domain(8_069));
        assert!(is_in_conjecture_domain(8_070));
        assert!(!is_in_conjecture_domain(8_071));
        assert!(!is_in_conjecture_domain(18_679));
        assert!(is_in_conjecture_domain(18_680));
        assert!(is_in_conjecture_domain(18_681));
    }

    #[test]
    fn known_small_zero_solution_is_outside_conjecture_domain() {
        assert_eq!(
            counterexamples_for_mode(&[91], BenchmarkMode::ConjectureDomain),
            Vec::<u64>::new()
        );
        assert_eq!(
            counterexamples_for_mode(&[91], BenchmarkMode::FullScan),
            Vec::<u64>::new()
        );
    }

    #[test]
    fn hypothetical_threshold_zero_solutions_are_counterexamples() {
        assert_eq!(
            counterexamples_for_mode(&[8_070], BenchmarkMode::ConjectureDomain),
            vec![8_070]
        );
        assert_eq!(
            counterexamples_for_mode(&[18_680], BenchmarkMode::ConjectureDomain),
            vec![18_680]
        );
    }
}
