pub use crate::scenario::Pattern as ErrorPattern;
use crate::{GeneratorError, LogGenerator, scenario::Pattern};
use chrono::{DateTime, Timelike, Utc};
use rand::{
    SeedableRng,
    distributions::{Distribution, WeightedIndex},
    rngs::StdRng,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
};

#[derive(Deserialize)]
#[serde(untagged)]
enum PatternFile {
    Multiple(Vec<Pattern>),
    Single(Pattern),
}

pub(crate) fn validate_pattern(pattern: &Pattern) -> Result<(), GeneratorError> {
    if pattern.id.is_empty()
        || !pattern
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        || pattern.category.is_empty()
        || pattern.category.chars().any(char::is_whitespace)
        || pattern.severity.is_empty()
        || pattern.severity.chars().any(char::is_whitespace)
    {
        return Err("invalid error pattern metadata".into());
    }
    if !pattern.frequency_weight.is_finite() || pattern.frequency_weight < 0.0 {
        return Err("invalid error pattern frequency_weight".into());
    }
    for value in pattern.context_fields.values() {
        if let serde_json::Value::Array(options) = value
            && options.is_empty()
        {
            return Err("invalid context value".into());
        }
    }
    pattern.render(&mut StdRng::seed_from_u64(0))?;
    Ok(())
}

fn validate_pattern_with_external_context(pattern: &Pattern) -> Result<(), GeneratorError> {
    let mut validated = pattern.clone();
    for part in crate::template_syntax::parse(&pattern.message_template) {
        if let crate::template_syntax::Part::Field(name) = part.map_err(io::Error::other)? {
            validated
                .context_fields
                .entry(name.to_owned())
                .or_insert_with(|| serde_json::Value::String("external".into()));
        }
    }
    validate_pattern(&validated)
}

pub struct ErrorLibrary {
    patterns: Vec<Pattern>,
    weights: WeightedIndex<f64>,
    rng: StdRng,
    usage: BTreeMap<String, PatternUsage>,
    weight_changes: BTreeMap<String, Vec<WeightChange>>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct WeightChange {
    pub timestamp: String,
    pub old_weight: f64,
    pub new_weight: f64,
    pub reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct PatternUsage {
    pub total_usage: u64,
    pub last_used: Option<DateTime<Utc>>,
}

#[derive(Debug, serde::Serialize)]
pub struct LibraryStatistics {
    pub total_patterns: usize,
    pub categories: BTreeMap<String, usize>,
    pub severity_distribution: BTreeMap<String, usize>,
    pub average_weight: f64,
}
impl ErrorLibrary {
    pub fn from_sources(
        directory: Option<&Path>,
        category: Option<&str>,
        severity: Option<&str>,
        seed: u64,
    ) -> Result<Self, GeneratorError> {
        Self::from_sources_inner(directory, category, severity, seed, false)
    }

    pub fn from_sources_with_external_context(
        directory: Option<&Path>,
        category: Option<&str>,
        severity: Option<&str>,
        seed: u64,
    ) -> Result<Self, GeneratorError> {
        Self::from_sources_inner(directory, category, severity, seed, true)
    }

    fn from_sources_inner(
        directory: Option<&Path>,
        category: Option<&str>,
        severity: Option<&str>,
        seed: u64,
        external_context: bool,
    ) -> Result<Self, GeneratorError> {
        let builtins: Vec<Pattern> =
            serde_json::from_str(include_str!("../data/builtin_error_patterns.json"))?;
        let mut patterns: BTreeMap<String, Pattern> = builtins
            .into_iter()
            .map(|pattern| (pattern.id.clone(), pattern))
            .collect();
        if let Some(directory) = directory {
            let mut paths = fs::read_dir(directory)?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<io::Result<Vec<_>>>()?;
            paths.sort();
            let mut custom_ids = HashSet::new();
            for path in paths.into_iter().filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            }) {
                let file: PatternFile = serde_json::from_slice(&fs::read(&path)?)?;
                let entries = match file {
                    PatternFile::Single(pattern) => vec![pattern],
                    PatternFile::Multiple(patterns) => patterns,
                };
                for pattern in entries {
                    if !custom_ids.insert(pattern.id.clone()) {
                        return Err(
                            format!("duplicate custom error pattern: {}", pattern.id).into()
                        );
                    }
                    patterns.insert(pattern.id.clone(), pattern);
                }
            }
        }
        let mut matching = Vec::new();
        for pattern in patterns.into_values() {
            if external_context {
                validate_pattern_with_external_context(&pattern)?;
            } else {
                validate_pattern(&pattern)?;
            }
            if category.is_none_or(|value| value == pattern.category)
                && severity.is_none_or(|value| value == pattern.severity)
            {
                matching.push(pattern);
            }
        }
        if matching.is_empty() {
            return Err("no error patterns match the selected filters".into());
        }
        let weights =
            crate::weights::checked(matching.iter().map(|pattern| pattern.frequency_weight))?;
        Ok(Self {
            patterns: matching,
            weights,
            rng: StdRng::seed_from_u64(seed),
            usage: BTreeMap::new(),
            weight_changes: BTreeMap::new(),
        })
    }
    pub fn pattern_ids(&self) -> impl Iterator<Item = &str> {
        self.patterns.iter().map(|pattern| pattern.id.as_str())
    }

