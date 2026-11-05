use chrono::{DateTime, Utc};
use serde_json::{Map, Number, Value, json};
use std::{collections::BTreeMap, fmt, io};

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub log_type: String,
    pub level: String,
    pub message: String,
    pub source_ip: Option<String>,
    pub user_agent: Option<String>,
    pub status_code: Option<u16>,
    pub response_time: Option<f64>,
    pub custom_fields: BTreeMap<String, Value>,
}

impl LogEntry {
    pub fn new(timestamp: DateTime<Utc>, log_type: &str, level: &str, message: &str) -> Self {
        Self {
            timestamp,
            log_type: log_type.to_owned(),
            level: level.to_owned(),
            message: message.to_owned(),
            source_ip: None,
            user_agent: None,
            status_code: None,
            response_time: None,
            custom_fields: BTreeMap::new(),
        }
    }

    pub fn to_dict(&self) -> io::Result<Map<String, Value>> {
        let response_time = if let Some(value) = self.custom_fields.get("response_time") {
            value.clone()
        } else {
            self.response_time
                .map(|value| {
                    Number::from_f64(value).map(Value::Number).ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "response time must be finite")
                    })
                })
                .transpose()?
                .unwrap_or(Value::Null)
        };
        let mut fields = Map::from_iter([
            ("timestamp".into(), json!(self.timestamp.to_rfc3339())),
            ("log_type".into(), json!(self.log_type)),
            ("level".into(), json!(self.level)),
            ("message".into(), json!(self.message)),
            ("source_ip".into(), json!(self.source_ip)),
            ("user_agent".into(), json!(self.user_agent)),
            ("status_code".into(), json!(self.status_code)),
            ("response_time".into(), response_time),
        ]);
        fields.extend(self.custom_fields.clone());
        Ok(fields)
    }

    pub fn to_json(&self) -> io::Result<String> {
        let mut fields = self.to_dict()?;
        for name in ["source_ip", "user_agent", "status_code", "response_time"] {
            if fields.get(name).is_some_and(Value::is_null)
                && !self.custom_fields.contains_key(name)
            {
                fields.remove(name);
            }
        }
        serde_json::to_string(&fields).map_err(io::Error::other)
    }
}

impl fmt::Display for LogEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ip) = &self.source_ip {
            write!(f, "{ip} - ")?;
        }
        write!(
            f,
            "{} [{}] {}",
            self.timestamp.to_rfc3339(),
            self.level,
            self.message
        )?;
        if let Some(status) = self.status_code {
            write!(f, " - Status: {status}")?;
        }
        if let Some(time) = self.response_time {
            write!(f, " - Time: {time:.3}s")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct GenerationStatistics {
    pub total_logs_generated: u64,
    pub logs_by_type: BTreeMap<String, u64>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub errors_count: u64,
    pub generation_rate: f64,
}

impl GenerationStatistics {
    pub fn add_log(&mut self, log_type: &str) {
        self.total_logs_generated += 1;
        *self.logs_by_type.entry(log_type.to_owned()).or_default() += 1;
    }

    pub fn add_error(&mut self) {
        self.errors_count += 1;
    }

    pub fn calculate_rate(&mut self) -> f64 {
        let duration = self
            .start_time
            .zip(self.end_time)
            .and_then(|(start, end)| end.signed_duration_since(start).to_std().ok())
            .map(|duration| duration.as_secs_f64())
            .filter(|seconds| *seconds > 0.0);
        self.generation_rate =
            duration.map_or(0.0, |seconds| self.total_logs_generated as f64 / seconds);
        self.generation_rate
    }

    pub fn to_dict(&self) -> Map<String, Value> {
        Map::from_iter([
            (
                "total_logs_generated".into(),
                json!(self.total_logs_generated),
            ),
            ("logs_by_type".into(), json!(self.logs_by_type)),
            (
                "start_time".into(),
                json!(self.start_time.map(|time| time.to_rfc3339())),
            ),
            (
                "end_time".into(),
                json!(self.end_time.map(|time| time.to_rfc3339())),
            ),
            ("errors_count".into(), json!(self.errors_count)),
            ("generation_rate".into(), json!(self.generation_rate)),
        ])
    }
}
