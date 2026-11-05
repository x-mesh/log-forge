use chrono::{DateTime, Utc};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, io, sync::LazyLock};

static IP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("valid IP pattern"));
static LEVEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([A-Za-z]+)\]").expect("valid level pattern"));
static KAFKA_LEVEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[[^\]]+\] (INFO|WARN|ERROR|DEBUG|TRACE)\b").expect("valid Kafka level pattern")
});
static STATUS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""[A-Z]+ [^"]*" ([1-5]\d{2})"#).expect("valid status pattern"));
static TRAILING_STATUS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([1-5]\d{2})\s+\d+\s*$").expect("valid trailing status pattern")
});
static RESPONSE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d+(?:\.\d+)?)\s*ms\b").expect("valid duration pattern"));
static RESPONSE_SECONDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d+(?:\.\d+)?)\s*s\b").expect("valid seconds pattern"));
static AGENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""([^"]*Mozilla[^"]*)""#).expect("valid agent pattern"));
static TRACE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(trace_id|span_id|parent_span_id)=([0-9a-f]+|-)").expect("valid trace pattern")
});

pub(crate) fn syslog_level(priority: u64) -> Option<&'static str> {
    if priority > 191 {
        return None;
    }
    Some(match priority % 8 {
        0 => "EMERGENCY",
        1 => "ALERT",
        2 => "CRITICAL",
        3 => "ERROR",
        4 => "WARNING",
        5 => "NOTICE",
        6 => "INFO",
        _ => "DEBUG",
    })
}

