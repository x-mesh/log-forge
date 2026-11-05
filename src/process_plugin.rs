use crate::{GeneratorError, LogGenerator};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::{Duration, Instant},
};

const MAX_RESPONSE: u64 = 1_048_576;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u8,
    pattern: Option<String>,
    record: Option<String>,
    error: Option<String>,
    supports_shutdown: Option<bool>,
    shutdown_complete: Option<bool>,
}

pub(crate) struct ProcessGenerator {
    child: Child,
    requests: Option<SyncSender<Vec<u8>>>,
    written: Receiver<io::Result<()>>,
    responses: Receiver<io::Result<Vec<u8>>>,
    timeout: Duration,
    pattern: String,
    validator: regex::Regex,
    supports_shutdown: bool,
    finished: bool,
}
impl ProcessGenerator {
    pub fn spawn(
        command: &Path,
        args: &[String],
        config: &serde_yaml::Value,
        seed: u64,
        timeout: Duration,
    ) -> Result<Self, GeneratorError> {
        let value = serde_json::to_value(config)?;
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take().ok_or("plugin stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("plugin stdout unavailable")?;
        let (requests, input) = mpsc::sync_channel::<Vec<u8>>(1);
        let (written, acknowledgements) = mpsc::sync_channel(1);
        if let Err(error) = thread::Builder::new()
            .name("log-forge-plugin-input".into())
            .spawn(move || {
                let mut stdin = stdin;
                for request in input {
                    let result = stdin.write_all(&request).and_then(|_| stdin.flush());
                    let failed = result.is_err();
                    if written.send(result).is_err() || failed {
                        break;
                    }
                }
            })
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        let (sender, responses) = mpsc::sync_channel(1);
        if let Err(error) = thread::Builder::new()
            .name("log-forge-plugin".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let mut line = Vec::new();
                    let result = (&mut reader)
                        .take(MAX_RESPONSE + 1)
                        .read_until(b'\n', &mut line)
                        .and_then(|count| {
                            if count == 0 {
                                Err(io::Error::new(
                                    io::ErrorKind::UnexpectedEof,
                                    "plugin closed stdout",
                                ))
                            } else if count as u64 > MAX_RESPONSE || !line.ends_with(b"\n") {
                                Err(io::Error::other(
                                    "plugin response exceeds limit or lacks newline",
                                ))
                            } else {
                                Ok(line)
                            }
                        });
                    let failed = result.is_err();
                    if sender.send(result).is_err() || failed {
                        break;
                    }
                }
            })
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        let mut plugin = Self {
            child,
            requests: Some(requests),
            written: acknowledgements,
            responses,
            timeout,
            pattern: String::new(),
            validator: regex::Regex::new("^$")?,
            supports_shutdown: false,
            finished: false,
        };
        let response = plugin
            .exchange(json!({"version": 1, "action": "init", "config": value, "seed": seed}))?;
        if response.record.is_some() || response.pattern.as_ref().is_none_or(String::is_empty) {
            return Err("plugin init must return a nonempty pattern".into());
        }
        plugin.supports_shutdown = response.supports_shutdown.unwrap_or(false);
        plugin.pattern = response.pattern.ok_or("missing plugin pattern")?;
        plugin.validator = match regex::Regex::new(&plugin.pattern) {
            Ok(validator) => validator,
            Err(error) => {
                if let Err(cleanup) = plugin.finish() {
                    return Err(format!("{error}; plugin cleanup failed: {cleanup}").into());
                }
                return Err(error.into());
            }
        };
        Ok(plugin)
    }
    fn exchange(&mut self, request: serde_json::Value) -> Result<Response, GeneratorError> {
        let deadline = Instant::now()
            .checked_add(self.timeout)
            .ok_or("plugin timeout is too large")?;
        let mut bytes = serde_json::to_vec(&request)?;
        if bytes.len() as u64 >= MAX_RESPONSE {
            return Err("plugin request exceeds limit".into());
        }
        bytes.push(b'\n');
        self.requests
            .as_ref()
            .ok_or("plugin stdin closed")?
            .try_send(bytes)
            .map_err(|_| io::Error::other("plugin input unavailable"))?;
        let wait = |deadline: Instant| deadline.saturating_duration_since(Instant::now());
        self.written
            .recv_timeout(wait(deadline))
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => {
                    io::Error::new(io::ErrorKind::TimedOut, "plugin request timed out")
                }
                mpsc::RecvTimeoutError::Disconnected => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "plugin input channel closed")
                }
            })??;
        let bytes =
            self.responses
                .recv_timeout(wait(deadline))
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => {
                        io::Error::new(io::ErrorKind::TimedOut, "plugin response timed out")
                    }
                    mpsc::RecvTimeoutError::Disconnected => io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "plugin response channel closed",
                    ),
                })??;
        let response: Response = serde_json::from_slice(&bytes)?;
        if response.version != 1 {
            return Err("unsupported plugin protocol version".into());
        }
        if let Some(error) = &response.error {
            if response.shutdown_complete == Some(true) {
                self.finished = true;
            }
            return Err(format!("plugin error: {error}").into());
        }
        if response.shutdown_complete.is_some() {
            return Err("plugin returned unexpected shutdown metadata".into());
        }
        Ok(response)
    }
}
impl LogGenerator for ProcessGenerator {
    fn generate(&mut self, timestamp: DateTime<Utc>) -> Result<String, GeneratorError> {
        let response = self.exchange(
            json!({"version": 1, "action": "generate", "timestamp": timestamp.to_rfc3339()}),
        )?;
        if response.pattern.is_some() || response.supports_shutdown.is_some() {
            return Err("plugin generate returned unexpected metadata".into());
        }
        response
            .record
            .filter(|record| !record.is_empty())
            .ok_or_else(|| "plugin returned an empty record".into())
    }
    fn pattern(&self) -> &str {
        &self.pattern
    }
    fn validate(&self, record: &str) -> bool {
        self.validator.is_match(record)
    }
    fn finish(&mut self) -> Result<(), GeneratorError> {
        if self.finished {
            return Ok(());
        }
        if self.supports_shutdown {
            let response = self.exchange(json!({"version": 1, "action": "shutdown"}))?;
            if response.pattern.is_some()
                || response.record.is_some()
                || response.supports_shutdown.is_some()
            {
                return Err("plugin shutdown returned unexpected data".into());
            }
        }
        self.finished = true;
        Ok(())
    }
}
impl Drop for ProcessGenerator {
    fn drop(&mut self) {
        self.requests.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
