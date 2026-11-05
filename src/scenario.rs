use crate::{GeneratorError, LogGenerator};
use chrono::{DateTime, Utc};
use rand::{Rng, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    fs, io,
    path::Path,
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Pattern {
    pub id: String,
    #[serde(default = "default_category")]
    pub category: String,
    pub severity: String,
    #[serde(default = "default_weight")]
    pub frequency_weight: f64,
    pub message_template: String,
    #[serde(default)]
    pub context_fields: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub real_world_examples: Vec<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

fn default_category() -> String {
    "uncategorized".into()
}
fn default_weight() -> f64 {
    1.0
}

fn same_pattern_ignoring_weight(first: &Pattern, second: &Pattern) -> bool {
    let mut adjusted = first.clone();
    adjusted.frequency_weight = second.frequency_weight;
    adjusted == *second
}

#[derive(Clone, Deserialize, Serialize)]
struct Scenario {
    name: String,
    patterns: Vec<String>,
    sequence_rules: Rules,
    #[serde(default = "duration")]
    duration_minutes: serde_json::Number,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}
fn duration() -> serde_json::Number {
    serde_json::Number::from(30)
}

#[derive(Clone, Deserialize, Serialize)]
struct Rules {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    escalation_chain: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    escalation_delay_minutes: Option<Vec<serde_json::Number>>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

pub struct ScenarioGenerator {
    scenario: Scenario,
    patterns: BTreeMap<String, Pattern>,
    started: Option<DateTime<Utc>>,
    last_generated: Option<DateTime<Utc>>,
    usage: BTreeMap<String, ScenarioPatternUsage>,
    last_event: Option<ScenarioEvent>,
    rng: StdRng,
}

#[derive(Clone, Debug, Default)]
pub struct ScenarioPatternUsage {
    pub count: u64,
    pub escalation_count: u64,
    pub last_used: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct ScenarioEvent {
    pub timestamp: DateTime<Utc>,
    pub scenario: String,
    pub pattern_id: String,
    pub category: String,
    pub severity: String,
}

#[derive(Debug, Serialize)]
pub struct ScenarioHealth {
    pub health_status: String,
    pub total_errors_last_hour: usize,
    pub error_rate_per_minute: f64,
    pub category_breakdown: BTreeMap<String, usize>,
    pub severity_breakdown: BTreeMap<String, usize>,
    pub active_scenarios: Vec<String>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ScenarioPatternAnalytics {
    pub pattern: Pattern,
    pub total_usage: u64,
    pub recent_usage: usize,
    pub escalation_count: u64,
    pub last_used: Option<String>,
    pub weight_changes: Vec<crate::error_library::WeightChange>,
}

pub enum ManagedError {
    Scenario(String),
    Pattern(String),
}

impl ScenarioPatternAnalytics {
    pub fn to_dict(&self) -> serde_json::Value {
        serde_json::json!({
            "pattern_info": {
                "id": self.pattern.id,
                "category": self.pattern.category,
                "severity": self.pattern.severity,
                "frequency_weight": self.pattern.frequency_weight,
                "description": self.pattern.description,
            },
            "usage_stats": {
                "total_usage": self.total_usage,
                "recent_usage": self.recent_usage,
                "last_used": self.last_used,
                "escalation_count": self.escalation_count,
            },
            "effectiveness": {
                "avg_response_time": 0.0,
                "weight_changes": self.weight_changes,
            },
        })
    }
}

#[derive(Default)]
pub struct ScenarioManager {
    definitions: BTreeMap<String, ScenarioGenerator>,
    active: BTreeMap<String, ScenarioGenerator>,
    active_order: Vec<String>,
    history: VecDeque<ScenarioEvent>,
    usage: BTreeMap<String, ScenarioPatternUsage>,
    regular_patterns: BTreeMap<String, Pattern>,
    frequency: Option<(
        crate::frequency::ErrorFrequencyConfig,
        crate::frequency::ErrorFrequencyController,
    )>,
}

impl ScenarioManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_frequency(
        &mut self,
        config: crate::frequency::ErrorFrequencyConfig,
        seed: u64,
    ) -> io::Result<()> {
        let controller = crate::frequency::ErrorFrequencyController::new(config.clone(), seed)?;
        self.frequency = Some((config, controller));
        Ok(())
    }

    pub fn generate_if_due(
        &mut self,
        name: &str,
        time: DateTime<Utc>,
    ) -> Result<Option<String>, GeneratorError> {
        let generator = self
            .active
            .get(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "scenario is not active"))?;
        if generator.started.is_some_and(|started| time < started)
            || generator.last_generated.is_some_and(|last| time < last)
        {
            return Err(io::Error::other("scenario time moved backwards").into());
        }
        let (_, frequency) = self
            .frequency
            .as_mut()
            .ok_or("scenario frequency is not configured")?;
        if !frequency.should_generate(time)? {
            return Ok(None);
        }
        self.generate(name, time).map(Some)
    }

    pub fn generate_first_active_if_due(
        &mut self,
        time: DateTime<Utc>,
    ) -> Result<Option<String>, GeneratorError> {
        self.check_prioritized_time(time)?;
        let (_, frequency) = self
            .frequency
            .as_mut()
            .ok_or("scenario frequency is not configured")?;
        if !frequency.should_generate(time)? {
            return Ok(None);
        }
        self.generate_first_active(time)
    }

    pub fn generate_realistic_if_due(
        &mut self,
        library: &mut crate::error_library::ErrorLibrary,
        time: DateTime<Utc>,
        category: Option<&str>,
        severity: Option<&str>,
        consider_scenarios: bool,
    ) -> Result<Option<ManagedError>, GeneratorError> {
        if consider_scenarios {
            self.check_prioritized_time(time)?;
        }
        let (config, frequency) = self
            .frequency
            .as_ref()
            .ok_or("scenario frequency is not configured")?;
        let config = config.clone();
        let mut trial = frequency.clone();
        if !trial.should_generate(time)? {
            self.frequency
                .as_mut()
                .ok_or("scenario frequency is not configured")?
                .1 = trial;
            return Ok(None);
        }
        if consider_scenarios {
            for name in &self.active_order {
                let generator = self.active.get(name).ok_or("missing active scenario")?;
                let expired = generator.started.is_some_and(|started| {
                    time.signed_duration_since(started)
                        .to_std()
                        .is_ok_and(|elapsed| {
                            elapsed.as_secs_f64() / 60.0
                                > generator
                                    .scenario
                                    .duration_minutes
                                    .as_f64()
                                    .expect("validated scenario duration")
                        })
                });
                if expired {
                    continue;
                }
                for (id, pattern) in &generator.patterns {
                    if !library
                        .get_pattern(id)
                        .is_some_and(|other| same_pattern_ignoring_weight(other, pattern))
                    {
                        return Err(
                            format!("scenario pattern differs from error library: {id}").into()
                        );
                    }
                }
                break;
            }
        }
        if consider_scenarios && let Some(record) = self.generate_first_active(time)? {
            let event = self.history.back().ok_or("missing scenario event")?;
            library.record_scenario_usage(&event.pattern_id, event.timestamp)?;
            if let Some(pattern) = library.get_pattern(&event.pattern_id) {
                self.regular_patterns
                    .insert(event.pattern_id.clone(), pattern.clone());
            }
            self.frequency
                .as_mut()
                .ok_or("scenario frequency is not configured")?
                .1 = trial;
            return Ok(Some(ManagedError::Scenario(record)));
        }
        let candidates = library.adjusted_weights(time, category, severity, &config)?;
        if candidates.is_empty() {
            self.frequency
                .as_mut()
                .ok_or("scenario frequency is not configured")?
                .1 = trial;
            return Ok(None);
        }
        for id in candidates.keys().map(String::as_str) {
            if let Some(previous) = self.regular_patterns.get(id)
                && !library
                    .get_pattern(id)
                    .is_some_and(|other| same_pattern_ignoring_weight(other, previous))
            {
                return Err(format!("error pattern differs from recorded history: {id}").into());
            }
            if let Some(defined) = self
                .definitions
                .values()
                .find_map(|generator| generator.patterns.get(id))
                && !library
                    .get_pattern(id)
                    .is_some_and(|other| same_pattern_ignoring_weight(other, defined))
            {
                return Err(format!("scenario pattern differs from error library: {id}").into());
            }
        }
        let (record, pattern) = library.generate_adjusted_record(
            time,
            category,
            severity,
            &config,
            &BTreeMap::new(),
        )?;
        self.frequency
            .as_mut()
            .ok_or("scenario frequency is not configured")?
            .1 = trial;
        let usage = self.usage.entry(pattern.id.clone()).or_default();
        usage.count += 1;
        usage.last_used = Some(time);
        if self.history.len() == 1000 {
            self.history.pop_front();
        }
        self.history.push_back(ScenarioEvent {
            timestamp: time,
            scenario: String::new(),
            pattern_id: pattern.id.clone(),
            category: pattern.category.clone(),
            severity: pattern.severity.clone(),
        });
        self.regular_patterns.insert(pattern.id.clone(), pattern);
        Ok(Some(ManagedError::Pattern(record)))
    }

    fn check_prioritized_time(&self, time: DateTime<Utc>) -> Result<(), GeneratorError> {
        for name in &self.active_order {
            let generator = self.active.get(name).ok_or("missing active scenario")?;
            if generator.started.is_some_and(|started| time < started)
                || generator.last_generated.is_some_and(|last| time < last)
            {
                return Err(io::Error::other("scenario time moved backwards").into());
            }
            let expired = generator.started.is_some_and(|started| {
                time.signed_duration_since(started)
                    .to_std()
                    .is_ok_and(|elapsed| {
                        elapsed.as_secs_f64() / 60.0
                            > generator
                                .scenario
                                .duration_minutes
                                .as_f64()
                                .expect("validated scenario duration")
                    })
            });
            if !expired {
                break;
            }
        }
        Ok(())
    }

    fn generate_first_active(
        &mut self,
        time: DateTime<Utc>,
    ) -> Result<Option<String>, GeneratorError> {
        while let Some(name) = self.active_order.first().cloned() {
            let generator = self.active.get(&name).ok_or("missing active scenario")?;
            if generator.started.is_some_and(|started| time < started)
                || generator.last_generated.is_some_and(|last| time < last)
            {
                return Err(io::Error::other("scenario time moved backwards").into());
            }
            let expired = generator.started.is_some_and(|started| {
                time.signed_duration_since(started)
                    .to_std()
                    .is_ok_and(|elapsed| {
                        elapsed.as_secs_f64() / 60.0
                            > generator
                                .scenario
                                .duration_minutes
                                .as_f64()
                                .expect("validated scenario duration")
                    })
            });
            if expired {
                self.stop(&name);
                continue;
            }
            return self.generate(&name, time).map(Some);
        }
        Ok(None)
    }

    pub fn register(&mut self, generator: ScenarioGenerator) -> io::Result<()> {
        let name = generator.scenario.name.clone();
        if name.is_empty() || self.definitions.contains_key(&name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "scenario name is empty or already registered",
            ));
        }
        for (id, pattern) in &generator.patterns {
            if self
                .regular_patterns
                .get(id)
                .is_some_and(|previous| !same_pattern_ignoring_weight(previous, pattern))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("conflicting scenario pattern: {id}"),
                ));
            }
            if self
                .definitions
                .values()
                .filter_map(|definition| definition.patterns.get(id))
                .any(|existing| existing != pattern)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("conflicting scenario pattern: {id}"),
                ));
            }
        }
        self.definitions.insert(name, generator);
        Ok(())
    }

    pub fn scenarios(&self) -> impl Iterator<Item = &str> {
        self.definitions.keys().map(String::as_str)
    }

    pub fn active_scenarios(&self) -> impl Iterator<Item = &str> {
        self.active_order.iter().map(String::as_str)
    }

    pub fn export_configuration(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;
        let scenarios: BTreeMap<_, _> = self
            .definitions
            .iter()
            .map(|(name, generator)| (name, &generator.scenario))
            .collect();
        let pattern_stats: BTreeMap<_, _> = self.usage.iter().map(|(id, usage)| (id, serde_json::json!({"usage_count": usage.count, "escalation_count": usage.escalation_count, "last_used": usage.last_used.map(|time| time.to_rfc3339())}))).collect();
        let mut document = serde_json::json!({"scenarios": scenarios, "pattern_stats": pattern_stats, "exported_at": Utc::now().to_rfc3339()});
        if let Some((config, _)) = &self.frequency {
            document["frequency_config"] = serde_json::to_value(config)?;
        }
        let data = serde_json::to_vec_pretty(&document)?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&data)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(path)
            .map_err(|error| error.error)?;
        Ok(())
    }

    pub fn start(&mut self, name: &str, seed: u64) -> io::Result<()> {
        self.start_inner(name, seed, None)
    }

    pub fn start_at(&mut self, name: &str, seed: u64, time: DateTime<Utc>) -> io::Result<()> {
        self.start_inner(name, seed, Some(time))
    }

    fn start_inner(
        &mut self,
        name: &str,
        seed: u64,
        started: Option<DateTime<Utc>>,
    ) -> io::Result<()> {
        if self.active.contains_key(name) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "scenario is already active",
            ));
        }
        let definition = self
            .definitions
            .get(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "scenario not registered"))?;
        let generator = ScenarioGenerator {
            scenario: definition.scenario.clone(),
            patterns: definition.patterns.clone(),
            started,
            last_generated: None,
            usage: BTreeMap::new(),
            last_event: None,
            rng: StdRng::seed_from_u64(seed),
        };
        self.active.insert(name.to_owned(), generator);
        self.active_order.push(name.to_owned());
        Ok(())
    }

    pub fn generate(&mut self, name: &str, time: DateTime<Utc>) -> Result<String, GeneratorError> {
        let expired = self.active.get(name).is_some_and(|generator| {
            generator.started.is_some_and(|started| {
                time.signed_duration_since(started)
                    .to_std()
                    .is_ok_and(|elapsed| {
                        elapsed.as_secs_f64() / 60.0
                            > generator
                                .scenario
                                .duration_minutes
                                .as_f64()
                                .expect("validated scenario duration")
                    })
            })
        });
        if expired {
            self.stop(name);
            return Err(io::Error::other("scenario duration exceeded").into());
        }
        let generator = self
            .active
            .get_mut(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "scenario is not active"))?;
        let output = generator.generate(time)?;
        let event = generator
            .last_event
            .as_ref()
            .ok_or("missing scenario event")?
            .clone();
        let usage = self.usage.entry(event.pattern_id.clone()).or_default();
        usage.count += 1;
        usage.escalation_count += 1;
        usage.last_used = Some(event.timestamp);
        if self.history.len() == 1000 {
            self.history.pop_front();
        }
        self.history.push_back(event);
        Ok(output)
    }

    pub fn active(&self, name: &str) -> Option<&ScenarioGenerator> {
        self.active.get(name)
    }

    pub fn pattern_analytics(&self, id: &str) -> Option<ScenarioPatternAnalytics> {
        let pattern = self
            .definitions
            .values()
            .find_map(|generator| generator.patterns.get(id));
        let pattern = self.regular_patterns.get(id).or(pattern)?;
        let usage = self.usage.get(id);
        Some(ScenarioPatternAnalytics {
            pattern: pattern.clone(),
            total_usage: usage.map_or(0, |usage| usage.count),
            escalation_count: usage.map_or(0, |usage| usage.escalation_count),
            weight_changes: Vec::new(),
            recent_usage: self
                .history
                .iter()
                .filter(|event| event.pattern_id == id)
                .count(),
            last_used: usage.and_then(|usage| usage.last_used.map(|time| time.to_rfc3339())),
        })
    }

    pub fn pattern_analytics_with_library(
        &self,
        library: &crate::error_library::ErrorLibrary,
        id: &str,
    ) -> io::Result<Option<ScenarioPatternAnalytics>> {
        let Some(pattern) = library.get_pattern(id) else {
            return Ok(None);
        };
        if self
            .regular_patterns
            .get(id)
            .is_some_and(|previous| !same_pattern_ignoring_weight(previous, pattern))
            || self
                .definitions
                .values()
                .filter_map(|generator| generator.patterns.get(id))
                .any(|previous| !same_pattern_ignoring_weight(previous, pattern))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("conflicting scenario pattern: {id}"),
            ));
        }
        let pattern = pattern.clone();
        let usage = self.usage.get(id);
        Ok(Some(ScenarioPatternAnalytics {
            pattern,
            total_usage: usage.map_or(0, |usage| usage.count),
            recent_usage: self
                .history
                .iter()
                .filter(|event| event.pattern_id == id)
                .count(),
            escalation_count: usage.map_or(0, |usage| usage.escalation_count),
            last_used: usage.and_then(|usage| usage.last_used.map(|time| time.to_rfc3339())),
            weight_changes: library.weight_changes(id).to_vec(),
        }))
    }

    pub fn stop(&mut self, name: &str) -> Option<ScenarioGenerator> {
        let result = self.active.remove(name);
        if result.is_some() {
            self.active_order.retain(|active| active != name);
        }
        result
    }

    pub fn health_at(&self, now: DateTime<Utc>) -> ScenarioHealth {
        let mut categories = BTreeMap::new();
        let mut severities = BTreeMap::new();
        let mut total = 0;
        for event in &self.history {
            let age = now.signed_duration_since(event.timestamp);
            if age >= chrono::Duration::zero() && age < chrono::Duration::hours(1) {
                total += 1;
                *categories.entry(event.category.clone()).or_insert(0) += 1;
                *severities
                    .entry(event.severity.to_ascii_lowercase())
                    .or_insert(0) += 1;
            }
        }
        let critical = severities.get("critical").copied().unwrap_or(0);
        let error = severities.get("error").copied().unwrap_or(0);
        let status = if critical > 5 || total > 50 {
            "critical"
        } else if critical > 2 || error > 20 {
            "warning"
        } else if total > 10 {
            "degraded"
        } else {
            "healthy"
        };
        let mut recommendations = Vec::new();
        if status == "critical" {
            recommendations
                .push("Review the synthetic scenario for critical error patterns".to_owned());
        }
        if critical > 3 {
            recommendations.push("Review critical events in the synthetic logs".to_owned());
        }
        for (category, limit, message) in [
            (
                "database",
                10,
                "Review database connections and queries in the synthetic logs",
            ),
            (
                "web_server",
                15,
                "Review upstream services and load in the synthetic logs",
            ),
            (
                "system",
                8,
                "Review disk and memory events in the synthetic logs",
            ),
        ] {
            if categories.get(category).copied().unwrap_or(0) > limit {
                recommendations.push(message.to_owned());
            }
        }
        if recommendations.is_empty() {
            recommendations.push("Continue monitoring the synthetic scenario".to_owned());
        }
        ScenarioHealth {
            health_status: status.into(),
            total_errors_last_hour: total,
            error_rate_per_minute: total as f64 / 60.0,
            category_breakdown: categories,
            severity_breakdown: severities,
            active_scenarios: self.active_order.clone(),
            recommendations,
        }
    }
}