    pub fn select_pattern(
        &mut self,
        category: Option<&str>,
        severity: Option<&str>,
    ) -> Result<&ErrorPattern, GeneratorError> {
        let candidates: Vec<usize> = self
            .patterns
            .iter()
            .enumerate()
            .filter_map(|(index, pattern)| {
                (category.is_none_or(|value| value == pattern.category)
                    && severity.is_none_or(|value| value == pattern.severity))
                .then_some(index)
            })
            .collect();
        if candidates.is_empty() {
            return Err("no error patterns match the selected filters".into());
        }
        let weights = crate::weights::checked(
            candidates
                .iter()
                .map(|index| self.patterns[*index].frequency_weight),
        )?;
        let index = candidates[weights.sample(&mut self.rng)];
        Ok(&self.patterns[index])
    }

    pub fn generate_scenario(
        &mut self,
        category: &str,
        count: usize,
    ) -> Result<Vec<(String, String)>, GeneratorError> {
        self.generate_scenario_with_context(category, count, &BTreeMap::new())
    }

    pub fn generate_scenario_with_context(
        &mut self,
        category: &str,
        count: usize,
        context: &BTreeMap<String, String>,
    ) -> Result<Vec<(String, String)>, GeneratorError> {
        if !self
            .patterns
            .iter()
            .any(|pattern| pattern.category == category)
        {
            return Ok(Vec::new());
        }
        let original_rng = self.rng.clone();
        let result = (|| {
            let mut result = Vec::new();
            for _ in 0..count {
                let id = self.select_pattern(Some(category), None)?.id.clone();
                let message = self.generate_message(&id, context)?;
                result.push((id, message));
            }
            Ok(result)
        })();
        if result.is_err() {
            self.rng = original_rng;
        }
        result
    }

    pub fn generate_filtered(
        &mut self,
        timestamp: DateTime<Utc>,
        category: Option<&str>,
        severity: Option<&str>,
    ) -> Result<String, GeneratorError> {
        self.generate_filtered_with_context(timestamp, category, severity, &BTreeMap::new())
    }

    pub fn generate_filtered_with_context(
        &mut self,
        timestamp: DateTime<Utc>,
        category: Option<&str>,
        severity: Option<&str>,
        context: &BTreeMap<String, String>,
    ) -> Result<String, GeneratorError> {
        let original_rng = self.rng.clone();
        let mut pattern = self.select_pattern(category, severity)?.clone();
        for (name, value) in context {
            pattern
                .context_fields
                .insert(name.clone(), serde_json::Value::String(value.clone()));
        }
        let message = match pattern.render(&mut self.rng) {
            Ok(message) => message,
            Err(error) => {
                self.rng = original_rng;
                return Err(error);
            }
        };
        let usage = self.usage.entry(pattern.id.clone()).or_default();
        usage.total_usage += 1;
        usage.last_used = Some(timestamp);
        Ok(format!(
            "{} [{}] category={} pattern={} {}",
            timestamp.to_rfc3339(),
            pattern.severity,
            pattern.category,
            pattern.id,
            message
        ))
    }

