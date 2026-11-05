use regex::Regex;
use serde::Serialize;
use std::{collections::BTreeMap, io, net::Ipv4Addr};

const ACCESS: &str =
    r#"^\S+ \S+ \S+ \[[^\]]+\] "[A-Z]+ [^"]+" \d{3} (?:\d+|-)(?: "[^"]*" "[^"]*")?$"#;
const ISO: &str = r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z";

#[derive(Serialize)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub log_entry: String,
    pub log_type: String,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub pattern_matches: BTreeMap<String, bool>,
    pub field_completeness: BTreeMap<String, bool>,
    pub quality_score: f64,
}

#[derive(Default, Serialize)]
pub struct ValidationStatistics {
    pub total_validated: u64,
    pub valid_logs: u64,
    pub invalid_logs: u64,
    pub validation_by_type: BTreeMap<String, TypeValidationCounts>,
    pub common_errors: BTreeMap<String, u64>,
    pub average_quality_score: f64,
    pub quality_distribution: BTreeMap<String, u64>,
    #[serde(skip)]
    quality_sum: f64,
    #[serde(skip)]
    error_order: BTreeMap<String, usize>,
}

#[derive(Default, Serialize)]
pub struct TypeValidationCounts {
    pub valid: u64,
    pub invalid: u64,
}

#[derive(Serialize)]
pub struct ValidationSummary<'a> {
    pub total_validated: u64,
    pub valid_logs: u64,
    pub invalid_logs: u64,
    pub success_rate: f64,
    pub average_quality_score: f64,
    pub validation_by_type: &'a BTreeMap<String, TypeValidationCounts>,
    pub common_errors: BTreeMap<&'a str, u64>,
    pub quality_distribution: &'a BTreeMap<String, u64>,
}

impl ValidationStatistics {
    pub fn add_result(&mut self, result: &ValidationResult) {
        self.total_validated += 1;
        let counts = self
            .validation_by_type
            .entry(result.log_type.clone())
            .or_default();
        if result.is_valid {
            self.valid_logs += 1;
            counts.valid += 1;
        } else {
            self.invalid_logs += 1;
            counts.invalid += 1;
        }
        for error in &result.errors {
            if !self.error_order.contains_key(error) {
                self.error_order
                    .insert(error.clone(), self.error_order.len());
            }
            *self.common_errors.entry(error.clone()).or_default() += 1;
        }
        self.quality_sum += result.quality_score;
        self.average_quality_score = self.quality_sum / self.total_validated as f64;
        if self.quality_distribution.is_empty() {
            for name in [
                "excellent (90-100)",
                "good (70-89)",
                "fair (50-69)",
                "poor (0-49)",
            ] {
                self.quality_distribution.insert(name.to_owned(), 0);
            }
        }
        let bucket = if result.quality_score >= 90.0 {
            "excellent (90-100)"
        } else if result.quality_score >= 70.0 {
            "good (70-89)"
        } else if result.quality_score >= 50.0 {
            "fair (50-69)"
        } else {
            "poor (0-49)"
        };
        *self
            .quality_distribution
            .entry(bucket.to_owned())
            .or_default() += 1;
    }

    pub fn success_rate(&self) -> f64 {
        if self.total_validated == 0 {
            0.0
        } else {
            self.valid_logs as f64 / self.total_validated as f64
        }
    }

    pub fn type_success_rate(&self, kind: &str) -> f64 {
        self.validation_by_type.get(kind).map_or(0.0, |counts| {
            let total = counts.valid + counts.invalid;
            if total == 0 {
                0.0
            } else {
                counts.valid as f64 / total as f64
            }
        })
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn summary(&self) -> ValidationSummary<'_> {
        let mut errors: Vec<_> = self.common_errors.iter().collect();
        errors.sort_by(|(name_a, count_a), (name_b, count_b)| {
            count_b.cmp(count_a).then_with(|| {
                self.error_order
                    .get(*name_a)
                    .cmp(&self.error_order.get(*name_b))
            })
        });
        ValidationSummary {
            total_validated: self.total_validated,
            valid_logs: self.valid_logs,
            invalid_logs: self.invalid_logs,
            success_rate: self.success_rate(),
            average_quality_score: self.average_quality_score,
            validation_by_type: &self.validation_by_type,
            common_errors: errors
                .into_iter()
                .take(10)
                .map(|(name, count)| (name.as_str(), *count))
                .collect(),
            quality_distribution: &self.quality_distribution,
        }
    }
}

pub struct LogValidator {
    kind: String,
    pattern: Regex,
    custom_patterns: BTreeMap<String, Regex>,
    required_fields: BTreeMap<String, Regex>,
    ip: Regex,
    status: Regex,
}

