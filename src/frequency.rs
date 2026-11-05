use chrono::{DateTime, Duration, Timelike, Utc};
use rand::{Rng, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ErrorFrequencyConfig {
    pub base_frequency: f64,
    pub peak_hours: Vec<u32>,
    pub peak_multiplier: f64,
    pub burst_probability: f64,
    pub burst_multiplier: f64,
    pub burst_duration_minutes: u32,
}
impl Default for ErrorFrequencyConfig {
    fn default() -> Self {
        Self {
            base_frequency: 1.0,
            peak_hours: vec![9, 10, 11, 14, 15, 16],
            peak_multiplier: 2.0,
            burst_probability: 0.1,
            burst_multiplier: 5.0,
            burst_duration_minutes: 5,
        }
    }
}
impl ErrorFrequencyConfig {
    pub fn validate(&self) -> io::Result<()> {
        if !self.base_frequency.is_finite()
            || self.base_frequency < 0.0
            || !self.peak_multiplier.is_finite()
            || self.peak_multiplier < 0.0
            || !self.burst_multiplier.is_finite()
            || self.burst_multiplier < 0.0
            || !self.burst_probability.is_finite()
            || !(0.0..=1.0).contains(&self.burst_probability)
            || self.peak_hours.iter().any(|hour| *hour > 23)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid error frequency configuration",
            ));
        }
        if !(self.base_frequency * self.peak_multiplier * self.burst_multiplier).is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "error frequency is too large",
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct ErrorFrequencyController {
    config: ErrorFrequencyConfig,
    burst_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    last_time: Option<DateTime<Utc>>,
    rng: StdRng,
}
impl ErrorFrequencyController {
    pub fn new(config: ErrorFrequencyConfig, seed: u64) -> io::Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            burst_range: None,
            last_time: None,
            rng: StdRng::seed_from_u64(seed),
        })
    }
    pub fn in_burst(&self, time: DateTime<Utc>) -> bool {
        self.burst_range
            .is_some_and(|(start, end)| start <= time && time < end)
    }
    pub fn should_generate(&mut self, time: DateTime<Utc>) -> io::Result<bool> {
        if self.last_time.is_some_and(|previous| time < previous) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frequency time moved backwards",
            ));
        }
        let mut rng = self.rng.clone();
        if !self.in_burst(time) && rng.gen_bool(self.config.burst_probability) {
            self.burst_range = Some((
                time,
                time.checked_add_signed(Duration::minutes(i64::from(
                    self.config.burst_duration_minutes,
                )))
                .ok_or_else(|| io::Error::other("burst end overflow"))?,
            ));
        }
        let peak = if self.config.peak_hours.contains(&time.hour()) {
            self.config.peak_multiplier
        } else {
            1.0
        };
        let burst = if self.in_burst(time) {
            self.config.burst_multiplier
        } else {
            1.0
        };
        let probability = (self.config.base_frequency * peak * burst / 60.0).min(1.0);
        let generate = rng.gen_bool(probability);
        self.last_time = Some(time);
        self.rng = rng;
        Ok(generate)
    }
}