    pub fn adjusted_weights(
        &self,
        timestamp: DateTime<Utc>,
        category: Option<&str>,
        severity: Option<&str>,
        frequency: &crate::frequency::ErrorFrequencyConfig,
    ) -> Result<BTreeMap<String, f64>, GeneratorError> {
        frequency.validate()?;
        let mut weights = BTreeMap::new();
        for pattern in &self.patterns {
            if category.is_some_and(|value| value != pattern.category)
                || severity.is_some_and(|value| value != pattern.severity)
            {
                continue;
            }
            let mut weight = pattern.frequency_weight;
            if let Some(last) = self
                .usage
                .get(&pattern.id)
                .and_then(|usage| usage.last_used)
            {
                let age = timestamp.signed_duration_since(last);
                if age < chrono::Duration::zero() {
                    return Err(io::Error::other("error pattern time moved backwards").into());
                }
                if age < chrono::Duration::minutes(10) {
                    weight *= age.to_std()?.as_secs_f64() / 600.0;
                }
            }
            if pattern.severity == "critical" && frequency.peak_hours.contains(&timestamp.hour()) {
                weight *= 0.5;
            }
            weights.insert(pattern.id.clone(), weight);
        }
        Ok(weights)
    }

    pub fn generate_adjusted(
        &mut self,
        timestamp: DateTime<Utc>,
        category: Option<&str>,
        severity: Option<&str>,
        frequency: &crate::frequency::ErrorFrequencyConfig,
    ) -> Result<String, GeneratorError> {
        self.generate_adjusted_with_context(
            timestamp,
            category,
            severity,
            frequency,
            &BTreeMap::new(),
        )
    }

    pub fn generate_adjusted_with_context(
        &mut self,
        timestamp: DateTime<Utc>,
        category: Option<&str>,
        severity: Option<&str>,
        frequency: &crate::frequency::ErrorFrequencyConfig,
        context: &BTreeMap<String, String>,
    ) -> Result<String, GeneratorError> {
        self.generate_adjusted_record(timestamp, category, severity, frequency, context)
            .map(|(record, _)| record)
    }

    pub(crate) fn generate_adjusted_record(
        &mut self,
        timestamp: DateTime<Utc>,
        category: Option<&str>,
        severity: Option<&str>,
        frequency: &crate::frequency::ErrorFrequencyConfig,
        context: &BTreeMap<String, String>,
    ) -> Result<(String, Pattern), GeneratorError> {
        let weights = self.adjusted_weights(timestamp, category, severity, frequency)?;
        let distribution = crate::weights::checked(weights.values().copied())?;
        let mut rng = self.rng.clone();
        let id = weights
            .keys()
            .nth(distribution.sample(&mut rng))
            .ok_or("no error patterns match the selected filters")?
            .clone();
        let mut pattern = self
            .get_pattern(&id)
            .ok_or("missing error pattern")?
            .clone();
        for (name, value) in context {
            pattern
                .context_fields
                .insert(name.clone(), serde_json::Value::String(value.clone()));
        }
        let message = pattern.render(&mut rng)?;
        self.rng = rng;
        let usage = self.usage.entry(pattern.id.clone()).or_default();
        usage.total_usage += 1;
        usage.last_used = Some(timestamp);
        Ok((
            format!(
                "{} [{}] category={} pattern={} {}",
                timestamp.to_rfc3339(),
                pattern.severity,
                pattern.category,
                pattern.id,
                message
            ),
            pattern,
        ))
    }

