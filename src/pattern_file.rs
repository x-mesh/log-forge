use chrono::{DateTime, Utc};
use rand::{SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize, Serialize)]
pub struct LogPattern {
    pub name: String,
    pub template: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub fields: BTreeMap<String, PatternField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PatternField {
    Faker {
        generator: String,
    },
    Choice {
        options: Vec<String>,
    },
    RandomInt {
        #[serde(default)]
        options: BTreeMap<String, serde_yaml::Value>,
    },
    RandomFloat {
        #[serde(default)]
        options: BTreeMap<String, serde_yaml::Value>,
    },
    Counter {
        #[serde(default)]
        options: BTreeMap<String, serde_yaml::Value>,
    },
    Uuid,
}
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum PatternFile {
    Multiple { patterns: Vec<LogPattern> },
    Single(LogPattern),
}

fn read_patterns(path: &Path) -> Result<Vec<LogPattern>, Box<dyn std::error::Error>> {
    let bytes = fs::read(path)?;
    let parsed: PatternFile = if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        serde_json::from_slice(&bytes)?
    } else {
        serde_yaml::from_slice(&bytes)?
    };
    let patterns = match parsed {
        PatternFile::Single(pattern) => vec![pattern],
        PatternFile::Multiple { patterns } => patterns,
    };
    if patterns.is_empty() {
        return Err("pattern file contains no patterns".into());
    }
    let mut names = BTreeSet::new();
    if patterns.iter().any(|pattern| !names.insert(&pattern.name)) {
        return Err("pattern names must be unique".into());
    }
    Ok(patterns)
}
fn normalized_fields(
    pattern: &LogPattern,
) -> Result<BTreeMap<String, serde_yaml::Value>, Box<dyn std::error::Error>> {
    let mut fields = BTreeMap::new();
    for (name, value) in &pattern.fields {
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("invalid pattern field name".into());
        }
        let value = match value {
            PatternField::Faker { generator }
                if !generator.is_empty()
                    && generator
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                serde_yaml::Value::String(generator.clone())
            }
            PatternField::Choice { options }
                if !options.is_empty()
                    && options
                        .iter()
                        .all(|option| !option.is_empty() && !option.contains(['\n', '\r'])) =>
            {
                serde_yaml::Value::Sequence(
                    options
                        .iter()
                        .cloned()
                        .map(serde_yaml::Value::String)
                        .collect(),
                )
            }
            PatternField::RandomInt { options } => encode_options("random_int", options)?,
            PatternField::RandomFloat { options } => encode_options("random_float", options)?,
            PatternField::Counter { options } => encode_options("counter", options)?,
            PatternField::Uuid => serde_yaml::Value::String("uuid".into()),
            _ => return Err(format!("invalid pattern field: {name}").into()),
        };
        fields.insert(name.clone(), value);
    }
    Ok(fields)
}
fn encode_options(
    kind: &str,
    options: &BTreeMap<String, serde_yaml::Value>,
) -> Result<serde_yaml::Value, Box<dyn std::error::Error>> {
    let mut parts = Vec::new();
    for (key, value) in options {
        let scalar = match value {
            serde_yaml::Value::Number(number) => number.to_string(),
            serde_yaml::Value::String(value) => value.clone(),
            _ => return Err("pattern option must be a number or string".into()),
        };
        if key.is_empty()
            || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || scalar.contains([',', ':', '\n', '\r'])
        {
            return Err("invalid pattern option".into());
        }
        parts.push(format!("{key}:{scalar}"));
    }
    Ok(serde_yaml::Value::String(if parts.is_empty() {
        kind.into()
    } else {
        format!("{kind}:{}", parts.join(","))
    }))
}
pub(crate) fn validate_pattern(pattern: &LogPattern) -> Result<(), Box<dyn std::error::Error>> {
    if pattern.name.is_empty()
        || pattern.name == "."
        || pattern.name == ".."
        || !pattern
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err("invalid pattern name".into());
    }
    if pattern.template.is_empty() || pattern.template.contains(['\n', '\r']) {
        return Err("invalid pattern template".into());
    }
    let settings = prepare(pattern, &BTreeMap::new())?;
    crate::app::compat::render_custom(&settings, Utc::now(), &mut StdRng::seed_from_u64(0), 0)
        .map_err(io::Error::other)?;
    Ok(())
}
pub(crate) fn prepare(
    pattern: &LogPattern,
    overrides: &BTreeMap<String, String>,
) -> Result<crate::app::compat::GeneratorFields, Box<dyn std::error::Error>> {
    let mut fields = normalized_fields(pattern)?;
    for (name, value) in overrides {
        fields.insert(
            name.clone(),
            serde_yaml::Value::Sequence(vec![serde_yaml::Value::String(value.clone())]),
        );
    }
    let file_fields = crate::app::compat::load_file_fields(&pattern.template, &fields)?;
    let mut settings = crate::app::compat::GeneratorFields::default();
    settings.template = Some(pattern.template.clone());
    settings.fields = fields;
    settings.file_fields = file_fields;
    Ok(settings)
}

