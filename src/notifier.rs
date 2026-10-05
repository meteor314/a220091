use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::campaign::{ChunkProgressSink, ChunkRecord};

const DAILY_SECONDS: u64 = 24 * 60 * 60;
const MAX_ATTEMPTS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationEvent {
    Start,
    DailyProgress,
    ImportantError,
    Completed,
    Recovery,
    Stopped,
}

impl NotificationEvent {
    fn label(self) -> &'static str {
        match self {
            Self::Start => "START",
            Self::DailyProgress => "DAILY_PROGRESS",
            Self::ImportantError => "IMPORTANT_ERROR",
            Self::Completed => "COMPLETED",
            Self::Recovery => "RECOVERY",
            Self::Stopped => "STOPPED",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DailyState {
    pub last_daily_timestamp: Option<u64>,
    pub last_completed_through: Option<u64>,
    pub last_notification_type: Option<String>,
}

impl Default for DailyState {
    fn default() -> Self {
        Self {
            last_daily_timestamp: None,
            last_completed_through: None,
            last_notification_type: None,
        }
    }
}

#[derive(Debug)]
pub enum NotifierError {
    InvalidConfiguration(String),
    State(String),
    Request(String),
}

impl std::fmt::Display for NotifierError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfiguration(message) => {
                write!(formatter, "invalid Discord configuration: {message}")
            }
            Self::State(message) => write!(formatter, "Discord state error: {message}"),
            Self::Request(message) => write!(formatter, "Discord request error: {message}"),
        }
    }
}

impl std::error::Error for NotifierError {}

#[derive(Debug)]
struct NotifierInner {
    url: String,
    client: Client,
    state_path: PathBuf,
    state: DailyState,
}

#[derive(Debug)]
pub struct DiscordNotifier {
    inner: Mutex<NotifierInner>,
}

impl DiscordNotifier {
    /// Missing DISCORD_WEBHOOK_URL disables notifications without error.
    pub fn from_env(
        disabled: bool,
        state_path: impl Into<PathBuf>,
    ) -> Result<Option<Self>, NotifierError> {
        if disabled {
            return Ok(None);
        }
        let Some(url) = std::env::var_os("DISCORD_WEBHOOK_URL") else {
            return Ok(None);
        };
        let url = url.to_string_lossy().trim().to_owned();
        if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() < 12 {
            return Err(NotifierError::InvalidConfiguration(
                "DISCORD_WEBHOOK_URL must be an HTTP(S) URL".to_owned(),
            ));
        }
        Self::from_url(url, state_path)
    }

    fn from_url(
        url: String,
        state_path: impl Into<PathBuf>,
    ) -> Result<Option<Self>, NotifierError> {
        if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() < 12 {
            return Err(NotifierError::InvalidConfiguration(
                "DISCORD_WEBHOOK_URL must be an HTTP(S) URL".to_owned(),
            ));
        }
        let state_path = state_path.into();
        let state = load_state(&state_path)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| NotifierError::Request(error.to_string()))?;
        Ok(Some(Self {
            inner: Mutex::new(NotifierInner {
                url,
                client,
                state_path,
                state,
            }),
        }))
    }

    pub fn notify(&self, event: NotificationEvent, content: &str) -> Result<(), NotifierError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| NotifierError::State("mutex poisoned".to_owned()))?;
        let payload = json!({"content": format!("A220091 {}\n{}", event.label(), content)});
        let mut last_error = None;
        for attempt in 0..MAX_ATTEMPTS {
            match inner.client.post(&inner.url).json(&payload).send() {
                Ok(response) if response.status().is_success() => {
                    inner.state.last_notification_type = Some(event.label().to_owned());
                    if event == NotificationEvent::DailyProgress {
                        inner.state.last_daily_timestamp = Some(now_seconds());
                    }
                    persist_state(&inner.state_path, &inner.state)?;
                    return Ok(());
                }
                Ok(response) => {
                    let retry_after = response
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok())
                        .unwrap_or(0)
                        .min(2);
                    last_error = Some(format!("HTTP {}", response.status()));
                    if attempt + 1 < MAX_ATTEMPTS && retry_after > 0 {
                        std::thread::sleep(Duration::from_secs(retry_after));
                    }
                }
                Err(_) => last_error = Some("transport failure".to_owned()),
            }
        }
        Err(NotifierError::Request(
            last_error.unwrap_or_else(|| "request failed".to_owned()),
        ))
    }

    pub fn maybe_daily_progress(
        &self,
        completed_through: u64,
        now: u64,
        content: &str,
    ) -> Result<bool, NotifierError> {
        let should_send = {
            let inner = self
                .inner
                .lock()
                .map_err(|_| NotifierError::State("mutex poisoned".to_owned()))?;
            should_send_daily(&inner.state, completed_through, now)
        };
        if !should_send {
            return Ok(false);
        }
        self.notify(NotificationEvent::DailyProgress, content)?;
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| NotifierError::State("mutex poisoned".to_owned()))?;
        inner.state.last_completed_through = Some(completed_through);
        persist_state(&inner.state_path, &inner.state)?;
        Ok(true)
    }
}

pub fn should_send_daily(state: &DailyState, completed_through: u64, now: u64) -> bool {
    if state.last_completed_through == Some(completed_through) {
        return false;
    }
    state
        .last_daily_timestamp
        .is_none_or(|last| now.saturating_sub(last) >= DAILY_SECONDS)
}