#[derive(Clone)]
pub(crate) struct Options {
    pub mapping: BTreeMap<String, String>,
    pub include_raw: bool,
    pub pretty: bool,
    pub response_time_millis: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum JsonFormat {
    Standard,
    Elk,
    Splunk,
}

pub(crate) fn options(
    format: JsonFormat,
    overrides: BTreeMap<String, String>,
    include_raw: bool,
    pretty: bool,
) -> Options {
    let pairs: &[(&str, &str)] = match format {
        JsonFormat::Standard => &[],
        JsonFormat::Elk => &[("response_time", "http.response.time_ms")],
        JsonFormat::Splunk => &[
            ("timestamp", "_time"),
            ("log_type", "sourcetype"),
            ("level", "log_level"),
            ("message", "_raw"),
            ("source_ip", "src_ip"),
            ("user_agent", "http_user_agent"),
            ("status_code", "status"),
            ("response_time", "response_time"),
        ],
    };
    let mut mapping: BTreeMap<String, String> = pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    mapping.extend(overrides);
    Options {
        mapping,
        include_raw,
        pretty,
        response_time_millis: matches!(format, JsonFormat::Elk),
    }
}

fn insert_nested(root: &mut Map<String, Value>, path: &str, value: Value) -> io::Result<()> {
    let (head, tail) = path.split_once('.').unwrap_or((path, ""));
    if head.is_empty() {
        return Err(io::Error::other("empty JSON field name"));
    }
    if tail.is_empty() {
        if root.insert(head.to_owned(), value).is_some() {
            return Err(io::Error::other(format!("duplicate JSON field: {path}")));
        }
    } else {
        let nested = root.entry(head.to_owned()).or_insert_with(|| json!({}));
        insert_nested(
            nested
                .as_object_mut()
                .ok_or_else(|| io::Error::other(format!("JSON field conflict: {path}")))?,
            tail,
            value,
        )?;
    }
    Ok(())
}

pub(crate) fn record_level(kind: &str, message: &str) -> Option<String> {
    let known = |value: &str| {
        [
            "DEBUG", "INFO", "WARN", "WARNING", "ERROR", "CRITICAL", "FATAL", "TRACE",
        ]
        .contains(&value)
        .then(|| value.to_owned())
    };
    if kind == "docker" {
        if let Ok(record) = serde_json::from_str::<Value>(message) {
            let priority = record
                .get("PRIORITY")
                .and_then(|value| {
                    value
                        .as_u64()
                        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
                })
                .filter(|priority| *priority <= 7)
                .and_then(syslog_level);
            let embedded = record
                .get("log")
                .or_else(|| record.get("MESSAGE"))
                .and_then(Value::as_str)
                .and_then(|text| text.trim_start().strip_prefix('['))
                .and_then(|text| text.split_once(']'))
                .and_then(|(level, _)| known(&level.to_ascii_uppercase()));
            return Some(
                priority
                    .map(str::to_owned)
                    .or(embedded)
                    .unwrap_or_else(|| "UNKNOWN".into()),
            );
        }
        if let Some(priority) = message
            .strip_prefix('<')
            .and_then(|text| text.split_once('>'))
            .and_then(|(priority, _)| priority.parse::<u64>().ok())
            .filter(|priority| *priority <= 191)
        {
            return syslog_level(priority).map(str::to_owned);
        }
    }
    match kind {
        "ssh_auth" => Some("UNKNOWN".to_owned()),
        "nginx_error" | "apache_error" => {
            let raw = if kind == "nginx_error" {
                message
                    .split_whitespace()
                    .nth(2)
                    .and_then(|value| value.strip_prefix('[')?.strip_suffix(']'))
            } else {
                message
                    .split_once("] [")
                    .and_then(|(_, rest)| rest.split_once(']').map(|(level, _)| level))
            };
            raw.and_then(|level| match level {
                "crit" => Some("CRITICAL".to_owned()),
                "emerg" => Some("EMERGENCY".to_owned()),
                "alert" => Some("ALERT".to_owned()),
                _ => known(&level.to_ascii_uppercase()),
            })
        }
        "kafka_broker" => KAFKA_LEVEL
            .captures(message)
            .map(|value| value[1].to_owned()),
        "django" | "django_request" | "django_sql" => {
            message.split_whitespace().nth(2).and_then(known)
        }
        "kubernetes_pod" => message.split_whitespace().nth(1).and_then(known),
        "postgresql" | "postgresql_slow" => message.split_whitespace().nth(4).and_then(|value| {
            let level = value.strip_prefix('[')?.strip_suffix(']')?;
            ["LOG", "INFO", "WARNING", "ERROR", "FATAL", "PANIC"]
                .contains(&level)
                .then(|| level.to_owned())
        }),
        "mysql_error" => message
            .split_whitespace()
            .nth(1)
            .and_then(|level| match level {
                "[Note]" => Some("NOTE".to_owned()),
                "[Warning]" => Some("WARNING".to_owned()),
                "[Error]" => Some("ERROR".to_owned()),
                _ => None,
            }),
        "kubernetes_event" | "kubernetes" => {
            message
                .split_whitespace()
                .nth(1)
                .and_then(|event| match event {
                    "Normal" => Some("INFO".to_owned()),
                    "Warning" => Some("WARNING".to_owned()),
                    _ => None,
                })
        }
        "redis" => message
            .split_whitespace()
            .nth(5)
            .and_then(|symbol| match symbol {
                "." => Some("DEBUG"),
                "-" => Some("VERBOSE"),
                "*" => Some("NOTICE"),
                "#" => Some("WARNING"),
                _ => None,
            })
            .map(str::to_owned),
        "syslog" => {
            let priority = message
                .strip_prefix('<')
                .and_then(|text| text.split_once('>'))
                .and_then(|(priority, _)| priority.parse::<u64>().ok());
            let labeled = message
                .strip_prefix('[')
                .and_then(|text| text.split_once(']'))
                .map(|(label, _)| label)
                .or_else(|| message.split_whitespace().next())
                .and_then(|label| label.split_once('.'))
                .and_then(|(facility, severity)| {
                    crate::app::compat::syslog_priority(facility, severity).ok()
                })
                .map(u64::from);
            Some(
                priority
                    .or(labeled)
                    .and_then(syslog_level)
                    .unwrap_or("UNKNOWN")
                    .to_owned(),
            )
        }
        "goloop" => {
            let json = message
                .starts_with('{')
                .then(|| serde_json::from_str::<Value>(message).ok())
                .flatten();
            let value = json
                .as_ref()
                .and_then(|record| record.get("level"))
                .and_then(Value::as_str)
                .or_else(|| message.split_once('|').map(|(level, _)| level));
            value
                .and_then(|level| match level {
                    "D" => Some("DEBUG"),
                    "I" => Some("INFO"),
                    "W" => Some("WARN"),
                    "E" => Some("ERROR"),
                    "F" => Some("FATAL"),
                    _ => None,
                })
                .map(str::to_owned)
        }
        _ => None,
    }
    .or_else(|| LEVEL.captures(message).map(|value| value[1].to_uppercase()))
}

#[cfg(test)]
fn format_record(
    kind: &str,
    time: DateTime<Utc>,
    message: &str,
    options: &Options,
) -> io::Result<Vec<u8>> {
    format_record_with_level(kind, time, message, options, None)
}

pub(crate) fn format_record_with_level(
    kind: &str,
    time: DateTime<Utc>,
    message: &str,
    options: &Options,
    level: Option<&str>,
) -> io::Result<Vec<u8>> {
    let mut fields: Vec<(&str, Value)> = vec![
        ("timestamp", json!(time.to_rfc3339())),
        ("log_type", json!(kind)),
        ("message", json!(message)),
    ];
    if let Some(ip) = IP.find(crate::access_log_content(kind, message)) {
        fields.push(("source_ip", json!(ip.as_str())));
    }
    fields.push((
        "level",
        json!(
            level
                .map(str::to_ascii_uppercase)
                .or_else(|| record_level(kind, message))
                .unwrap_or_else(|| "INFO".to_owned())
        ),
    ));
    let status = if kind == "haproxy" {
        crate::haproxy::status(message)
    } else {
        STATUS
            .captures(message)
            .or_else(|| TRAILING_STATUS.captures(message))
            .and_then(|capture| capture[1].parse::<u16>().ok())
    };
    if let Some(status) = status {
        fields.push(("status_code", json!(status)));
    }
    if let Some(duration) = RESPONSE.captures(message) {
        fields.push((
            "response_time",
            json!(
                duration[1].parse::<f64>().map_err(io::Error::other)?
                    / if options.response_time_millis {
                        1.0
                    } else {
                        1000.0
                    }
            ),
        ));
    } else if let Some(duration) = RESPONSE_SECONDS.captures(message) {
        fields.push((
            "response_time",
            json!(
                duration[1].parse::<f64>().map_err(io::Error::other)?
                    * if options.response_time_millis {
                        1000.0
                    } else {
                        1.0
                    }
            ),
        ));
    }
    if let Some(agent) = AGENT.captures(message) {
        fields.push(("user_agent", json!(&agent[1])));
    }
    for capture in TRACE.captures_iter(message) {
        let key = match &capture[1] {
            "trace_id" => "trace_id",
            "span_id" => "span_id",
            "parent_span_id" => "parent_span_id",
            _ => unreachable!(),
        };
        if !fields.iter().any(|(existing, _)| *existing == key) {
            fields.push((key, json!(&capture[2])));
        }
    }
    format_fields(
        Some(json!(kind)),
        message,
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value)),
        options,
    )
}