pub use crate::template_frequency::{ConditionOperator, CorrelationRule};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FieldTimePattern {
    pub peak_hours: Vec<u32>,
    pub peak_multiplier: f64,
    pub weekend_factor: f64,
    pub business_hours_only: bool,
}
impl Default for FieldTimePattern {
    fn default() -> Self {
        Self {
            peak_hours: Vec::new(),
            peak_multiplier: 1.0,
            weekend_factor: 1.0,
            business_hours_only: false,
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FieldBurstPattern {
    pub burst_probability: f64,
    pub burst_duration_seconds: u32,
    pub burst_multiplier: f64,
    pub burst_values: Vec<String>,
}
impl Default for FieldBurstPattern {
    fn default() -> Self {
        Self {
            burst_probability: 0.0,
            burst_duration_seconds: 60,
            burst_multiplier: 1.0,
            burst_values: Vec::new(),
        }
    }
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FieldFrequencyConfig {
    pub distributions: std::collections::BTreeMap<String, std::collections::BTreeMap<String, f64>>,
    pub time_patterns: std::collections::BTreeMap<String, FieldTimePattern>,
    pub correlation_rules: Vec<CorrelationRule>,
    pub burst_patterns: std::collections::BTreeMap<String, FieldBurstPattern>,
}
impl FieldFrequencyConfig {
    pub fn web() -> Self {
        let distribution = |values: &[(&str, f64)]| {
            values
                .iter()
                .map(|(value, weight)| ((*value).to_owned(), *weight))
                .collect()
        };
        Self {
            distributions: std::collections::BTreeMap::from([
                (
                    "http_status_code".into(),
                    distribution(&[
                        ("200", 0.70),
                        ("404", 0.10),
                        ("500", 0.05),
                        ("301", 0.03),
                        ("302", 0.03),
                        ("400", 0.02),
                        ("401", 0.02),
                        ("403", 0.02),
                        ("503", 0.02),
                        ("502", 0.01),
                    ]),
                ),
                (
                    "http_method".into(),
                    distribution(&[
                        ("GET", 0.65),
                        ("POST", 0.25),
                        ("PUT", 0.04),
                        ("DELETE", 0.02),
                        ("PATCH", 0.02),
                        ("HEAD", 0.01),
                        ("OPTIONS", 0.01),
                    ]),
                ),
                (
                    "log_level".into(),
                    distribution(&[
                        ("INFO", 0.50),
                        ("WARN", 0.25),
                        ("ERROR", 0.15),
                        ("DEBUG", 0.08),
                        ("FATAL", 0.02),
                    ]),
                ),
            ]),
            time_patterns: std::collections::BTreeMap::from([(
                "http_status_code".into(),
                FieldTimePattern {
                    peak_hours: (9..=18).collect(),
                    peak_multiplier: 2.0,
                    weekend_factor: 0.6,
                    business_hours_only: false,
                },
            )]),
            correlation_rules: vec![CorrelationRule {
                condition_field: "http_status_code".into(),
                condition_value: "500".into(),
                condition_operator: ConditionOperator::Equal,
                target_field: "response_time".into(),
                target_distribution: distribution(&[
                    ("1000", 0.3),
                    ("2000", 0.3),
                    ("3000", 0.2),
                    ("5000", 0.2),
                ]),
                probability: 0.8,
            }],
            burst_patterns: std::collections::BTreeMap::from([(
                "http_status_code".into(),
                FieldBurstPattern {
                    burst_probability: 0.01,
                    burst_duration_seconds: 300,
                    burst_multiplier: 5.0,
                    burst_values: vec!["500".into(), "502".into(), "503".into()],
                },
            )]),
        }
    }

    pub fn security() -> Self {
        Self {
            distributions: std::collections::BTreeMap::from([(
                "security_event".into(),
                [
                    ("login_success", 0.85),
                    ("login_failure", 0.10),
                    ("permission_denied", 0.03),
                    ("suspicious_activity", 0.02),
                ]
                .into_iter()
                .map(|(value, weight)| (value.to_owned(), weight))
                .collect(),
            )]),
            time_patterns: std::collections::BTreeMap::from([(
                "security_event".into(),
                FieldTimePattern {
                    peak_hours: (8..=17).collect(),
                    peak_multiplier: 4.0,
                    weekend_factor: 0.2,
                    business_hours_only: false,
                },
            )]),
            burst_patterns: std::collections::BTreeMap::from([(
                "security_event".into(),
                FieldBurstPattern {
                    burst_probability: 0.005,
                    burst_duration_seconds: 600,
                    burst_multiplier: 10.0,
                    burst_values: vec![
                        "login_failure".into(),
                        "permission_denied".into(),
                        "suspicious_activity".into(),
                    ],
                },
            )]),
            correlation_rules: Vec::new(),
        }
    }

    pub fn database() -> Self {
        Self {
            distributions: std::collections::BTreeMap::from([(
                "query_type".into(),
                [
                    ("SELECT", 0.70),
                    ("INSERT", 0.15),
                    ("UPDATE", 0.10),
                    ("DELETE", 0.05),
                ]
                .into_iter()
                .map(|(value, weight)| (value.to_owned(), weight))
                .collect(),
            )]),
            time_patterns: std::collections::BTreeMap::from([(
                "query_type".into(),
                FieldTimePattern {
                    peak_hours: vec![9, 10, 11, 14, 15, 16],
                    peak_multiplier: 3.0,
                    weekend_factor: 0.3,
                    business_hours_only: false,
                },
            )]),
            correlation_rules: vec![CorrelationRule {
                condition_field: "rows_affected".into(),
                condition_value: "1000".into(),
                condition_operator: ConditionOperator::GreaterThan,
                target_field: "query_duration".into(),
                target_distribution: [("100", 0.2), ("500", 0.3), ("1000", 0.3), ("2000", 0.2)]
                    .into_iter()
                    .map(|(value, weight)| (value.to_owned(), weight))
                    .collect(),
                probability: 0.9,
            }],
            burst_patterns: std::collections::BTreeMap::new(),
        }
    }

    pub fn validate(&self) -> io::Result<()> {
        let valid_name = |name: &str| !name.is_empty() && !name.chars().any(char::is_whitespace);
        for (name, distribution) in &self.distributions {
            if !valid_name(name) {
                return Err(io::Error::other("invalid frequency field name"));
            }
            crate::template_frequency::validate_distribution(distribution)
                .map_err(io::Error::other)?;
        }
        for (name, pattern) in &self.time_patterns {
            if !valid_name(name)
                || pattern.peak_hours.iter().any(|hour| *hour > 23)
                || !pattern.peak_multiplier.is_finite()
                || pattern.peak_multiplier < 1.0
                || !pattern.weekend_factor.is_finite()
                || !(0.0..=1.0).contains(&pattern.weekend_factor)
            {
                return Err(io::Error::other("invalid field time pattern"));
            }
        }
        for (name, burst) in &self.burst_patterns {
            if !valid_name(name)
                || !burst.burst_probability.is_finite()
                || !(0.0..=1.0).contains(&burst.burst_probability)
                || burst.burst_duration_seconds == 0
                || !burst.burst_multiplier.is_finite()
                || burst.burst_multiplier < 1.0
                || burst
                    .burst_values
                    .iter()
                    .any(|value| value.is_empty() || value.contains(['\n', '\r']))
            {
                return Err(io::Error::other("invalid field burst pattern"));
            }
        }
        for rule in &self.correlation_rules {
            rule.validate_condition().map_err(io::Error::other)?;
            if !valid_name(&rule.condition_field)
                || !valid_name(&rule.target_field)
                || !rule.probability.is_finite()
                || !(0.0..=1.0).contains(&rule.probability)
            {
                return Err(io::Error::other("invalid field correlation rule"));
            }
            crate::template_frequency::validate_distribution(&rule.target_distribution)
                .map_err(io::Error::other)?;
        }
        Ok(())
    }
}

pub struct FieldFrequencyController {
    config: FieldFrequencyConfig,
    burst_ends: std::collections::BTreeMap<String, (DateTime<Utc>, DateTime<Utc>)>,
    last_time: Option<DateTime<Utc>>,
    rng: StdRng,
}

#[derive(Debug, Serialize)]
pub struct FieldFrequencyStatistics {
    pub frequency_maps_count: usize,
    pub time_patterns_count: usize,
    pub correlation_rules_count: usize,
    pub burst_patterns_count: usize,
    pub configured_fields: Vec<String>,
}

impl FieldFrequencyController {
    pub fn new(config: FieldFrequencyConfig, seed: u64) -> io::Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            burst_ends: std::collections::BTreeMap::new(),
            last_time: None,
            rng: StdRng::seed_from_u64(seed),
        })
    }
    fn check_time(&mut self, time: DateTime<Utc>) -> io::Result<()> {
        if self.last_time.is_some_and(|previous| time < previous) {
            return Err(io::Error::other("frequency time moved backwards"));
        }
        self.last_time = Some(time);
        Ok(())
    }
    pub fn in_burst(&self, field: &str, time: DateTime<Utc>) -> bool {
        self.burst_ends
            .get(field)
            .is_some_and(|(start, end)| *start <= time && time < *end)
    }

    pub fn statistics(&self) -> FieldFrequencyStatistics {
        let fields: std::collections::BTreeSet<_> = self
            .config
            .distributions
            .keys()
            .chain(self.config.time_patterns.keys())
            .chain(self.config.burst_patterns.keys())
            .cloned()
            .collect();
        FieldFrequencyStatistics {
            frequency_maps_count: self.config.distributions.len(),
            time_patterns_count: self.config.time_patterns.len(),
            correlation_rules_count: self.config.correlation_rules.len(),
            burst_patterns_count: self.config.burst_patterns.len(),
            configured_fields: fields.into_iter().collect(),
        }
    }
    pub fn should_generate_field(&mut self, field: &str, time: DateTime<Utc>) -> io::Result<bool> {
        use chrono::Datelike;
        self.check_time(time)?;
        let mut probability = 1.0;
        if let Some(pattern) = self.config.time_patterns.get(field) {
            let weekend = time.weekday().num_days_from_monday() >= 5;
            if pattern.business_hours_only && (weekend || !(9..=17).contains(&time.hour())) {
                return Ok(false);
            }
            if weekend {
                probability *= pattern.weekend_factor;
            }
            if !pattern.peak_hours.is_empty() && !pattern.peak_hours.contains(&time.hour()) {
                probability /= pattern.peak_multiplier;
            }
        }
        if self.in_burst(field, time)
            && let Some(burst) = self.config.burst_patterns.get(field)
        {
            probability = (probability * burst.burst_multiplier).min(1.0);
        }
        Ok(self.rng.gen_bool(probability))
    }
    pub fn controlled_value<F>(
        &mut self,
        field: &str,
        context: &std::collections::HashMap<String, String>,
        time: DateTime<Utc>,
        default: F,
    ) -> io::Result<String>
    where
        F: FnOnce() -> String,
    {
        match self.value_override(field, context, time)? {
            Some(value) => Ok(value),
            None => Ok(default()),
        }
    }
    pub(crate) fn value_override(
        &mut self,
        field: &str,
        context: &std::collections::HashMap<String, String>,
        time: DateTime<Utc>,
    ) -> io::Result<Option<String>> {
        let previous_time = self.last_time;
        let previous_rng = self.rng.clone();
        let previous_burst = self.burst_ends.get(field).copied();
        let result = self.value_override_inner(field, context, time);
        if result.is_err() {
            self.last_time = previous_time;
            self.rng = previous_rng;
            if let Some(burst) = previous_burst {
                self.burst_ends.insert(field.to_owned(), burst);
            } else {
                self.burst_ends.remove(field);
            }
        }
        result
    }

    fn value_override_inner(
        &mut self,
        field: &str,
        context: &std::collections::HashMap<String, String>,
        time: DateTime<Utc>,
    ) -> io::Result<Option<String>> {
        self.check_time(time)?;
        if !self.in_burst(field, time)
            && let Some(burst) = self.config.burst_patterns.get(field)
            && self.rng.gen_bool(burst.burst_probability)
        {
            let end = time
                .checked_add_signed(Duration::seconds(i64::from(burst.burst_duration_seconds)))
                .ok_or_else(|| io::Error::other("burst end overflow"))?;
            self.burst_ends.insert(field.to_owned(), (time, end));
        }
        if self.in_burst(field, time)
            && let Some(burst) = self.config.burst_patterns.get(field)
        {
            if burst.burst_values.is_empty() {
                return Ok(None);
            }
            return Ok(Some(
                burst.burst_values[self.rng.gen_range(0..burst.burst_values.len())].clone(),
            ));
        }
        crate::template_frequency::value(
            field,
            context,
            &self.config.distributions,
            &self.config.correlation_rules,
            &mut self.rng,
        )
        .map_err(io::Error::other)
    }
}
