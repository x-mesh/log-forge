use crate::{
    app::compat::render_custom,
    pattern_file::{LogPattern, prepare, validate_pattern},
    template_syntax::{self, Part},
};
use chrono::{DateTime, Utc};
use rand::{SeedableRng, rngs::StdRng};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    time::Instant,
};

#[derive(Serialize)]
pub struct FieldInfo {
    pub name: String,
    pub options: String,
}
#[derive(Serialize)]
pub struct PatternValidation {
    pub valid: bool,
    pub fields: Vec<FieldInfo>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}
pub fn validate(pattern: &LogPattern) -> PatternValidation {
    let mut fields = Vec::new();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for part in template_syntax::parse(&pattern.template) {
        match part {
            Ok(Part::Field(spec)) => {
                let (name, options) = spec.split_once(':').unwrap_or((spec, ""));
                fields.push(FieldInfo {
                    name: name.into(),
                    options: options.into(),
                });
            }
            Ok(Part::Text(_)) => {}
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty()
        && let Err(error) = validate_pattern(pattern)
    {
        errors.push(error.to_string());
    }
    if !fields
        .iter()
        .any(|field| field.name.contains("timestamp") || field.name == "datetime")
    {
        warnings.push("template has no timestamp field".into());
    }
    PatternValidation {
        valid: errors.is_empty(),
        fields,
        errors,
        warnings,
    }
}

pub fn suggest_improvements(pattern: &LogPattern) -> Result<Vec<String>, String> {
    let mut fields = Vec::new();
    let mut static_content = String::new();
    for part in template_syntax::parse(&pattern.template) {
        match part? {
            Part::Field(spec) => fields.push(spec),
            Part::Text(text) => static_content.push_str(text),
        }
    }
    let mut suggestions: Vec<String> = validate(pattern)
        .warnings
        .into_iter()
        .map(|warning| format!("Warning: {warning}"))
        .collect();
    if fields.iter().collect::<BTreeSet<_>>().len() < fields.len() {
        suggestions.push("Consider using different field names to avoid confusion".into());
    }
    if !fields.iter().any(|field| field.contains("timestamp")) {
        suggestions.push("Consider adding a timestamp field for better log correlation".into());
    }
    if static_content.trim().chars().count() < 10 {
        suggestions.push("Consider adding more static content for better log structure".into());
    }
    if fields.iter().all(|field| !field.contains(':')) {
        suggestions.push("Consider using field options (e.g., {status_code:200,404,500}) for more realistic data".into());
    }
    Ok(suggestions)
}

#[derive(Serialize)]
pub struct GenerationReport {
    pub samples: Vec<String>,
    pub total_generated: usize,
    pub unique_outputs: usize,
}
pub fn test_generation(
    pattern: &LogPattern,
    timestamp: DateTime<Utc>,
    seed: u64,
    count: usize,
) -> Result<GenerationReport, Box<dyn std::error::Error>> {
    if count == 0 || count > 10_000 {
        return Err("sample count must be between 1 and 10000".into());
    }
    validate_pattern(pattern)?;
    let settings = prepare(pattern, &BTreeMap::new())?;
    let mut rng = StdRng::seed_from_u64(seed);
    let mut samples = Vec::with_capacity(count);
    for index in 0..count {
        samples.push(
            render_custom(&settings, timestamp, &mut rng, index as u64)
                .map_err(io::Error::other)?,
        );
    }
    let unique_outputs = samples.iter().collect::<BTreeSet<_>>().len();
    Ok(GenerationReport {
        total_generated: samples.len(),
        samples,
        unique_outputs,
    })
}

#[derive(Serialize)]
pub struct RegexMatch {
    pub input: String,
    pub matched: String,
    pub groups: Vec<Option<String>>,
}
#[derive(Serialize)]
pub struct RegexReport {
    pub matches: Vec<RegexMatch>,
    pub non_matches: Vec<String>,
}
pub fn test_regex(expression: &str, inputs: &[String]) -> Result<RegexReport, regex::Error> {
    let regex = regex::Regex::new(expression)?;
    let mut report = RegexReport {
        matches: Vec::new(),
        non_matches: Vec::new(),
    };
    for input in inputs {
        if let Some(captures) = regex.captures(input) {
            report.matches.push(RegexMatch {
                input: input.clone(),
                matched: captures[0].to_owned(),
                groups: captures
                    .iter()
                    .skip(1)
                    .map(|group| group.map(|value| value.as_str().to_owned()))
                    .collect(),
            });
        } else {
            report.non_matches.push(input.clone());
        }
    }
    Ok(report)
}

#[derive(Serialize)]
pub struct PatternComparison {
    pub first: GenerationReport,
    pub second: GenerationReport,
    pub common_fields: Vec<String>,
    pub first_only: Vec<String>,
    pub second_only: Vec<String>,
}
pub fn compare(
    first: &LogPattern,
    second: &LogPattern,
    timestamp: DateTime<Utc>,
    seed: u64,
    count: usize,
) -> Result<PatternComparison, Box<dyn std::error::Error>> {
    let names = |pattern: &LogPattern| -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
        let mut result = BTreeSet::new();
        for part in template_syntax::parse(&pattern.template) {
            if let Part::Field(spec) = part.map_err(io::Error::other)? {
                result.insert(spec.split(':').next().unwrap_or("").to_owned());
            }
        }
        Ok(result)
    };
    let first_names = names(first)?;
    let second_names = names(second)?;
    Ok(PatternComparison {
        first: test_generation(first, timestamp, seed, count)?,
        second: test_generation(second, timestamp, seed, count)?,
        common_fields: first_names.intersection(&second_names).cloned().collect(),
        first_only: first_names.difference(&second_names).cloned().collect(),
        second_only: second_names.difference(&first_names).cloned().collect(),
    })
}

#[derive(Serialize)]
pub struct BenchmarkReport {
    pub iterations: usize,
    pub total_seconds: f64,
    pub average_seconds: f64,
    pub min_seconds: f64,
    pub max_seconds: f64,
}
pub fn benchmark(
    pattern: &LogPattern,
    timestamp: DateTime<Utc>,
    seed: u64,
    iterations: usize,
) -> Result<BenchmarkReport, Box<dyn std::error::Error>> {
    if iterations == 0 {
        return Err("iterations must be greater than zero".into());
    }
    validate_pattern(pattern)?;
    let settings = prepare(pattern, &BTreeMap::new())?;
    let mut rng = StdRng::seed_from_u64(seed);
    let mut total = 0.0;
    let mut min = f64::INFINITY;
    let mut max: f64 = 0.0;
    for index in 0..iterations {
        let started = Instant::now();
        render_custom(&settings, timestamp, &mut rng, index as u64).map_err(io::Error::other)?;
        let elapsed = started.elapsed().as_secs_f64();
        total += elapsed;
        min = min.min(elapsed);
        max = max.max(elapsed);
    }
    Ok(BenchmarkReport {
        iterations,
        total_seconds: total,
        average_seconds: total / iterations as f64,
        min_seconds: min,
        max_seconds: max,
    })
}