    pub fn get_pattern(&self, id: &str) -> Option<&ErrorPattern> {
        self.patterns.iter().find(|pattern| pattern.id == id)
    }

    pub fn required_context_fields(&self, id: &str) -> Result<Vec<String>, GeneratorError> {
        let pattern = self.get_pattern(id).ok_or("unknown error pattern")?;
        let mut fields = std::collections::BTreeSet::new();
        for part in crate::template_syntax::parse(&pattern.message_template) {
            if let crate::template_syntax::Part::Field(name) = part.map_err(io::Error::other)?
                && !pattern.context_fields.contains_key(name)
            {
                fields.insert(name.to_owned());
            }
        }
        Ok(fields.into_iter().collect())
    }

    pub fn pattern_usage(&self, id: &str) -> Option<&PatternUsage> {
        self.usage.get(id)
    }

    pub(crate) fn record_scenario_usage(
        &mut self,
        id: &str,
        timestamp: DateTime<Utc>,
    ) -> Result<(), GeneratorError> {
        if self.get_pattern(id).is_none() {
            return Err("scenario pattern not found in error library".into());
        }
        let usage = self.usage.entry(id.to_owned()).or_default();
        usage.total_usage += 1;
        usage.last_used = Some(timestamp);
        Ok(())
    }

    pub fn statistics(&self) -> LibraryStatistics {
        let mut categories = BTreeMap::new();
        let mut severities = BTreeMap::new();
        for pattern in &self.patterns {
            *categories.entry(pattern.category.clone()).or_insert(0) += 1;
            *severities.entry(pattern.severity.clone()).or_insert(0) += 1;
        }
        LibraryStatistics {
            total_patterns: self.patterns.len(),
            categories,
            severity_distribution: severities,
            average_weight: self
                .patterns
                .iter()
                .map(|pattern| pattern.frequency_weight)
                .sum::<f64>()
                / self.patterns.len() as f64,
        }
    }

    pub fn add_pattern(&mut self, pattern: ErrorPattern) -> Result<(), GeneratorError> {
        validate_pattern(&pattern)?;
        self.insert_pattern(pattern)
    }

    pub fn add_pattern_with_external_context(
        &mut self,
        pattern: ErrorPattern,
    ) -> Result<(), GeneratorError> {
        validate_pattern_with_external_context(&pattern)?;
        self.insert_pattern(pattern)
    }

    fn insert_pattern(&mut self, pattern: ErrorPattern) -> Result<(), GeneratorError> {
        let id = pattern.id.clone();
        let mut updated = self.patterns.clone();
        if let Some(existing) = updated.iter_mut().find(|entry| entry.id == pattern.id) {
            *existing = pattern;
        } else {
            updated.push(pattern);
            updated.sort_by(|a, b| a.id.cmp(&b.id));
        }
        let weights = crate::weights::checked(updated.iter().map(|entry| entry.frequency_weight))?;
        self.patterns = updated;
        self.weights = weights;
        self.usage.remove(&id);
        self.weight_changes.remove(&id);
        Ok(())
    }

    pub fn generate_message(
        &mut self,
        id: &str,
        context: &BTreeMap<String, String>,
    ) -> Result<String, GeneratorError> {
        let mut pattern = self.get_pattern(id).ok_or("unknown error pattern")?.clone();
        for (key, value) in context {
            pattern
                .context_fields
                .insert(key.clone(), serde_json::Value::String(value.clone()));
        }
        let mut rng = self.rng.clone();
        let message = pattern.render(&mut rng)?;
        self.rng = rng;
        Ok(message)
    }