fn update_pattern_metadata(
    pattern: &mut LogPattern,
    created_at: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let now = Utc::now();
    let settings = prepare(pattern, &BTreeMap::new())?;
    pattern.sample_output = Some(
        crate::app::compat::render_custom(&settings, now, &mut StdRng::seed_from_u64(0), 0)
            .map_err(io::Error::other)?,
    );
    if pattern.created_at.is_none() {
        pattern.created_at = Some(created_at.map_or_else(|| now.to_rfc3339(), str::to_owned));
    }
    pattern.updated_at = Some(now.to_rfc3339());
    Ok(())
}

pub(crate) fn load(
    path: &Path,
    selected: Option<&str>,
) -> Result<(String, BTreeMap<String, serde_yaml::Value>), Box<dyn std::error::Error>> {
    let patterns = read_patterns(path)?;
    let pattern = if let Some(name) = selected {
        patterns
            .into_iter()
            .find(|pattern| pattern.name == name)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("pattern not found: {name}"),
                )
            })?
    } else if patterns.len() == 1 {
        patterns
            .into_iter()
            .next()
            .ok_or("pattern file contains no patterns")?
    } else {
        return Err("pattern_name is required when the file has multiple patterns".into());
    };
    validate_pattern(&pattern)?;
    Ok((pattern.template.clone(), normalized_fields(&pattern)?))
}

pub struct PatternManager {
    directory: PathBuf,
    patterns: BTreeMap<String, LogPattern>,
    sources: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Serialize)]
pub struct SamplePatternReport {
    pub created: Vec<String>,
    pub existing: Vec<String>,
}
impl PatternManager {
    pub fn create_sample_patterns(
        &mut self,
    ) -> Result<SamplePatternReport, Box<dyn std::error::Error>> {
        let patterns: Vec<LogPattern> =
            serde_json::from_str(include_str!("../data/sample_patterns.json"))?;
        for pattern in &patterns {
            validate_pattern(pattern)?;
        }
        let mut report = SamplePatternReport {
            created: Vec::new(),
            existing: Vec::new(),
        };
        for pattern in patterns {
            if self.patterns.contains_key(&pattern.name) {
                report.existing.push(pattern.name);
            } else {
                let name = pattern.name.clone();
                self.add(pattern)?;
                report.created.push(name);
            }
        }
        Ok(report)
    }

