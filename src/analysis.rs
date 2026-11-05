use crate::validation::LogValidator;
use rand::{Rng, SeedableRng, rngs::StdRng, seq::SliceRandom};
use regex::Regex;
use serde::Serialize;
use std::{collections::BTreeMap, io, sync::LazyLock};

static IP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("valid IP pattern"));
static STATUS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""[A-Z]+ [^"]*" ([1-5]\d{2})"#).expect("valid status pattern"));
static LEVEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(DEBUG|INFO|WARN|WARNING|ERROR|CRITICAL|FATAL|NOTICE)\b")
        .expect("valid level pattern")
});
static AGENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""([^"]*Mozilla[^"]*)""#).expect("valid agent pattern"));
static HOUR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:\d{2}/[A-Za-z]{3}/\d{4}:|\d{4}-\d{2}-\d{2}[T ]|[A-Z][a-z]{2}\s+\d{1,2}\s+|\d{8}-|\d{1,2} [A-Z][a-z]{2} \d{4} )(\d{2}):\d{2}:\d{2}").expect("valid timestamp pattern")
});
static ERROR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\berror[:\s]+([^,\n]+)").expect("valid error pattern"));
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+").expect("valid number pattern"));

#[derive(Default, Serialize)]
pub struct LogStatistics {
    pub total_logs: u64,
    pub valid_logs: u64,
    pub invalid_logs: u64,
    pub log_types: BTreeMap<String, u64>,
    pub log_levels: BTreeMap<String, u64>,
    pub status_codes: BTreeMap<String, u64>,
    pub ip_addresses: BTreeMap<String, u64>,
    pub user_agents: BTreeMap<String, u64>,
    pub time_distribution: BTreeMap<String, u64>,
    pub error_patterns: BTreeMap<String, u64>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub authentication_events: BTreeMap<String, u64>,
    pub field_completeness: BTreeMap<String, f64>,
    pub average_log_length: f64,
    pub average_quality_score: f64,
    pub quality_metrics: BTreeMap<String, f64>,
}

impl LogStatistics {
    pub fn to_dict(&self) -> serde_json::Value {
        let top = |values: &BTreeMap<String, u64>, limit: usize| {
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|(name_a, count_a), (name_b, count_b)| {
                count_b.cmp(count_a).then_with(|| name_a.cmp(name_b))
            });
            entries
                .into_iter()
                .take(limit)
                .map(|(name, count)| (name.clone(), *count))
                .collect::<BTreeMap<_, _>>()
        };
        serde_json::json!({
            "total_logs": self.total_logs,
            "log_types": self.log_types,
            "log_levels": self.log_levels,
            "status_codes": self.status_codes,
            "top_ip_addresses": top(&self.ip_addresses, 10),
            "top_user_agents": top(&self.user_agents, 5),
            "time_distribution": self.time_distribution,
            "error_patterns": top(&self.error_patterns, 10),
            "field_completeness": self.field_completeness,
            "average_log_length": self.average_log_length,
            "quality_metrics": self.quality_metrics,
        })
    }
}

pub struct LogAnalyzer {
    kind: String,
    validator: LogValidator,
    stats: LogStatistics,
    field_counts: BTreeMap<String, u64>,
    length: u64,
    quality: f64,
    completeness: f64,
}

fn count(map: &mut BTreeMap<String, u64>, value: &str) {
    *map.entry(value.to_owned()).or_default() += 1;
}

fn header_level(record: &str) -> Option<String> {
    record
        .split_whitespace()
        .nth(1)?
        .strip_prefix('[')?
        .strip_suffix(']')
        .map(str::to_ascii_uppercase)
}

fn ssh_event(record: &str) -> Option<&'static str> {
    let (_, message) = record.split_once("]: ")?;
    if message.starts_with("Failed password ") {
        Some("failed")
    } else if message.starts_with("Accepted password ")
        || message.starts_with("Accepted publickey ")
    {
        Some("accepted")
    } else if message.starts_with("Invalid user ") {
        Some("invalid_user")
    } else if message.starts_with("Connection closed ") {
        Some("connection_closed")
    } else {
        None
    }
}

impl LogAnalyzer {
    pub fn new(kind: &str) -> io::Result<Self> {
        Ok(Self::with_validator(LogValidator::new(kind)?))
    }
    pub fn with_validator(validator: LogValidator) -> Self {
        let kind = validator.log_type().to_owned();
        let mut stats = LogStatistics::default();
        stats.log_types.insert(kind.clone(), 0);
        Self {
            kind,
            validator,
            stats,
            field_counts: BTreeMap::new(),
            length: 0,
            quality: 0.0,
            completeness: 0.0,
        }
    }
    pub fn observe(&mut self, record: &str) {
        let result = self.validator.validate(record);
        self.observe_result(record, &result);
    }