impl ScenarioGenerator {
    pub fn definition(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(&self.scenario)
    }

    pub fn save_definition(&self, path: &Path) -> Result<(), GeneratorError> {
        use std::io::Write;
        let data = serde_json::to_vec_pretty(&self.scenario)?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&data)?;
        file.as_file().sync_all()?;
        file.persist_noclobber(path).map_err(|error| error.error)?;
        Ok(())
    }

    pub fn from_files(
        scenario_path: &Path,
        patterns_dir: &Path,
        seed: u64,
    ) -> Result<Self, GeneratorError> {
        Self::from_sources(scenario_path, Some(patterns_dir), seed)
    }

    pub fn from_sources(
        scenario_path: &Path,
        patterns_dir: Option<&Path>,
        seed: u64,
    ) -> Result<Self, GeneratorError> {
        let scenario: Scenario = serde_json::from_slice(&fs::read(scenario_path)?)?;
        Self::from_scenario(scenario, patterns_dir, seed)
    }

    pub fn from_definition(
        definition: serde_json::Value,
        patterns_dir: Option<&Path>,
        seed: u64,
    ) -> Result<Self, GeneratorError> {
        let scenario = serde_json::from_value(definition)?;
        Self::from_scenario(scenario, patterns_dir, seed)
    }

    fn from_scenario(
        scenario: Scenario,
        patterns_dir: Option<&Path>,
        seed: u64,
    ) -> Result<Self, GeneratorError> {
        if scenario.name.is_empty() {
            return Err(io::Error::other("scenario name is empty").into());
        }
        if !scenario
            .duration_minutes
            .as_f64()
            .is_some_and(|value| value.is_finite() && value >= 0.0)
        {
            return Err(io::Error::other("invalid scenario duration").into());
        }
        let chain = scenario
            .sequence_rules
            .escalation_chain
            .as_deref()
            .unwrap_or_default();
        let ids = if chain.is_empty() || scenario.sequence_rules.escalation_delay_minutes.is_none()
        {
            scenario.patterns.as_slice()
        } else {
            chain
        };
        if ids.is_empty() {
            return Err(io::Error::other("scenario has no patterns").into());
        }
        if scenario
            .sequence_rules
            .escalation_delay_minutes
            .as_ref()
            .is_some_and(|delays| {
                let values: Vec<f64> = delays
                    .iter()
                    .filter_map(serde_json::Number::as_f64)
                    .collect();
                values.len() != delays.len()
                    || values
                        .iter()
                        .any(|value| !value.is_finite() || *value < 0.0)
                    || values.windows(2).any(|value| value[0] > value[1])
            })
        {
            return Err(io::Error::other("scenario delays must be ordered").into());
        }
        let mut patterns = BTreeMap::new();
        let builtins: Vec<Pattern> =
            serde_json::from_str(include_str!("../data/builtin_error_patterns.json"))?;
        let mut builtins: BTreeMap<_, _> = builtins
            .into_iter()
            .map(|pattern| (pattern.id.clone(), pattern))
            .collect();
        for id in scenario.patterns.iter().chain(ids.iter()) {
            if patterns.contains_key(id) {
                continue;
            }
            if id.is_empty()
                || !id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(io::Error::other("invalid pattern id").into());
            }
            let file = patterns_dir.map(|dir| dir.join(format!("{id}.json")));
            let pattern = match file.as_ref().map(fs::read) {
                Some(Ok(data)) => serde_json::from_slice::<Pattern>(&data)?,
                Some(Err(error)) if error.kind() != io::ErrorKind::NotFound => {
                    return Err(error.into());
                }
                _ => {
                    // Existing scenarios can reference patterns supplied by the original built-in library.
                    let pattern = builtins.remove(id).ok_or_else(|| {
                        io::Error::new(io::ErrorKind::NotFound, format!("pattern not found: {id}"))
                    })?;
                    eprintln!("Using built-in error pattern: {id}");
                    pattern
                }
            };
            if pattern.id != *id {
                return Err(io::Error::other("pattern id does not match filename").into());
            }
            crate::error_library::validate_pattern(&pattern)?;
            patterns.insert(id.clone(), pattern);
        }
        Ok(Self {
            scenario,
            patterns,
            started: None,
            last_generated: None,
            usage: BTreeMap::new(),
            last_event: None,
            rng: StdRng::seed_from_u64(seed),
        })
    }

    pub fn started_at(&self) -> Option<DateTime<Utc>> {
        self.started
    }

    pub fn last_generated_at(&self) -> Option<DateTime<Utc>> {
        self.last_generated
    }

    pub fn pattern_usage(&self) -> &BTreeMap<String, ScenarioPatternUsage> {
        &self.usage
    }
}