    pub fn patterns_by_category<'a>(
        &'a self,
        category: &'a str,
    ) -> impl Iterator<Item = &'a ErrorPattern> {
        self.patterns
            .iter()
            .filter(move |pattern| pattern.category == category)
    }

    pub fn patterns_by_severity<'a>(
        &'a self,
        severity: &'a str,
    ) -> impl Iterator<Item = &'a ErrorPattern> {
        self.patterns
            .iter()
            .filter(move |pattern| pattern.severity == severity)
    }

    pub fn categories(&self) -> Vec<&str> {
        self.patterns
            .iter()
            .map(|pattern| pattern.category.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn severities(&self) -> Vec<&str> {
        self.patterns
            .iter()
            .map(|pattern| pattern.severity.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn update_weight(&mut self, id: &str, weight: f64) -> Result<(), GeneratorError> {
        if !weight.is_finite() || weight < 0.0 {
            return Err("invalid error pattern frequency_weight".into());
        }
        let index = self
            .patterns
            .iter()
            .position(|pattern| pattern.id == id)
            .ok_or("unknown error pattern")?;
        let weights =
            crate::weights::checked(self.patterns.iter().enumerate().map(|(i, pattern)| {
                if i == index {
                    weight
                } else {
                    pattern.frequency_weight
                }
            }))?;
        self.patterns[index].frequency_weight = weight;
        self.weights = weights;
        Ok(())
    }

    pub fn update_weight_with_reason(
        &mut self,
        id: &str,
        weight: f64,
        reason: &str,
        time: DateTime<Utc>,
    ) -> Result<(), GeneratorError> {
        let old_weight = self
            .get_pattern(id)
            .ok_or("unknown error pattern")?
            .frequency_weight;
        self.update_weight(id, weight)?;
        self.weight_changes
            .entry(id.to_owned())
            .or_default()
            .push(WeightChange {
                timestamp: time.to_rfc3339(),
                old_weight,
                new_weight: weight,
                reason: reason.to_owned(),
            });
        Ok(())
    }

    pub fn weight_changes(&self, id: &str) -> &[WeightChange] {
        self.weight_changes.get(id).map_or(&[], Vec::as_slice)
    }

    pub fn export_analytics(&self, path: &Path) -> Result<(), GeneratorError> {
        let stats: BTreeMap<_, _> = self.patterns.iter().map(|pattern| {
            let usage = self.usage.get(&pattern.id);
            (pattern.id.as_str(), serde_json::json!({
                "total_usage": usage.map_or(0, |value| value.total_usage),
                "last_used": usage.and_then(|value| value.last_used.map(|time| time.to_rfc3339())),
                "weight_changes": self.weight_changes(&pattern.id),
            }))
        }).collect();
        let data = serde_json::to_vec_pretty(
            &serde_json::json!({"patterns": self.patterns, "pattern_stats": stats, "exported_at": Utc::now().to_rfc3339()}),
        )?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut output = tempfile::NamedTempFile::new_in(parent)?;
        output.write_all(&data)?;
        output.as_file().sync_all()?;
        output
            .persist_noclobber(path)
            .map_err(|error| error.error)?;
        Ok(())
    }

    pub fn export(&self, path: &Path) -> Result<(), GeneratorError> {
        let bytes = serde_json::to_vec_pretty(&self.patterns)?;
        let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
        output.write_all(&bytes)?;
        Ok(())
    }
}
impl LogGenerator for ErrorLibrary {
    fn generate(&mut self, timestamp: DateTime<Utc>) -> Result<String, GeneratorError> {
        let pattern = &self.patterns[self.weights.sample(&mut self.rng)];
        let message = pattern.render(&mut self.rng)?;
        let usage = self.usage.entry(pattern.id.clone()).or_default();
        usage.total_usage += 1;
        usage.last_used = Some(timestamp);
        Ok(format!(
            "{} [{}] category={} pattern={} {}",
            timestamp.to_rfc3339(),
            pattern.severity,
            pattern.category,
            pattern.id,
            message
        ))
    }
    fn pattern(&self) -> &str {
        "category=<category> pattern=<id>"
    }
    fn validate(&self, record: &str) -> bool {
        record.contains(" category=")
            && self
                .patterns
                .iter()
                .any(|pattern| record.contains(&format!(" pattern={} ", pattern.id)))
    }
}