pub fn progress_percentage(start: u64, end: u64, completed_through: u64) -> f64 {
    if start >= end {
        return if completed_through >= end { 100.0 } else { 0.0 };
    }
    let completed = completed_through
        .saturating_sub(start)
        .saturating_add(1)
        .min(end - start + 1);
    completed as f64 * 100.0 / (end - start + 1) as f64
}

pub fn chunk_progress_message(chunk: &ChunkRecord, start: u64, end: u64) -> String {
    format!(
        "Status: RUNNING\nProgress: {:.1}%\nRange: {} -> {}\nCompleted through: {}\nChunk: {} -> {}\nQ candidates: {}\nChunk result: {}",
        progress_percentage(start, end, chunk.chunk_end),
        start,
        end,
        chunk.chunk_end,
        chunk.chunk_start,
        chunk.chunk_end,
        chunk.q_tests,
        chunk.result_sha256,
    )
}

fn load_state(path: &Path) -> Result<DailyState, NotifierError> {
    match fs::read(path) {
        Ok(contents) => serde_json::from_slice(&contents)
            .map_err(|error| NotifierError::State(error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DailyState::default()),
        Err(error) => Err(NotifierError::State(error.to_string())),
    }
}

fn persist_state(path: &Path, state: &DailyState) -> Result<(), NotifierError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| NotifierError::State(error.to_string()))?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(state)
            .map_err(|error| NotifierError::State(error.to_string()))?,
    )
    .map_err(|error| NotifierError::State(error.to_string()))?;
    fs::rename(temporary, path).map_err(|error| NotifierError::State(error.to_string()))
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug)]
pub struct DiscordProgressSink {
    notifier: std::sync::Arc<DiscordNotifier>,
    start: u64,
    end: u64,
}

impl DiscordProgressSink {
    pub fn new(notifier: std::sync::Arc<DiscordNotifier>, start: u64, end: u64) -> Self {
        Self {
            notifier,
            start,
            end,
        }
    }
}

impl ChunkProgressSink for DiscordProgressSink {
    fn chunk_committed(&self, chunk: &ChunkRecord) {
        let content = chunk_progress_message(chunk, self.start, self.end);
        if let Err(_) = self
            .notifier
            .maybe_daily_progress(chunk.chunk_end, now_seconds(), &content)
        {
            eprintln!("Discord notification failed; computation continues");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn missing_configuration_disables_notifications() {
        std::env::remove_var("DISCORD_WEBHOOK_URL");
        assert!(
            DiscordNotifier::from_env(true, "notifications/test-state.json")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn malformed_configuration_is_rejected_without_echoing_value() {
        let error =
            DiscordNotifier::from_url("not-a-url".to_owned(), "notifications/test-state.json")
                .unwrap_err();
        assert!(error.to_string().contains("HTTP(S) URL"));
        assert!(!error.to_string().contains("not-a-url"));
    }

    #[test]
    fn daily_decision_and_percentage_are_deterministic() {
        let state = DailyState {
            last_daily_timestamp: Some(100),
            last_completed_through: Some(10),
            last_notification_type: Some("DAILY_PROGRESS".to_owned()),
        };
        assert!(!should_send_daily(&state, 10, 100 + DAILY_SECONDS));
        assert!(should_send_daily(&state, 11, 100 + DAILY_SECONDS));
        assert!((progress_percentage(10, 20, 15) - 54.54545454545455).abs() < f64::EPSILON);
    }

    #[test]
    fn messages_do_not_contain_webhook_configuration() {
        let chunk = ChunkRecord {
            chunk_start: 10,
            chunk_end: 20,
            result_sha256: "r".to_owned(),
            n_processed: 11,
            q_tests: 3,
            zero_solution_n: Vec::new(),
            completed_at: 0,
        };
        let message = chunk_progress_message(&chunk, 1, 100);
        assert!(!message.contains("DISCORD_WEBHOOK_URL"));
        assert!(message.contains("Progress:"));
    }

    #[test]
    fn mock_server_receives_all_supported_events() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut bodies = Vec::new();
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 4096];
                let size = stream.read(&mut request).unwrap_or(0);
                bodies.push(String::from_utf8_lossy(&request[..size]).to_string());
                stream
                    .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            }
            bodies
        });
        let state_path =
            std::env::temp_dir().join(format!("a220091-discord-state-{}", std::process::id()));
        let notifier = DiscordNotifier::from_url(format!("http://{address}/webhook"), &state_path)
            .unwrap()
            .unwrap();
        for event in [
            NotificationEvent::Start,
            NotificationEvent::DailyProgress,
            NotificationEvent::ImportantError,
            NotificationEvent::Completed,
        ] {
            notifier.notify(event, "test progress").unwrap();
        }
        let bodies = server.join().unwrap();
        assert_eq!(bodies.len(), 4);
        assert!(bodies.iter().all(|body| body.contains("test progress")));
        assert!(bodies
            .iter()
            .all(|body| !body.contains("DISCORD_WEBHOOK_URL")));
        let _ = fs::remove_file(state_path);
    }

    #[test]
    fn failed_endpoint_is_non_fatal_to_callers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let state_path =
            std::env::temp_dir().join(format!("a220091-discord-failure-{}", std::process::id()));
        let notifier = DiscordNotifier::from_url(format!("http://{address}/webhook"), &state_path)
            .unwrap()
            .unwrap();
        let error = notifier
            .notify(NotificationEvent::ImportantError, "test")
            .unwrap_err();
        assert!(!error.to_string().contains(&address.to_string()));
        let _ = fs::remove_file(state_path);
    }
}
