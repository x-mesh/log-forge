use chrono::{DateTime, Utc};
pub mod analysis;
mod analysis_cli;
mod app;
pub mod config;
#[cfg(unix)]
mod control;
#[cfg(windows)]
#[path = "control_windows.rs"]
mod control;
#[cfg(any(unix, windows, test))]
mod control_state;
pub mod engine;
pub mod error_library;
pub mod frequency;
mod goloop;
mod haproxy;
mod http_output;
mod internal_log;
mod json_output;
mod microservice;
pub mod models;
mod monitor;
mod network_output;
pub mod output;
pub mod pattern_file;
pub mod pattern_validation;
mod patterns;
pub mod performance;
mod process_plugin;
mod prometheus;
pub mod scenario;
mod template_frequency;
mod template_syntax;
mod tracing;
mod validate_cli;
pub mod validation;
mod weights;
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    io::{self, Write},
};

pub(crate) fn access_log_content<'a>(kind: &str, record: &'a str) -> &'a str {
    if kind == "apache_access"
        && let Some((_, rest)) = record.split_once(' ')
        && record
            .split_whitespace()
            .nth(4)
            .is_some_and(|field| field.starts_with('['))
    {
        return rest;
    }
    record
}

pub type GeneratorError = Box<dyn Error + Send + Sync>;

#[derive(Clone, Debug)]
pub struct GeneratedRecord {
    pub message: String,
    pub level: Option<String>,
}

pub const BUILTIN_GENERATOR_TYPES: &[&str] = &[
    "nginx_access",
    "nginx_error",
    "apache_access",
    "apache_error",
    "django_request",
    "django_sql",
    "docker",
    "kubernetes_pod",
    "kubernetes_event",
    "mysql_query",
    "mysql_error",
    "mysql_slow",
    "postgresql",
    "postgresql_slow",
    "syslog",
    "fastapi",
    "goloop",
    "redis",
    "ssh_auth",
    "kafka_broker",
    "haproxy",
];

pub fn create_builtin(
    name: &str,
    config: &serde_yaml::Value,
    seed: u64,
) -> Result<Box<dyn LogGenerator>, GeneratorError> {
    app::compat::create_builtin(name, config, seed)
}

pub trait LogGenerator: Send {
    fn generate(&mut self, timestamp: DateTime<Utc>) -> Result<String, GeneratorError>;
    fn generate_entry(
        &mut self,
        _timestamp: DateTime<Utc>,
    ) -> Result<Option<models::LogEntry>, GeneratorError> {
        // Existing generators return text records.
        Ok(None)
    }
    fn generate_record(
        &mut self,
        timestamp: DateTime<Utc>,
    ) -> Result<GeneratedRecord, GeneratorError> {
        Ok(GeneratedRecord {
            message: self.generate(timestamp)?,
            level: None,
        })
    }
    fn pattern(&self) -> &str;
    fn validate(&self, record: &str) -> bool;
    fn metadata(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
    fn structured_metadata(&self) -> serde_json::Value {
        serde_json::json!(self.metadata())
    }
    fn finish(&mut self) -> Result<(), GeneratorError> {
        Ok(())
    }
}

type Factory =
    dyn Fn(&serde_yaml::Value) -> Result<Box<dyn LogGenerator>, GeneratorError> + Send + Sync;

#[derive(Default)]
pub struct GeneratorRegistry {
    factories: BTreeMap<String, Box<Factory>>,
    builtins: BTreeSet<String>,
}

impl GeneratorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_builtins(seed: u64) -> Result<Self, GeneratorError> {
        let mut registry = Self::new();
        for &name in BUILTIN_GENERATOR_TYPES {
            registry.register(name, move |config| create_builtin(name, config, seed))?;
            registry.builtins.insert(name.to_owned());
        }
        Ok(registry)
    }

    pub fn register<F>(&mut self, name: impl Into<String>, factory: F) -> Result<(), GeneratorError>
    where
        F: Fn(&serde_yaml::Value) -> Result<Box<dyn LogGenerator>, GeneratorError>
            + Send
            + Sync
            + 'static,
    {
        let name = name.into();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(
                io::Error::new(io::ErrorKind::InvalidInput, "invalid generator name").into(),
            );
        }
        if self.factories.contains_key(&name) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("generator already registered: {name}"),
            )
            .into());
        }
        self.factories.insert(name, Box::new(factory));
        Ok(())
    }

    pub fn unregister(&mut self, name: &str) -> bool {
        self.builtins.remove(name);
        self.factories.remove(name).is_some()
    }

    pub(crate) fn is_builtin(&self, name: &str) -> bool {
        self.builtins.contains(name)
    }

    pub fn types(&self) -> impl Iterator<Item = &str> {
        self.factories.keys().map(String::as_str)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.factories.contains_key(name)
    }

    pub fn create(
        &self,
        name: &str,
        config: &serde_yaml::Value,
    ) -> Result<Box<dyn LogGenerator>, GeneratorError> {
        let factory = self.factories.get(name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("unknown generator: {name}"),
            )
        })?;
        factory(config)
    }
}

pub fn run_with_registry(
    args: impl IntoIterator<Item = String>,
    registry: &GeneratorRegistry,
) -> Result<(), Box<dyn Error>> {
    app::run(args.into_iter().collect(), registry)
}

pub fn write_records<W: Write>(
    generator: &mut dyn LogGenerator,
    output: &mut W,
    timestamps: impl IntoIterator<Item = DateTime<Utc>>,
) -> Result<u64, GeneratorError> {
    let mut written = 0_u64;
    for timestamp in timestamps {
        let record = if let Some(entry) = generator.generate_entry(timestamp)? {
            entry.to_string()
        } else {
            generator.generate(timestamp)?
        };
        output.write_all(record.as_bytes())?;
        if !record.ends_with('\n') {
            output.write_all(b"\n")?;
        }
        written = written
            .checked_add(1)
            .ok_or_else(|| io::Error::other("record count overflow"))?;
    }
    output.flush()?;
    Ok(written)
}