fn context_repr(value: &serde_json::Value, nested: bool) -> String {
    match value {
        serde_json::Value::Null => "None".into(),
        serde_json::Value::Bool(value) => if *value { "True" } else { "False" }.into(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) if !nested => value.clone(),
        serde_json::Value::String(value) => {
            let quote = if value.contains('\'') && !value.contains('"') {
                '"'
            } else {
                '\''
            };
            let mut result = String::from(quote);
            for character in value.chars() {
                match character {
                    '\\' => result.push_str("\\\\"),
                    '\n' => result.push_str("\\n"),
                    '\r' => result.push_str("\\r"),
                    '\t' => result.push_str("\\t"),
                    '\u{2028}' => result.push_str("\\u2028"),
                    '\u{2029}' => result.push_str("\\u2029"),
                    c if c == quote => {
                        result.push('\\');
                        result.push(c);
                    }
                    c if c.is_control() && (c as u32) <= 255 => {
                        result.push_str(&format!("\\x{:02x}", c as u32))
                    }
                    c => result.push(c),
                }
            }
            result.push(quote);
            result
        }
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(|value| context_repr(value, true))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        serde_json::Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!(
                    "{}: {}",
                    context_repr(&serde_json::Value::String(key.clone()), true),
                    context_repr(value, true)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

impl Pattern {
    pub(crate) fn render(&self, rng: &mut StdRng) -> Result<String, GeneratorError> {
        let mut values = BTreeMap::new();
        for (key, value) in &self.context_fields {
            let value = match value {
                serde_json::Value::Array(values) if !values.is_empty() => {
                    values[rng.gen_range(0..values.len())].clone()
                }
                serde_json::Value::Object(values)
                    if values.contains_key("min_val") && values.contains_key("max_val") =>
                {
                    let min = values["min_val"]
                        .as_i64()
                        .ok_or("invalid context minimum")?;
                    let max = values["max_val"]
                        .as_i64()
                        .ok_or("invalid context maximum")?;
                    if min > max {
                        return Err("invalid context range".into());
                    }
                    serde_json::Value::from(rng.gen_range(min..=max))
                }
                serde_json::Value::Object(values) => {
                    values.get("default").cloned().unwrap_or_else(|| {
                        // The original emits this marker when a context default is absent.
                        serde_json::Value::String("unknown".to_owned())
                    })
                }
                value => value.clone(),
            };
            let value = context_repr(&value, false);
            values.insert(key.as_str(), value);
        }
        let mut rendered = String::new();
        for part in crate::template_syntax::parse(&self.message_template) {
            match part.map_err(io::Error::other)? {
                crate::template_syntax::Part::Text(text) => rendered.push_str(text),
                crate::template_syntax::Part::Field(name) => {
                    rendered.push_str(values.get(name).ok_or("unresolved pattern field")?)
                }
            }
        }
        Ok(rendered)
    }
}

impl LogGenerator for ScenarioGenerator {
    fn generate(&mut self, timestamp: DateTime<Utc>) -> Result<String, GeneratorError> {
        if self.last_generated.is_some_and(|last| timestamp < last) {
            return Err(io::Error::other("scenario time moved backwards").into());
        }
        let started = self.started.unwrap_or(timestamp);
        let duration = timestamp.signed_duration_since(started);
        if duration < chrono::Duration::zero() {
            return Err(io::Error::other("scenario time moved backwards").into());
        }
        if duration.to_std()?.as_secs_f64() / 60.0
            > self
                .scenario
                .duration_minutes
                .as_f64()
                .expect("validated scenario duration")
        {
            return Err(io::Error::other("scenario duration exceeded").into());
        }
        let elapsed_minutes = duration.to_std()?.as_secs_f64() / 60.0;
        let chain = self
            .scenario
            .sequence_rules
            .escalation_chain
            .as_deref()
            .unwrap_or_default();
        let id = if chain.is_empty()
            || self
                .scenario
                .sequence_rules
                .escalation_delay_minutes
                .is_none()
        {
            &self.scenario.patterns[self.rng.gen_range(0..self.scenario.patterns.len())]
        } else {
            let level = self
                .scenario
                .sequence_rules
                .escalation_delay_minutes
                .as_deref()
                .unwrap_or_default()
                .iter()
                .take_while(|delay| delay.as_f64().is_some_and(|value| elapsed_minutes >= value))
                .count();
            &chain[level.min(chain.len() - 1)]
        };
        let pattern = self.patterns.get(id).ok_or("missing scenario pattern")?;
        let rendered = pattern.render(&mut self.rng)?;
        self.started = Some(started);
        self.last_generated = Some(timestamp);
        let usage = self.usage.entry(pattern.id.clone()).or_default();
        usage.count += 1;
        usage.escalation_count += 1;
        usage.last_used = Some(timestamp);
        self.last_event = Some(ScenarioEvent {
            timestamp,
            scenario: self.scenario.name.clone(),
            pattern_id: pattern.id.clone(),
            category: pattern.category.clone(),
            severity: pattern.severity.clone(),
        });
        Ok(format!(
            "{} [{}] scenario={} pattern={} {}",
            timestamp.to_rfc3339(),
            pattern.severity,
            self.scenario.name,
            pattern.id,
            rendered
        ))
    }
    fn pattern(&self) -> &str {
        "scenario=<name> pattern=<id>"
    }
    fn validate(&self, record: &str) -> bool {
        record.contains(&format!("scenario={} pattern=", self.scenario.name))
    }
}