impl LogValidator {
    pub fn log_type(&self) -> &str {
        &self.kind
    }

    pub fn pattern(&self) -> &str {
        self.pattern.as_str()
    }

    pub fn new(kind: &str) -> io::Result<Self> {
        let pattern = match kind {
            "nginx_access" => ACCESS.to_owned(),
            "apache_access" => format!(r"^(?:\S+ )?{}", ACCESS.trim_start_matches('^')),
            "nginx_error" => r"^\d{4}/\d{2}/\d{2} \d{2}:\d{2}:\d{2} \[\w+\] \d+#\d+: .+$".to_owned(),
            "apache_error" => r"^\[[^\]]+\] \[\w+\] \[[^\]]+\] \[pid \d+\] \[client [\d.]+\] .+$".to_owned(),
            "syslog" => r"^(?:<\d+>|\[\w+\.\w+\]|\w+\.\w+ )?(?:\w{3}\s+\d{1,2} \d{2}:\d{2}:\d{2} \S+ \S+:.+|1 \S+ \S+ \S+ \S+ \S+ \S+ .+)$".to_owned(),
            "fastapi" => r"^(?:\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} \[\w+\] (?:\w{8}: )?.+|\w+ \d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} - .+)$".to_owned(),
            "django_request" => r#"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} (?:\w+ [\w.]+: "[A-Z]+ [^"]+" \d+ \[\d+ms\] \(user: [^,]+, ip: [\d.]+\)|ERROR django\.request: Internal Server Error: [^\n]+\nTraceback \(most recent call last\):\n(?s:.+))$"#.to_owned(),
            "django" => r#"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} (?:\w+ [\w.]+: .+|ERROR django\.request: [^\n]+\n(?s:.+))$"#.to_owned(),
            "django_sql" => r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} \w+ [\w.]+: \([0-9.]+\) .+; args=.+$".to_owned(),
            "docker" => format!(r#"^(?:{ISO} \[\w+\] [^:]+: .+|<\d+>1 {ISO} \S+ \S+ \S+ - - .+|\{{.*\}})$"#),
            "kubernetes_pod" => format!(r"^{ISO} \w+ .+$"),
            "kubernetes_event" | "kubernetes" => format!(r"^{ISO} (?:Normal|Warning) \w+ \w+/\S+ .+$"),
            "mysql_query" => r"^\d{6} \d{2}:\d{2}:\d{2}\s+\d+\s+\w+\s+.+$".to_owned(),
            "mysql_error" => format!(r"^{ISO} \[\w+\] .+$"),
            "mysql_slow" => format!(r"^# Time: {ISO}\n# User@Host: [^\n]+\n# Thread_id: \d+\s+Schema: [^\n]*\n# Query_time: [\d.]+\s+Lock_time: [\d.]+\s+Rows_sent: \d+\s+Rows_examined: \d+\nSET timestamp=\d+;\n.+;$"),
            "postgresql" => r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+ UTC \[\d+\]: \[\w+\] .+$".to_owned(),
            "postgresql_slow" => r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+ UTC \[\d+\]: \[LOG\] duration: [\d.]+ ms statement: .+$".to_owned(),
            "goloop" => r"^(?:[DIWEF]\|\d{8}-\d{2}:\d{2}:\d{2}\.\d{6}\|[0-9a-f]{4}\|(?:\d+|-)\|[A-Za-z0-9]+(?:\|[A-Z0-9]+)?\|[a-z0-9_]+\.go:\d+ .+|\{.*\})$".to_owned(),
            "redis" => r"^\d+:[A-Z] \d{2} \w{3} \d{4} \d{2}:\d{2}:\d{2}\.\d+ [.*#-] .+$".to_owned(),
            "ssh_auth" => r"^\w{3}\s+\d{1,2} \d{2}:\d{2}:\d{2} \S+ sshd\[\d+\]: .+$".to_owned(),
            "kafka_broker" => r"^\[\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}\] (?:INFO|WARN|ERROR|DEBUG|TRACE) .+ \([\w.$]+\)$".to_owned(),
            "haproxy" => crate::haproxy::PATTERN.to_owned(),
            "microservice" => format!(r"^{ISO} \[\w+\] service=\S+ operation=\S+ .+$"),
            "scenario" => r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2}) \[\w+\] scenario=.+ pattern=[A-Za-z0-9_-]+ .+$".to_owned(),
            "error_library" => r"^\S+ \[\w+\] category=\S+ pattern=\S+ .+$".to_owned(),
            "error_pattern" => format!(r"^{ISO} \[\w+\] category=\S+ .+$"),
            "json" => r"^\{.*\}$".to_owned(),
            _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported log type: {kind}"))),
        };
        Self::with_pattern(kind, &pattern)
    }

    pub fn with_pattern(kind: &str, pattern: &str) -> io::Result<Self> {
        let compile = |pattern: &str| Regex::new(pattern).map_err(io::Error::other);
        Ok(Self {
            kind: kind.to_owned(),
            pattern: compile(pattern)?,
            custom_patterns: BTreeMap::new(),
            required_fields: BTreeMap::new(),
            ip: compile(r"\b(?:\d{1,3}\.){3}\d{1,3}\b")?,
            status: compile(r#""[^"]+" (\d{3})"#)?,
        })
    }

    pub fn add_pattern(&mut self, name: &str, pattern: &str) -> Result<(), regex::Error> {
        let pattern = Regex::new(pattern)?;
        self.custom_patterns.insert(name.to_owned(), pattern);
        Ok(())
    }

    pub fn add_required_field(&mut self, name: &str, pattern: &str) -> Result<(), regex::Error> {
        let pattern = Regex::new(pattern)?;
        self.required_fields.insert(name.to_owned(), pattern);
        Ok(())
    }

    pub fn validate(&self, record: &str) -> ValidationResult {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut fields = BTreeMap::new();
        let pattern_matches: BTreeMap<_, _> = self
            .custom_patterns
            .iter()
            .map(|(name, pattern)| (name.clone(), pattern.is_match(record)))
            .collect();
        if record.trim().is_empty() {
            errors.push("Empty log entry".to_string());
        } else if !self.pattern.is_match(record)
            && !pattern_matches.values().any(|matched| *matched)
        {
            errors.push("Log entry does not match the expected format".to_string());
        }
        if self.kind == "json"
            || (["docker", "goloop"].contains(&self.kind.as_str()) && record.starts_with('{'))
        {
            match serde_json::from_str::<Value>(record) {
                Ok(value)
                    if self.kind == "docker"
                        && value["MESSAGE"].as_str().is_none()
                        && (value["log"].as_str().is_none()
                            || !["stdout", "stderr"]
                                .contains(&value["stream"].as_str().unwrap_or(""))
                            || value["time"].as_str().is_none()) =>
                {
                    errors.push("Invalid Docker JSON fields".to_string())
                }
                Ok(value)
                    if self.kind == "docker"
                        && value["MESSAGE"].is_string()
                        && [
                            "CONTAINER_NAME",
                            "CONTAINER_ID",
                            "CONTAINER_ID_FULL",
                            "SYSLOG_IDENTIFIER",
                            "__REALTIME_TIMESTAMP",
                        ]
                        .iter()
                        .any(|field| value[*field].as_str().is_none()) =>
                {
                    errors.push("Invalid journald JSON fields".to_string())
                }
                Ok(value)
                    if self.kind == "goloop"
                        && ["level", "timestamp", "module", "msg"]
                            .iter()
                            .any(|field| value[*field].as_str().is_none()) =>
                {
                    errors.push("Invalid Goloop JSON fields".to_string())
                }
                Ok(_) => (),
                Err(_) => errors.push("Invalid JSON".to_string()),
            }
        }
        if matches!(self.kind.as_str(), "nginx_access" | "apache_access") {
            let ip = self.ip.find(crate::access_log_content(&self.kind, record));
            fields.insert("ip".into(), ip.is_some());
            if ip.is_some_and(|v| v.as_str().parse::<Ipv4Addr>().is_err()) {
                warnings.push("Invalid IP address".into());
            }
            let status = self
                .status
                .captures(record)
                .and_then(|capture| capture[1].parse::<u16>().ok());
            fields.insert("status_code".into(), status.is_some());
            if status.is_some_and(|code| !(100..=599).contains(&code)) {
                warnings.push("Invalid HTTP status code".into());
            }
            fields.insert(
                "timestamp".into(),
                record.contains('[') && record.contains(']'),
            );
            fields.insert("request".into(), record.contains('"'));
        }
        for (name, pattern) in &self.required_fields {
            let present = pattern.is_match(record);
            fields.insert(name.clone(), present);
            if !present {
                warnings.push(format!("Missing or invalid {name} field"));
            }
        }
        if !record.is_empty() && record.len() < 10 {
            warnings.push("Log entry seems too short".into());
        }
        if record.len() > 10_000 {
            warnings.push("Log entry is very long".into());
        }
        if record
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            warnings.push("Contains non-printable characters".into());
        }
        let is_valid = errors.is_empty();
        let quality_score = if is_valid {
            (100.0 - warnings.len() as f64 * 10.0).max(0.0)
        } else {
            0.0
        };
        ValidationResult {
            is_valid,
            log_entry: record.to_owned(),
            log_type: self.kind.clone(),
            errors,
            warnings,
            pattern_matches,
            field_completeness: fields,
            quality_score,
        }
    }
}
use serde_json::Value;