    fn observe_result(&mut self, record: &str, result: &crate::validation::ValidationResult) {
        self.stats.total_logs += 1;
        count(&mut self.stats.log_types, &self.kind);
        self.length += record.chars().count() as u64;
        if result.is_valid {
            self.stats.valid_logs += 1;
        } else {
            self.stats.invalid_logs += 1;
        }
        self.quality += result.quality_score;
        if !result.field_completeness.is_empty() {
            self.completeness += result
                .field_completeness
                .values()
                .filter(|present| **present)
                .count() as f64
                / result.field_completeness.len() as f64;
        }
        for (field, present) in &result.field_completeness {
            let count = self.field_counts.entry(field.clone()).or_default();
            if *present {
                *count += 1;
            }
        }
        if let Some(ip) = IP.find(crate::access_log_content(&self.kind, record)) {
            count(&mut self.stats.ip_addresses, ip.as_str());
        }
        if self.kind == "haproxy" && result.is_valid {
            if let Some(status) = crate::haproxy::status(record) {
                count(&mut self.stats.status_codes, &status.to_string());
            }
        } else if let Some(status) = STATUS.captures(record) {
            count(&mut self.stats.status_codes, &status[1]);
        }
        if matches!(
            self.kind.as_str(),
            "scenario" | "error_library" | "error_pattern" | "microservice"
        ) && result.is_valid
        {
            if let Some(level) = header_level(record) {
                count(&mut self.stats.log_levels, &level);
            }
        } else if matches!(
            self.kind.as_str(),
            "redis"
                | "goloop"
                | "syslog"
                | "kubernetes_event"
                | "kubernetes"
                | "mysql_error"
                | "postgresql"
                | "postgresql_slow"
                | "kubernetes_pod"
                | "kafka_broker"
                | "nginx_error"
                | "apache_error"
                | "docker"
        ) && result.is_valid
        {
            if let Some(level) = crate::json_output::record_level(&self.kind, record)
                && level != "UNKNOWN"
            {
                count(&mut self.stats.log_levels, &level);
            }
        } else if self.kind != "ssh_auth"
            && !matches!(
                self.kind.as_str(),
                "redis"
                    | "goloop"
                    | "syslog"
                    | "scenario"
                    | "error_library"
                    | "error_pattern"
                    | "microservice"
                    | "kubernetes_event"
                    | "kubernetes"
                    | "mysql_error"
                    | "postgresql"
                    | "postgresql_slow"
                    | "kubernetes_pod"
                    | "kafka_broker"
                    | "nginx_error"
                    | "apache_error"
                    | "docker"
            )
            && let Some(level) = LEVEL.captures(record)
        {
            count(&mut self.stats.log_levels, &level[1].to_uppercase());
        }
        if self.kind == "ssh_auth"
            && result.is_valid
            && let Some(event) = ssh_event(record)
        {
            count(&mut self.stats.authentication_events, event);
        }
        if let Some(agent) = AGENT.captures(record) {
            count(&mut self.stats.user_agents, &agent[1]);
        }
        if let Some(hour) = HOUR.captures(record) {
            count(&mut self.stats.time_distribution, &hour[1]);
        }
        if let Some(error) = ERROR.captures(record) {
            count(
                &mut self.stats.error_patterns,
                &NUMBER.replace_all(error[1].trim(), "<num>"),
            );
        }
    }
    pub fn statistics(&self) -> LogStatistics {
        let count = self.stats.total_logs as f64;
        let mut stats = LogStatistics {
            total_logs: self.stats.total_logs,
            valid_logs: self.stats.valid_logs,
            invalid_logs: self.stats.invalid_logs,
            log_types: self.stats.log_types.clone(),
            log_levels: self.stats.log_levels.clone(),
            status_codes: self.stats.status_codes.clone(),
            ip_addresses: self.stats.ip_addresses.clone(),
            user_agents: self.stats.user_agents.clone(),
            time_distribution: self.stats.time_distribution.clone(),
            error_patterns: self.stats.error_patterns.clone(),
            authentication_events: self.stats.authentication_events.clone(),
            ..Default::default()
        };
        if count > 0.0 {
            stats.average_log_length = self.length as f64 / count;
            stats.average_quality_score = self.quality / count;
            stats.quality_metrics = BTreeMap::from([
                (
                    "validation_rate".to_owned(),
                    self.stats.valid_logs as f64 / count,
                ),
                (
                    "error_rate".to_owned(),
                    self.stats.invalid_logs as f64 / count,
                ),
                (
                    "average_quality_score".to_owned(),
                    stats.average_quality_score,
                ),
                ("completeness_score".to_owned(), self.completeness / count),
            ]);
            stats.field_completeness = self
                .field_counts
                .iter()
                .map(|(name, value)| (name.clone(), *value as f64 / count))
                .collect();
        }
        stats
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingStrategy {
    Random,
    Systematic,
    Stratified,
    QualityBased,
}

#[derive(Serialize)]
pub struct LogSample {
    pub log_type: String,
    pub sample_size: usize,
    pub logs: Vec<String>,
    pub strategy: SamplingStrategy,
    pub original_count: usize,
    pub sampling_ratio: f64,
    pub timestamp: String,
    pub metadata: SampleMetadata,
}

impl LogSample {
    pub fn to_dict(&self) -> serde_json::Value {
        let metadata = if self.original_count == 0 {
            serde_json::json!({})
        } else {
            serde_json::json!(self.metadata)
        };
        serde_json::json!({"log_type": self.log_type, "sample_size": self.sample_size, "logs": self.logs, "timestamp": self.timestamp, "metadata": metadata})
    }
}

#[derive(Serialize)]
pub struct SampleMetadata {
    pub strategy: SamplingStrategy,
    pub original_count: usize,
    pub sampling_ratio: f64,
}

fn stratum(record: &str, kind: &str) -> &'static str {
    if kind == "ssh_auth" {
        return ssh_event(record).unwrap_or("other");
    }
    if kind == "haproxy" {
        return match crate::haproxy::status(record) {
            Some(100..=299) => "success",
            Some(300..=399) => "redirect",
            Some(400..=499) => "client_error",
            Some(500..=599) => "server_error",
            _ => "other",
        };
    }
    if !matches!(
        kind,
        "scenario"
            | "error_library"
            | "error_pattern"
            | "microservice"
            | "kubernetes_event"
            | "kubernetes"
            | "mysql_error"
            | "postgresql"
            | "postgresql_slow"
            | "syslog"
            | "redis"
            | "goloop"
            | "kubernetes_pod"
            | "kafka_broker"
            | "nginx_error"
            | "apache_error"
            | "docker"
    ) && let Some(status) = STATUS
        .captures(record)
        .and_then(|c| c[1].parse::<u16>().ok())
    {
        return match status {
            100..=299 => "success",
            300..=399 => "redirect",
            400..=499 => "client_error",
            _ => "server_error",
        };
    }
    let level = if matches!(
        kind,
        "scenario" | "error_library" | "error_pattern" | "microservice"
    ) {
        header_level(record)
    } else if matches!(
        kind,
        "redis"
            | "goloop"
            | "syslog"
            | "kubernetes_event"
            | "kubernetes"
            | "mysql_error"
            | "postgresql"
            | "postgresql_slow"
            | "kubernetes_pod"
            | "kafka_broker"
            | "nginx_error"
            | "apache_error"
            | "docker"
    ) {
        crate::json_output::record_level(kind, record)
    } else {
        LEVEL
            .captures(record)
            .map(|capture| capture[1].to_ascii_uppercase())
    };
    if let Some(level) = level {
        return match level.as_str() {
            "TRACE" | "DEBUG" | "VERBOSE" | "INFO" | "NOTICE" | "NOTE" | "LOG" => "info",
            "WARN" | "WARNING" => "warning",
            "UNKNOWN" => "other",
            _ => "error",
        };
    }
    "other"
}

pub fn generate_samples(
    records: &[String],
    kind: &str,
    size: usize,
    strategy: SamplingStrategy,
    seed: u64,
) -> io::Result<LogSample> {
    let validator = LogValidator::new(kind)?;
    let size = size.min(records.len());
    let mut rng = StdRng::seed_from_u64(seed);
    let indices = match strategy {
        SamplingStrategy::Random => {
            let mut indices: Vec<usize> = (0..records.len()).collect();
            indices.shuffle(&mut rng);
            indices.truncate(size);
            indices
        }
        SamplingStrategy::Systematic if size > 0 => {
            let interval = records.len() / size;
            let start = rng.gen_range(0..interval);
            (0..size).map(|index| start + index * interval).collect()
        }
        SamplingStrategy::Systematic => Vec::new(),
        SamplingStrategy::Stratified | SamplingStrategy::QualityBased => {
            let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
            for (index, record) in records.iter().enumerate() {
                let key = match strategy {
                    SamplingStrategy::Stratified => stratum(record, kind).to_owned(),
                    SamplingStrategy::QualityBased => {
                        ((validator.validate(record).quality_score as u32 / 25).min(3)).to_string()
                    }
                    _ => unreachable!(),
                };
                groups.entry(key).or_default().push(index);
            }
            for group in groups.values_mut() {
                group.shuffle(&mut rng);
            }
            let mut selected = Vec::with_capacity(size);
            while selected.len() < size {
                for group in groups.values_mut() {
                    if let Some(index) = group.pop() {
                        selected.push(index);
                    }
                    if selected.len() == size {
                        break;
                    }
                }
            }
            selected
        }
    };
    let sampling_ratio = if records.is_empty() {
        0.0
    } else {
        size as f64 / records.len() as f64
    };
    Ok(LogSample {
        log_type: kind.to_owned(),
        sample_size: size,
        logs: indices
            .into_iter()
            .map(|index| records[index].clone())
            .collect(),
        strategy,
        original_count: records.len(),
        sampling_ratio,
        timestamp: chrono::Utc::now().to_rfc3339(),
        metadata: SampleMetadata {
            strategy,
            original_count: records.len(),
            sampling_ratio,
        },
    })
}

#[derive(Serialize)]
pub struct DistributionDifference {
    pub first_proportion: f64,
    pub second_proportion: f64,
    pub difference: f64,
}
#[derive(Serialize)]
pub struct LogComparison {
    pub first: LogStatistics,
    pub second: LogStatistics,
    pub status_differences: BTreeMap<String, DistributionDifference>,
    pub level_differences: BTreeMap<String, DistributionDifference>,
}

impl LogComparison {
    pub fn to_dict(&self) -> serde_json::Value {
        let mut differences = serde_json::Map::new();
        if self.first.total_logs > 0 && self.second.total_logs > 0 {
            let summarize =
                |first: &BTreeMap<String, u64>,
                 second: &BTreeMap<String, u64>,
                 values: &BTreeMap<String, DistributionDifference>| {
                    let significant: BTreeMap<_, _> = values.iter().map(|(name, value)| (name, serde_json::json!({"set1_proportion": value.first_proportion, "set2_proportion": value.second_proportion, "difference": value.difference}))).collect();
                    let union: std::collections::BTreeSet<_> =
                        first.keys().chain(second.keys()).collect();
                    let common = first.keys().filter(|key| second.contains_key(*key)).count();
                    serde_json::json!({"significant_differences": significant, "total_keys": union.len(), "common_keys": common})
                };
            if !self.first.status_codes.is_empty() && !self.second.status_codes.is_empty() {
                differences.insert(
                    "status_codes".into(),
                    summarize(
                        &self.first.status_codes,
                        &self.second.status_codes,
                        &self.status_differences,
                    ),
                );
            }
            if !self.first.log_levels.is_empty() && !self.second.log_levels.is_empty() {
                differences.insert(
                    "log_levels".into(),
                    summarize(
                        &self.first.log_levels,
                        &self.second.log_levels,
                        &self.level_differences,
                    ),
                );
            }
            differences.insert("quality_metrics".into(), serde_json::json!({"set1": self.first.quality_metrics, "set2": self.second.quality_metrics}));
        }
        serde_json::json!({"set1_stats": self.first.to_dict(), "set2_stats": self.second.to_dict(), "differences": differences, "similarities": {}, "recommendations": []})
    }
}

fn compare_distribution(
    first: &BTreeMap<String, u64>,
    second: &BTreeMap<String, u64>,
) -> BTreeMap<String, DistributionDifference> {
    let a: u64 = first.values().sum();
    let b: u64 = second.values().sum();
    if a == 0 || b == 0 {
        return BTreeMap::new();
    }
    let mut differences = BTreeMap::new();
    for key in first.keys().chain(second.keys()) {
        let x = *first.get(key).unwrap_or(&0) as f64 / a as f64;
        let y = *second.get(key).unwrap_or(&0) as f64 / b as f64;
        let delta = (x - y).abs();
        if delta > 0.05 {
            differences.insert(
                key.clone(),
                DistributionDifference {
                    first_proportion: x,
                    second_proportion: y,
                    difference: delta,
                },
            );
        }
    }
    differences
}

pub fn compare_log_sets(
    first: &[String],
    second: &[String],
    kind: &str,
) -> io::Result<LogComparison> {
    let mut first_analyzer = LogAnalyzer::new(kind)?;
    let mut second_analyzer = LogAnalyzer::new(kind)?;
    for record in first {
        first_analyzer.observe(record);
    }
    for record in second {
        second_analyzer.observe(record);
    }
    let first = first_analyzer.statistics();
    let second = second_analyzer.statistics();
    let status_differences = compare_distribution(&first.status_codes, &second.status_codes);
    let level_differences = compare_distribution(&first.log_levels, &second.log_levels);
    Ok(LogComparison {
        first,
        second,
        status_differences,
        level_differences,
    })
}

#[derive(Serialize)]
pub struct QualityReport {
    pub overall_score: f64,
    pub total_logs_analyzed: usize,
    pub validation_summary: ValidationSummary,
    pub validation_results: Vec<crate::validation::ValidationResult>,
    pub statistics: LogStatistics,
    pub recommendations: Vec<String>,
    pub issues_found: Vec<String>,
    pub timestamp: String,
}

impl QualityReport {
    pub fn to_dict(&self) -> serde_json::Value {
        serde_json::json!({
            "overall_score": self.overall_score,
            "total_logs_analyzed": self.total_logs_analyzed,
            "validation_summary": self.validation_summary,
            "statistics": self.statistics.to_dict(),
            "recommendations": self.recommendations,
            "issues_found": self.issues_found,
            "timestamp": self.timestamp,
        })
    }
}

#[derive(Serialize)]
pub struct ValidationSummary {
    pub valid_logs: u64,
    pub invalid_logs: u64,
    pub average_quality_score: f64,
}

pub fn quality_report(records: &[String], kind: &str) -> io::Result<QualityReport> {
    let mut analyzer = LogAnalyzer::new(kind)?;
    let mut validation_results = Vec::with_capacity(records.len());
    let mut errors = BTreeMap::new();
    for record in records {
        let result = analyzer.validator.validate(record);
        analyzer.observe_result(record, &result);
        for error in &result.errors {
            count(&mut errors, error);
        }
        validation_results.push(result);
    }
    let statistics = analyzer.statistics();
    let valid_rate = if records.is_empty() {
        0.0
    } else {
        statistics.valid_logs as f64 / records.len() as f64
    };
    let overall_score = valid_rate * 60.0 + statistics.average_quality_score * 0.4;
    let mut recommendations = Vec::new();
    if !records.is_empty() && valid_rate < 0.8 {
        recommendations.push("Improve log format consistency".to_owned());
    }
    if statistics
        .field_completeness
        .values()
        .any(|fraction| *fraction < 0.9)
    {
        recommendations.push("Ensure required fields are present".to_owned());
    }
    let error_levels: u64 = ["ERROR", "CRITICAL", "FATAL", "PANIC", "ALERT", "EMERGENCY"]
        .iter()
        .map(|level| statistics.log_levels.get(*level).copied().unwrap_or(0))
        .sum();
    if !records.is_empty() && error_levels as f64 / records.len() as f64 > 0.1 {
        recommendations.push("High error rate detected - investigate error patterns".to_owned());
    }
    if records.len() > 100 && statistics.ip_addresses.len() < 5 {
        recommendations
            .push("Low IP address diversity - consider adding more varied source IPs".to_owned());
    }
    let status_total: u64 = statistics.status_codes.values().sum();
    let success: u64 = statistics
        .status_codes
        .iter()
        .filter(|(status, _)| status.starts_with(['2', '3']))
        .map(|(_, count)| count)
        .sum();
    if status_total > 0 && (success as f64 / status_total as f64) < 0.7 {
        recommendations.push("Consider increasing success rate in HTTP status codes".to_owned());
    }
    let mut issues_found: Vec<_> = errors
        .into_iter()
        .filter(|(_, count)| *count as f64 > records.len() as f64 * 0.1)
        .map(|(error, count)| format!("{error}: {count} records"))
        .collect();
    if statistics
        .ip_addresses
        .values()
        .max()
        .is_some_and(|count| *count as f64 > records.len() as f64 * 0.5)
    {
        issues_found
            .push("Single IP address dominates traffic (possible bot or test data)".to_owned());
    }
    let hours: u64 = statistics.time_distribution.values().sum();
    if hours > 0
        && statistics
            .time_distribution
            .values()
            .max()
            .is_some_and(|count| *count as f64 > hours as f64 * 0.8)
    {
        issues_found
            .push("Logs concentrated in single hour (possible batch generation)".to_owned());
    }
    Ok(QualityReport {
        overall_score,
        total_logs_analyzed: records.len(),
        validation_summary: ValidationSummary {
            valid_logs: statistics.valid_logs,
            invalid_logs: statistics.invalid_logs,
            average_quality_score: statistics.average_quality_score,
        },
        validation_results,
        statistics,
        recommendations,
        issues_found,
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
}
