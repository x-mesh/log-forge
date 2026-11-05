use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Instant,
};

pub(crate) struct ControlState {
    stopped: Arc<AtomicBool>,
    paused: AtomicBool,
    data: Mutex<ControlData>,
    started: Instant,
    worker: thread::Thread,
}
#[derive(Default)]
struct ControlData {
    counts: BTreeMap<String, u64>,
    recent: VecDeque<Value>,
    error_count: u64,
}
impl ControlState {
    pub(crate) fn new(stopped: Arc<AtomicBool>) -> Self {
        Self {
            stopped,
            paused: AtomicBool::new(false),
            data: Mutex::new(ControlData::default()),
            started: Instant::now(),
            worker: thread::current(),
        }
    }
    pub(crate) fn snapshot(&self, show_logs: bool) -> io::Result<Value> {
        let data = self
            .data
            .lock()
            .map_err(|_| io::Error::other("control counts unavailable"))?;
        let generated: u64 = data.counts.values().sum();
        let state = if self.stopped.load(Ordering::SeqCst) {
            "stopping"
        } else if self.paused.load(Ordering::SeqCst) {
            "paused"
        } else {
            "running"
        };
        let mut result = json!({ "state": state, "generated": generated, "error_count": data.error_count, "types": data.counts, "pid": std::process::id(), "rate": generated as f64 / self.started.elapsed().as_secs_f64().max(0.001) });
        if show_logs {
            result["recent_logs"] = json!(data.recent);
        }
        Ok(result)
    }
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
    pub fn record_error(&self) -> io::Result<()> {
        let mut data = self
            .data
            .lock()
            .map_err(|_| io::Error::other("control counts unavailable"))?;
        data.error_count = data
            .error_count
            .checked_add(1)
            .ok_or_else(|| io::Error::other("control error count overflow"))?;
        Ok(())
    }
    pub fn record(&self, kind: &str, bytes: &[u8]) -> io::Result<()> {
        let record = std::str::from_utf8(bytes)
            .map_err(io::Error::other)?
            .trim_end_matches('\n');
        let mut data = self
            .data
            .lock()
            .map_err(|_| io::Error::other("control counts unavailable"))?;
        *data.counts.entry(kind.to_owned()).or_default() += 1;
        // Bound memory when a generator returns an oversized record.
        let mut end = record.len().min(4096);
        while !record.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = end < record.len();
        if data.recent.len() == 20 {
            data.recent.pop_front();
        }
        data.recent
            .push_back(json!({"type": kind, "message": &record[..end], "truncated": truncated}));
        Ok(())
    }
}

pub(crate) fn apply_command(command: &str, state: &ControlState) -> io::Result<bool> {
    let show_logs = match command {
        "status" => false,
        "monitor_logs" => true,
        "pause" => {
            state.paused.store(true, Ordering::SeqCst);
            state.worker.unpark();
            false
        }
        "resume" => {
            state.paused.store(false, Ordering::SeqCst);
            state.worker.unpark();
            false
        }
        "stop" => {
            state.stopped.store(true, Ordering::SeqCst);
            state.worker.unpark();
            false
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown control command",
            ));
        }
    };
    Ok(show_logs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_commands_change_state_and_request_recent_logs() {
        let stopped = Arc::new(AtomicBool::new(false));
        let state = ControlState::new(Arc::clone(&stopped));
        state.record("redis", b"test record\n").unwrap();
        assert!(!apply_command("pause", &state).unwrap());
        assert!(state.paused());
        assert_eq!(state.snapshot(false).unwrap()["state"], "paused");
        assert!(apply_command("monitor_logs", &state).unwrap());
        assert_eq!(
            state.snapshot(true).unwrap()["recent_logs"][0]["message"],
            "test record"
        );
        assert!(!apply_command("resume", &state).unwrap());
        assert_eq!(state.snapshot(false).unwrap()["state"], "running");
        assert!(apply_command("unknown", &state).is_err());
        assert!(!apply_command("stop", &state).unwrap());
        assert!(stopped.load(Ordering::SeqCst));
        assert_eq!(state.snapshot(false).unwrap()["state"], "stopping");
    }

    #[test]
    fn invalid_record_does_not_change_statistics() {
        let state = ControlState::new(Arc::new(AtomicBool::new(false)));
        assert!(state.record("redis", &[0xff]).is_err());
        let snapshot = state.snapshot(true).unwrap();
        assert_eq!(snapshot["generated"], 0);
        assert_eq!(snapshot["recent_logs"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn snapshot_reports_record_errors() {
        let state = ControlState::new(Arc::new(AtomicBool::new(false)));
        state.record_error().unwrap();
        state.record_error().unwrap();
        assert_eq!(state.snapshot(false).unwrap()["error_count"], 2);
        assert_eq!(state.snapshot(false).unwrap()["generated"], 0);
    }
}