pub(crate) fn format_entry(
    entry: &crate::models::LogEntry,
    options: &Options,
) -> io::Result<Vec<u8>> {
    let mut fields = entry.to_dict()?;
    fields.retain(|_, value| !value.is_null());
    if options.response_time_millis
        && let Some(value) = fields.get_mut("response_time")
    {
        let seconds = value
            .as_f64()
            .ok_or_else(|| io::Error::other("invalid response time"))?;
        let millis = seconds * 1000.0;
        if !millis.is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "response time is too large",
            ));
        }
        *value = json!(millis);
    }
    let kind = fields.get("log_type").cloned();
    format_fields(kind, &entry.to_string(), fields, options)
}

fn format_fields(
    kind: Option<Value>,
    message: &str,
    fields: impl IntoIterator<Item = (String, Value)>,
    options: &Options,
) -> io::Result<Vec<u8>> {
    let mut result = Map::new();
    for (name, value) in fields {
        let default = match name.as_str() {
            "timestamp" => "@timestamp",
            "log_type" => "type",
            "source_ip" => "source.ip",
            "status_code" => "http.response.status_code",
            "response_time" => "http.response.time",
            other => other,
        };
        insert_nested(
            &mut result,
            options.mapping.get(&name).map_or(default, String::as_str),
            value,
        )?;
    }
    if options.include_raw {
        insert_nested(&mut result, "raw_message", json!(message))?;
    }
    insert_nested(&mut result, "@version", json!("1"))?;
    if !result.contains_key("log_type")
        && let Some(kind) = kind
    {
        result.insert("log_type".to_owned(), kind);
    }
    if let Some(timestamp) = result.get("@timestamp").cloned()
        && !result.contains_key("timestamp")
    {
        result.insert("timestamp".to_owned(), timestamp);
    } else if let Some(timestamp) = result.get("timestamp").cloned()
        && !result.contains_key("@timestamp")
    {
        result.insert("@timestamp".to_owned(), timestamp);
    }
    let mut bytes = if options.pretty {
        serde_json::to_vec_pretty(&result)?
    } else {
        serde_json::to_vec(&result)?
    };
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kafka_json_retains_unbracketed_level() {
        let options = options(JsonFormat::Standard, BTreeMap::new(), false, false);
        let message = "[2024-01-01 00:00:00,000] ERROR [LogManager broker=1] Failed to write (kafka.log.LogManager)";
        let record = format_record(
            "kafka_broker",
            DateTime::from_timestamp(0, 0).unwrap(),
            message,
            &options,
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&record).unwrap();
        assert_eq!(value["level"], "ERROR");
    }

    #[test]
    fn preserves_levels_from_django_kubernetes_and_goloop() {
        let options = options(JsonFormat::Standard, BTreeMap::new(), false, false);
        for (kind, message, expected) in [
            (
                "django_request",
                "2024-01-01 00:00:00,000 ERROR django.request: Internal Server Error",
                "ERROR",
            ),
            (
                "django_sql",
                "2024-01-01 00:00:00,000 DEBUG django.db.backends: SELECT 1",
                "DEBUG",
            ),
            (
                "kubernetes_pod",
                "2024-01-01T00:00:00.000Z WARN probe failed",
                "WARN",
            ),
            (
                "goloop",
                "E|20240101-00:00:00.000000|1234|1|SV|handler.go:1 failed",
                "ERROR",
            ),
            ("goloop", r#"{"level":"F","msg":"failed"}"#, "FATAL"),
        ] {
            let record = format_record(
                kind,
                DateTime::from_timestamp(0, 0).unwrap(),
                message,
                &options,
            )
            .unwrap();
            let value: Value = serde_json::from_slice(&record).unwrap();
            assert_eq!(value["level"], expected, "{kind}: {message}");
        }
    }

    #[test]
    fn docker_json_reports_unknown_level_and_syslog_retains_priority() {
        let options = options(JsonFormat::Standard, BTreeMap::new(), false, false);
        let cases = [
            (
                r#"{"log":"database failed\n","stream":"stdout","time":"2024-01-01T00:00:00Z"}"#,
                "UNKNOWN",
            ),
            (
                r#"{"log":"[ERROR] failed\n","stream":"stderr","time":"2024-01-01T00:00:00Z"}"#,
                "ERROR",
            ),
            (
                "<27>1 2024-01-01T00:00:00Z server app 123 - - failed",
                "ERROR",
            ),
        ];
        for (message, expected) in cases {
            let record = format_record(
                "docker",
                DateTime::from_timestamp(0, 0).unwrap(),
                message,
                &options,
            )
            .unwrap();
            let value: Value = serde_json::from_slice(&record).unwrap();
            assert_eq!(value["level"], expected, "{message}");
        }
    }

    #[test]
    fn redis_and_syslog_levels_follow_recorded_severity() {
        let options = options(JsonFormat::Standard, BTreeMap::new(), false, false);
        for (kind, message, expected) in [
            ("redis", "123:M 01 Jan 2024 00:00:00.000 * Ready", "NOTICE"),
            (
                "redis",
                "123:M 01 Jan 2024 00:00:00.000 # Failed",
                "WARNING",
            ),
            (
                "redis",
                "123:M 01 Jan 2024 00:00:00.000 - Connected",
                "VERBOSE",
            ),
            (
                "syslog",
                "<27>1 2024-01-01T00:00:00Z server app 123 - - failed",
                "ERROR",
            ),
            (
                "syslog",
                "[daemon.warning] Jan  1 00:00:00 server app: failed",
                "WARNING",
            ),
            (
                "syslog",
                "daemon.notice Jan  1 00:00:00 server app: started",
                "NOTICE",
            ),
            ("syslog", "Jan  1 00:00:00 server app: [FAILED]", "UNKNOWN"),
        ] {
            let bytes = format_record(
                kind,
                DateTime::from_timestamp(0, 0).unwrap(),
                message,
                &options,
            )
            .unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["level"], expected, "{message}");
        }
    }

    #[test]
    fn ssh_auth_without_priority_does_not_invent_a_level() {
        let options = options(JsonFormat::Standard, BTreeMap::new(), false, false);
        let raw = "Jan  1 00:00:00 server sshd[1234]: Failed password for root from 192.0.2.1 port 5000 ssh2";
        let result = format_record(
            "ssh_auth",
            DateTime::from_timestamp(0, 0).unwrap(),
            raw,
            &options,
        )
        .unwrap();
        let record: Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(record["level"], "UNKNOWN");
        assert_eq!(record["source"]["ip"], "192.0.2.1");
    }

    #[test]
    fn extracts_seconds_and_trailing_status() {
        let options = Options {
            mapping: BTreeMap::new(),
            include_raw: false,
            pretty: false,
            response_time_millis: false,
        };
        let bytes = format_record(
            "custom",
            DateTime::from_timestamp(0, 0).unwrap(),
            "completed in 1.25 s 503 128",
            &options,
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["http"]["response"]["time"], 1.25);
        assert_eq!(value["http"]["response"]["status_code"], 503);
    }

    #[test]
    fn prefers_milliseconds_and_quoted_http_status() {
        let options = Options {
            mapping: BTreeMap::new(),
            include_raw: false,
            pretty: false,
            response_time_millis: false,
        };
        let bytes = format_record(
            "custom",
            DateTime::from_timestamp(0, 0).unwrap(),
            "\"GET / HTTP/1.1\" 201 1.25 s 250 ms 503 128",
            &options,
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["http"]["response"]["time"], 0.25);
        assert_eq!(value["http"]["response"]["status_code"], 201);
    }
}
