use std::{
    env, fmt,
    path::{Path, PathBuf},
    process,
    sync::Arc,
    time::Duration,
};

use a220091::{
    campaign::{
        self, CampaignConfig, CampaignError, CampaignStatus, RunOptions, DEFAULT_BLOCK_SIZE,
    },
    notifier::{DiscordNotifier, DiscordProgressSink, NotificationEvent},
    prime_oracle::{DEFAULT_CACHE_SEGMENTS, DEFAULT_SEGMENT_SIZE},
};

const DEFAULT_DB: &str = "/var/lib/a220091/campaign/campaign.db";
const DEFAULT_CHUNK_SIZE: u64 = 1_000_000;

#[derive(Debug)]
enum CliError {
    Usage(String),
    Campaign(CampaignError),
    Other(String),
    Counterexamples(Vec<u64>),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::Other(message) => formatter.write_str(message),
            Self::Campaign(error) => write!(formatter, "{error}"),
            Self::Counterexamples(values) => {
                write!(formatter, "in-domain counterexamples found: {values:?}")
            }
        }
    }
}

impl From<CampaignError> for CliError {
    fn from(error: CampaignError) -> Self {
        Self::Campaign(error)
    }
}

#[derive(Debug)]
struct RunArgs {
    db: PathBuf,
    start: u64,
    end: u64,
    chunk_size: u64,
    block_size: u64,
    threads: usize,
    mode: String,
    recover: bool,
    max_chunks: Option<u64>,
    heartbeat_seconds: u64,
    no_discord: bool,
    segment_size: u64,
    cache_segments: usize,
    notification_state: Option<PathBuf>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("campaign failed: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), CliError> {
    let mut arguments: Vec<String> = env::args().skip(1).collect();
    let command = match arguments.first().map(String::as_str) {
        Some("run" | "status" | "pause") => arguments.remove(0),
        Some("--help" | "-h") => {
            print_help();
            return Ok(());
        }
        _ => "run".to_owned(),
    };
    match command.as_str() {
        "status" => status(&arguments),
        "pause" => pause(&arguments),
        _ => run_command(&arguments),
    }
}

fn print_help() {
    println!(
        "Usage:\n  campaign run    [--db PATH] --start N --end N [--chunk-size N] [--block-size N] [--threads N] [--mode conjecture-domain|full-scan] [--recover] [--max-chunks N] [--heartbeat-seconds N] [--no-discord] [--segment-size N] [--cache-segments N] [--notification-state PATH]\n  campaign status [--db PATH]\n  campaign pause  [--db PATH]\n\nSQLite is the only persistent campaign state. `run` creates the database on first use and resumes it afterwards."
    );
}

fn value(arguments: &mut std::vec::IntoIter<String>, name: &str) -> Result<String, CliError> {
    arguments
        .next()
        .ok_or_else(|| CliError::Usage(format!("{name} requires a value")))
}

fn number<T: std::str::FromStr>(
    arguments: &mut std::vec::IntoIter<String>,
    name: &str,
) -> Result<T, CliError> {
    value(arguments, name)?
        .parse()
        .map_err(|_| CliError::Usage(format!("{name} value must be an unsigned integer")))
}

fn db_only(arguments: &[String]) -> Result<PathBuf, CliError> {
    let mut db = PathBuf::from(DEFAULT_DB);
    let mut iterator = arguments.to_vec().into_iter();
    while let Some(argument) = iterator.next() {
        match argument.as_str() {
            "--db" => db = PathBuf::from(value(&mut iterator, "--db")?),
            unknown => return Err(CliError::Usage(format!("unknown argument {unknown}"))),
        }
    }
    Ok(db)
}

fn status(arguments: &[String]) -> Result<(), CliError> {
    let db = db_only(arguments)?;
    if !db.exists() {
        return Err(CliError::Other(format!(
            "no campaign database at {}",
            db.display()
        )));
    }
    let connection = campaign::open_connection(&db)?;
    let state = campaign::load_state(&connection)?;
    println!("status: {}", state.status.as_str());
    println!("range: [{}..={}]", state.range_start, state.range_end);
    println!(
        "current position: {} (next unverified n)",
        state.current_position
    );
    println!("chunk size: {}", state.chunk_size);
    println!("block size: {}", state.block_size);
    println!(
        "blocks completed: {} of {}",
        state.blocks_completed,
        state.total_blocks()
    );
    println!("chunks completed: {}", state.chunks_completed);
    println!("n processed: {}", state.n_processed);
    println!("total q tests: {}", state.total_q_tests);
    println!("zero-solution count: {}", state.zero_solution_count);
    println!("elapsed seconds: {}", state.elapsed_seconds);
    println!("run count: {}", state.run_count);
    println!("crash recoveries: {}", state.crash_recoveries);
    println!("pause requested: {}", state.pause_requested);
    if let Some(root) = &state.result_root {
        println!("result root: {root}");
    }
    Ok(())
}

fn pause(arguments: &[String]) -> Result<(), CliError> {
    let db = db_only(arguments)?;
    if campaign::request_pause(&db)? {
        println!("pause requested; the run stops after its next committed chunk");
    } else {
        println!("campaign is not RUNNING; nothing to pause");
    }
    Ok(())
}

fn parse_run(arguments: &[String]) -> Result<RunArgs, CliError> {
    let mut args = RunArgs {
        db: PathBuf::from(DEFAULT_DB),
        start: 0,
        end: 0,
        chunk_size: DEFAULT_CHUNK_SIZE,
        block_size: DEFAULT_BLOCK_SIZE,
        threads: 1,
        mode: "conjecture-domain".to_owned(),
        recover: false,
        max_chunks: None,
        heartbeat_seconds: 60,
        no_discord: false,
        segment_size: DEFAULT_SEGMENT_SIZE,
        cache_segments: DEFAULT_CACHE_SEGMENTS,
        notification_state: None,
    };
    let mut have_start = false;
    let mut have_end = false;
    let mut iterator = arguments.to_vec().into_iter();
    while let Some(argument) = iterator.next() {
        match argument.as_str() {
            "--db" => args.db = PathBuf::from(value(&mut iterator, "--db")?),
            "--start" => {
                args.start = number(&mut iterator, "--start")?;
                have_start = true;
            }
            "--end" => {
                args.end = number(&mut iterator, "--end")?;
                have_end = true;
            }
            "--chunk-size" => args.chunk_size = number(&mut iterator, "--chunk-size")?,
            "--block-size" => args.block_size = number(&mut iterator, "--block-size")?,
            "--threads" => args.threads = number(&mut iterator, "--threads")?,
            "--mode" => {
                let mode = value(&mut iterator, "--mode")?;
                if mode != "conjecture-domain" && mode != "full-scan" {
                    return Err(CliError::Usage(
                        "--mode must be conjecture-domain or full-scan".to_owned(),
                    ));
                }
                args.mode = mode;
            }
            "--recover" => args.recover = true,
            "--max-chunks" => args.max_chunks = Some(number(&mut iterator, "--max-chunks")?),
            "--heartbeat-seconds" => {
                args.heartbeat_seconds = number(&mut iterator, "--heartbeat-seconds")?
            }
            "--no-discord" => args.no_discord = true,
            "--segment-size" => args.segment_size = number(&mut iterator, "--segment-size")?,
            "--cache-segments" => args.cache_segments = number(&mut iterator, "--cache-segments")?,
            "--notification-state" => {
                args.notification_state =
                    Some(PathBuf::from(value(&mut iterator, "--notification-state")?))
            }
            unknown => {
                return Err(CliError::Usage(format!(
                    "unknown argument {unknown}; use --help for usage"
                )))
            }
        }
    }
    if !have_start || !have_end {
        return Err(CliError::Usage("--start and --end are required".to_owned()));
    }
    if args.threads == 0
        || args.chunk_size == 0
        || args.block_size == 0
        || args.segment_size == 0
        || args.cache_segments == 0
        || args.heartbeat_seconds == 0
    {
        return Err(CliError::Usage(
            "thread, chunk, segment, cache and heartbeat values must be greater than zero"
                .to_owned(),
        ));
    }
    Ok(args)
}

fn run_command(arguments: &[String]) -> Result<(), CliError> {
    let args = parse_run(arguments)?;
    if let Some(parent) = args.db.parent() {
        std::fs::create_dir_all(parent).map_err(|error| CliError::Other(error.to_string()))?;
    }

    let config = CampaignConfig::new(
        args.start,
        args.end,
        args.chunk_size,
        args.block_size,
        &args.mode,
        env::var("A220091_SOURCE_VERSION").ok(),
    );
    campaign::open_or_create(&args.db, &config, campaign::unix_now())?;

    let notification_state = args
        .notification_state
        .clone()
        .unwrap_or_else(|| default_notification_state(&args.db));
    let notifier = match DiscordNotifier::from_env(args.no_discord, notification_state) {
        Ok(value) => value.map(Arc::new),
        Err(_) => {
            eprintln!("Discord configuration unavailable; computation continues");
            None
        }
    };
    let notify = |event: NotificationEvent, message: &str| {
        if let Some(notifier) = &notifier {
            if notifier.notify(event, message).is_err() {
                eprintln!("Discord notification failed; computation continues");
            }
        }
    };
    let sink = notifier
        .as_ref()
        .map(|notifier| DiscordProgressSink::new(Arc::clone(notifier), args.start, args.end));

    let mut options = RunOptions::new(args.threads);
    options.heartbeat_interval = Duration::from_secs(args.heartbeat_seconds);
    options.recover = args.recover;
    options.max_chunks = args.max_chunks;
    options.segment_size = args.segment_size;
    options.cache_segments = args.cache_segments;

    notify(
        NotificationEvent::Start,
        &format!(
            "Status: RUNNING\nRange: {} -> {}\nThreads: {}\nChunk size: {}\nBlock size: {}",
            args.start, args.end, args.threads, args.chunk_size, args.block_size
        ),
    );

    let result = campaign::run_campaign(
        &args.db,
        &options,
        sink.as_ref()
            .map(|sink| sink as &dyn campaign::ChunkProgressSink),
    );

    match result {
        Ok(outcome) => {
            let state = &outcome.state;
            println!("status: {}", state.status.as_str());
            println!("chunks committed this run: {}", outcome.chunks_committed);
            println!("blocks completed: {}", state.blocks_completed);
            println!("current position: {}", state.current_position);
            println!("elapsed seconds: {}", state.elapsed_seconds);
            if outcome.recovered_crash {
                println!("recovered from an unclean previous run");
                notify(
                    NotificationEvent::Recovery,
                    "Status: RECOVERED\nPrevious run ended without a clean stop.",
                );
            }
            if let Some(root) = &state.result_root {
                println!("result root: {root}");
            }
            if !outcome.in_domain_counterexamples.is_empty() {
                notify(
                    NotificationEvent::ImportantError,
                    "Status: FAILED\nIn-domain counterexample found.",
                );
                return Err(CliError::Counterexamples(outcome.in_domain_counterexamples));
            }
            if state.status == CampaignStatus::Completed {
                notify(
                    NotificationEvent::Completed,
                    &format!(
                        "Status: COMPLETED\nRange: {} -> {}\nQ candidates: {}\nResult root: {}\nElapsed seconds: {}",
                        state.range_start,
                        state.range_end,
                        state.total_q_tests,
                        state.result_root.as_deref().unwrap_or("unavailable"),
                        state.elapsed_seconds
                    ),
                );
            } else {
                notify(
                    NotificationEvent::Stopped,
                    &format!(
                        "Status: PAUSED\nCompleted through: {}",
                        state.current_position.saturating_sub(1)
                    ),
                );
            }
            Ok(())
        }
        Err(CampaignError::AlreadyCompleted) => {
            println!("campaign is already COMPLETED; nothing to do");
            Ok(())
        }
        Err(error) => {
            notify(
                NotificationEvent::ImportantError,
                "Status: FAILED\nCampaign stopped with an error; see container logs.",
            );
            Err(error.into())
        }
    }
}

fn default_notification_state(db: &Path) -> PathBuf {
    db.parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
        .join("notifier-state.json")
}