    pub fn open(directory: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let directory = directory.as_ref().to_owned();
        fs::create_dir_all(&directory)?;
        let mut paths = fs::read_dir(&directory)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?;
        paths.sort();
        let mut manager = Self {
            directory,
            patterns: BTreeMap::new(),
            sources: BTreeMap::new(),
        };
        for path in paths {
            if !fs::symlink_metadata(&path)?.file_type().is_file()
                || !path.extension().is_some_and(|ext| {
                    ["json", "yaml", "yml"]
                        .iter()
                        .any(|suffix| ext.eq_ignore_ascii_case(suffix))
                })
            {
                continue;
            }
            for pattern in read_patterns(&path)? {
                validate_pattern(&pattern)?;
                if manager.patterns.contains_key(&pattern.name) {
                    return Err(format!("duplicate pattern: {}", pattern.name).into());
                }
                manager.sources.insert(pattern.name.clone(), path.clone());
                manager.patterns.insert(pattern.name.clone(), pattern);
            }
        }
        Ok(manager)
    }
    pub fn get(&self, name: &str) -> Option<&LogPattern> {
        self.patterns.get(name)
    }
    pub fn list<'a>(&'a self, category: Option<&'a str>) -> impl Iterator<Item = &'a LogPattern> {
        self.patterns
            .values()
            .filter(move |pattern| category.is_none_or(|category| pattern.category == category))
    }
    pub fn categories(&self) -> Vec<&str> {
        self.patterns
            .values()
            .map(|pattern| pattern.category.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub fn generate(
        &self,
        name: &str,
        timestamp: DateTime<Utc>,
        seed: u64,
        index: u64,
    ) -> Result<String, Box<dyn std::error::Error>> {
        self.generate_with_values(name, timestamp, seed, index, &BTreeMap::new())
    }
    pub fn generate_with_values(
        &self,
        name: &str,
        timestamp: DateTime<Utc>,
        seed: u64,
        index: u64,
        overrides: &BTreeMap<String, String>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let pattern = self.get(name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("pattern not found: {name}"),
            )
        })?;
        let settings = prepare(pattern, overrides)?;
        Ok(crate::app::compat::render_custom(
            &settings,
            timestamp,
            &mut StdRng::seed_from_u64(seed),
            index,
        )
        .map_err(io::Error::other)?)
    }
    pub fn add(&mut self, mut pattern: LogPattern) -> Result<(), Box<dyn std::error::Error>> {
        validate_pattern(&pattern)?;
        if self.patterns.contains_key(&pattern.name) {
            return Err(format!("pattern already exists: {}", pattern.name).into());
        }
        update_pattern_metadata(&mut pattern, None)?;
        let path = self.directory.join(format!("{}.json", pattern.name));
        let bytes = serde_json::to_vec_pretty(&pattern)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(&path)
            .map_err(|error| error.error)?;
        self.sources.insert(pattern.name.clone(), path);
        self.patterns.insert(pattern.name.clone(), pattern);
        Ok(())
    }
    fn managed_path(&self, name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let source = self.sources.get(name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("pattern not found: {name}"),
            )
        })?;
        let expected = self.directory.join(format!("{name}.json"));
        if source != &expected || self.sources.values().filter(|path| *path == source).count() != 1
        {
            return Err("update or remove requires a dedicated JSON pattern file".into());
        }
        if !fs::symlink_metadata(source)?.file_type().is_file() {
            return Err("pattern file is not a regular file".into());
        }
        Ok(expected)
    }
    pub fn update(&mut self, mut pattern: LogPattern) -> Result<(), Box<dyn std::error::Error>> {
        validate_pattern(&pattern)?;
        let path = self.managed_path(&pattern.name)?;
        let created_at = self
            .patterns
            .get(&pattern.name)
            .and_then(|existing| existing.created_at.as_deref());
        update_pattern_metadata(&mut pattern, created_at)?;
        let bytes = serde_json::to_vec_pretty(&pattern)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&path).map_err(|error| error.error)?;
        self.patterns.insert(pattern.name.clone(), pattern);
        Ok(())
    }
    pub fn remove(&mut self, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let path = self.managed_path(name)?;
        fs::remove_file(path)?;
        self.sources.remove(name);
        self.patterns.remove(name);
        Ok(())
    }
    pub fn export(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let file = PatternFile::Multiple {
            patterns: self.patterns.values().cloned().collect(),
        };
        let bytes = if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            serde_json::to_vec_pretty(&file)?
        } else {
            serde_yaml::to_string(&file)?.into_bytes()
        };
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(path)
            .map_err(|error| error.error)?;
        Ok(())
    }
    pub fn import(&mut self, path: &Path) -> Result<usize, Box<dyn std::error::Error>> {
        let patterns = read_patterns(path)?;
        for pattern in &patterns {
            validate_pattern(pattern)?;
            if self.patterns.contains_key(&pattern.name)
                || self
                    .directory
                    .join(format!("{}.json", pattern.name))
                    .exists()
            {
                return Err(format!("pattern already exists: {}", pattern.name).into());
            }
        }
        let count = patterns.len();
        for (index, pattern) in patterns.into_iter().enumerate() {
            if let Err(error) = self.add(pattern) {
                return Err(format!("import stopped after {index} patterns: {error}").into());
            }
        }
        Ok(count)
    }
}
