use chrono::Local;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
};

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
    level: String,
    format: String,
    file: Option<PathBuf>,
    loggers: BTreeMap<String, String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            level: "INFO".into(),
            format: "%(asctime)s - %(name)s - %(levelname)s - %(message)s".into(),
            file: None,
            loggers: BTreeMap::new(),
        }
    }
}
fn level(value: &str) -> io::Result<u8> {
    match value.to_ascii_uppercase().as_str() {
        "DEBUG" => Ok(10),
        "INFO" => Ok(20),
        "WARNING" | "WARN" => Ok(30),
        "ERROR" => Ok(40),
        "CRITICAL" | "FATAL" => Ok(50),
        _ => Err(io::Error::other(format!("invalid logging level: {value}"))),
    }
}
impl Config {
    pub fn validate(&self) -> io::Result<()> {
        level(&self.level)?;
        for (name, value) in &self.loggers {
            if name.is_empty() || name.chars().any(char::is_whitespace) {
                return Err(io::Error::other("invalid logger name"));
            }
            level(value)?;
        }
        if self.format.contains(['\n', '\r']) {
            return Err(io::Error::other("invalid logging format"));
        }
        let mut remainder = self.format.as_str();
        while let Some(index) = remainder.find('%') {
            remainder = &remainder[index..];
            if let Some(rest) = remainder.strip_prefix("%%") {
                remainder = rest;
                continue;
            }
            let field = [
                "%(asctime)s",
                "%(name)s",
                "%(levelname)s",
                "%(funcName)s",
                "%(lineno)d",
                "%(message)s",
            ]
            .iter()
            .find(|field| remainder.starts_with(**field));
            remainder = remainder
                .strip_prefix(
                    field.ok_or_else(|| io::Error::other("unsupported logging format field"))?,
                )
                .unwrap();
        }
        if self
            .file
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err(io::Error::other("logging file must not be empty"));
        }
        Ok(())
    }
}
pub(crate) struct InternalLog<'a> {
    config: &'a Config,
    file: Option<fs::File>,
}
impl<'a> InternalLog<'a> {
    pub fn open(config: &'a Config) -> io::Result<Self> {
        config.validate()?;
        let file = if let Some(path) = &config.file {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            Some(OpenOptions::new().create(true).append(true).open(path)?)
        } else {
            None
        };
        Ok(Self { config, file })
    }
    pub fn info(&mut self, message: &str, line: u32) -> io::Result<()> {
        let name = "log_generator.core";
        let selected = self
            .config
            .loggers
            .iter()
            .filter(|(prefix, _)| {
                name == prefix.as_str()
                    || name
                        .strip_prefix(prefix.as_str())
                        .is_some_and(|suffix| suffix.starts_with('.'))
            })
            .max_by_key(|(prefix, _)| prefix.len())
            .map_or(self.config.level.as_str(), |(_, value)| value.as_str());
        if level("INFO")? < level(selected)? {
            return Ok(());
        }
        let mut rendered = String::new();
        let mut remainder = self.config.format.as_str();
        while let Some(index) = remainder.find('%') {
            rendered.push_str(&remainder[..index]);
            remainder = &remainder[index..];
            let fields = [
                ("%%", "%".to_owned()),
                (
                    "%(asctime)s",
                    Local::now().format("%Y-%m-%d %H:%M:%S,%3f").to_string(),
                ),
                ("%(name)s", name.into()),
                ("%(levelname)s", "INFO".into()),
                ("%(funcName)s", "run".into()),
                ("%(lineno)d", line.to_string()),
                ("%(message)s", message.into()),
            ];
            let (token, value) = fields
                .iter()
                .find(|(token, _)| remainder.starts_with(token))
                .ok_or_else(|| io::Error::other("unsupported logging format field"))?;
            rendered.push_str(value);
            remainder = &remainder[token.len()..];
        }
        rendered.push_str(remainder);
        if let Some(file) = &mut self.file {
            writeln!(file, "{rendered}")?;
            file.flush()
        } else {
            writeln!(io::stderr().lock(), "{rendered}")
        }
    }
}
