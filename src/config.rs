use crate::{GeneratorRegistry, app::compat::ConfigurationHandle};
use std::{error::Error, fs::OpenOptions, io::Write, path::Path};

pub struct Configuration {
    inner: ConfigurationHandle,
}

impl Configuration {
    pub fn load(path: &Path, registry: &GeneratorRegistry) -> Result<Self, Box<dyn Error>> {
        let source = std::fs::read_to_string(path)?;
        let absolute_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        Ok(Self {
            inner: ConfigurationHandle::parse(&source, Some(&absolute_path), registry, false)?,
        })
    }

    pub fn load_with_original_defaults(
        path: &Path,
        registry: &GeneratorRegistry,
    ) -> Result<Self, Box<dyn Error>> {
        let source = std::fs::read_to_string(path)?;
        let absolute_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        Ok(Self {
            inner: ConfigurationHandle::parse(&source, Some(&absolute_path), registry, true)?,
        })
    }

    pub fn parse(source: &str, registry: &GeneratorRegistry) -> Result<Self, Box<dyn Error>> {
        let path = std::env::current_dir()?.join("log-forge-inline.yaml");
        Ok(Self {
            inner: ConfigurationHandle::parse(source, Some(&path), registry, false)?,
        })
    }

    pub fn defaults(registry: &GeneratorRegistry) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            inner: ConfigurationHandle::parse("{}", None, registry, true)?,
        })
    }

    pub fn global(&self) -> Result<serde_yaml::Value, serde_yaml::Error> {
        self.inner.global()
    }

    pub fn log_type(&self, name: &str) -> Result<Option<serde_yaml::Value>, serde_yaml::Error> {
        self.inner.log_type(name)
    }

    pub fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        self.inner.to_yaml()
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        self.inner.to_json()
    }

    pub fn save(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        let contents = if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            self.to_json()?
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"))
        {
            self.to_yaml()?
        } else {
            return Err("configuration path must end in .json, .yaml, or .yml".into());
        };
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(contents.as_bytes())?;
        file.write_all(b"\n")?;
        Ok(())
    }
}
