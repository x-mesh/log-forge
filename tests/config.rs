use log_forge::{GeneratorRegistry, config::Configuration};

#[test]
fn loads_checks_and_saves_configuration_without_overwrite() {
    let registry = GeneratorRegistry::new();
    let defaults = Configuration::defaults(&registry).unwrap();
    assert!(defaults.log_type("nginx_access").unwrap().is_some());
    assert!(defaults.log_type("missing").unwrap().is_none());
    assert_eq!(
        defaults.global().unwrap()["output_format"].as_str(),
        Some("multi_file")
    );
    let temp = tempfile::tempdir().unwrap();
    for extension in ["yaml", "json"] {
        let path = temp.path().join(format!("config.{extension}"));
        defaults.save(&path).unwrap();
        let loaded = Configuration::load(&path, &registry).unwrap();
        assert_eq!(
            loaded.global().unwrap()["output_format"].as_str(),
            Some("multi_file")
        );
        assert!(loaded.log_type("nginx_access").unwrap().is_some());
        assert!(defaults.save(&path).is_err());
    }
    assert!(defaults.save(&temp.path().join("config.txt")).is_err());
    let invalid = "log_generator:\n  global:\n    output_format: missing\n";
    assert!(Configuration::parse(invalid, &registry).is_err());
    let unknown = "log_generator:\n  global:\n    unknown_setting: true\n";
    assert!(Configuration::parse(unknown, &registry).is_err());
}

#[test]
fn save_creates_parent_directories_without_overwriting() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nested/config/settings.yaml");
    let config = Configuration::defaults(&GeneratorRegistry::new()).unwrap();
    config.save(&path).unwrap();
    assert!(Configuration::load(&path, &GeneratorRegistry::new()).is_ok());
    let before = std::fs::read(&path).unwrap();
    assert!(config.save(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn load_rejects_unsupported_extensions_and_accepts_uppercase() {
    let root = tempfile::tempdir().unwrap();
    let registry = GeneratorRegistry::new();
    for extension in ["txt", "", "toml"] {
        let path = root.path().join(format!("config{extension}"));
        std::fs::write(&path, "log_generator: {}\n").unwrap();
        assert!(
            Configuration::load(&path, &registry).is_err(),
            "{}",
            path.display()
        );
        assert!(Configuration::load_with_original_defaults(&path, &registry).is_err());
    }
    for (extension, contents) in [
        ("YAML", "log_generator: {}\n"),
        ("JSON", "{\"log_generator\":{}}"),
    ] {
        let path = root.path().join(format!("config.{extension}"));
        std::fs::write(&path, contents).unwrap();
        assert!(
            Configuration::load(&path, &registry).is_ok(),
            "{}",
            path.display()
        );
    }
}

#[test]
fn library_defaults_match_original_configuration() {
    let config = Configuration::defaults(&GeneratorRegistry::new()).unwrap();
    let original: serde_json::Value =
        serde_json::from_str(include_str!("../data/original_default_config.json")).unwrap();
    for name in ["nginx_access", "nginx_error", "syslog", "fastapi", "goloop"] {
        let actual = config.log_type(name).unwrap().unwrap();
        assert_eq!(actual["enabled"].as_bool(), Some(true));
        assert_eq!(
            actual["frequency"].as_f64(),
            original["log_generator"]["log_types"][name]["frequency"].as_f64()
        );
    }
    assert_eq!(
        config.log_type("custom").unwrap().unwrap()["enabled"].as_bool(),
        Some(false)
    );
}

#[test]
fn saves_relative_pattern_reference_for_use_from_another_directory() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    let output = root.path().join("output");
    std::fs::create_dir(&input).unwrap();
    std::fs::create_dir(&output).unwrap();
    let pattern = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("demo_patterns/custom_web_access.json");
    std::fs::copy(pattern, input.join("pattern.json")).unwrap();
    let source = input.join("config.yaml");
    std::fs::write(&source, "log_generator:\n  global:\n    output_format: console\n    total_logs: 1\n  log_types:\n    custom:\n      enabled: true\n      pattern_file: pattern.json\n").unwrap();
    let current = std::env::current_dir().unwrap();
    let mut common = current.as_path();
    let mut relative_source = std::path::PathBuf::new();
    while !source.starts_with(common) {
        let Some(parent) = common.parent() else {
            eprintln!("relative-path test requires the temporary directory on the current volume");
            return;
        };
        relative_source.push("..");
        common = parent;
    }
    relative_source.push(source.strip_prefix(common).unwrap());
    let config = Configuration::load(&relative_source, &GeneratorRegistry::new()).unwrap();
    let saved = output.join("config.yaml");
    config.save(&saved).unwrap();
    assert!(Configuration::load(&saved, &GeneratorRegistry::new()).is_ok());
}

#[test]
fn original_default_merge_preserves_nested_settings() {
    let root = tempfile::tempdir().unwrap();
    let yaml = root.path().join("config.yaml");
    let json = root.path().join("config.json");
    std::fs::write(&yaml, "log_generator:\n  global:\n    output_format: console\n  log_types:\n    nginx_access:\n      custom_fields:\n        status_codes: {503: 1.0}\n").unwrap();
    std::fs::write(&json, r#"{"log_generator":{"global":{"output_format":"console"},"log_types":{"nginx_access":{"custom_fields":{"status_codes":{"503":1.0}}}}}}"#).unwrap();
    for path in [&yaml, &json] {
        let config =
            Configuration::load_with_original_defaults(path, &GeneratorRegistry::new()).unwrap();
        assert!(config.log_type("syslog").unwrap().is_some());
        assert_eq!(
            config.global().unwrap()["output_format"].as_str(),
            Some("console")
        );
        let settings = config.log_type("nginx_access").unwrap().unwrap();
        assert_eq!(
            settings["custom_fields"]["status_codes"][200].as_f64(),
            Some(0.7)
        );
        assert_eq!(
            settings["custom_fields"]["status_codes"][503].as_f64(),
            Some(1.0)
        );
        assert_eq!(settings["frequency"].as_f64(), Some(0.4));
        let native = Configuration::load(path, &GeneratorRegistry::new()).unwrap();
        assert!(native.log_type("syslog").unwrap().is_none());
    }
    std::fs::write(&yaml, "").unwrap();
    let defaults =
        Configuration::load_with_original_defaults(&yaml, &GeneratorRegistry::new()).unwrap();
    assert!(defaults.log_type("syslog").unwrap().is_some());
    std::fs::write(&json, "{invalid: true}").unwrap();
    assert!(Configuration::load_with_original_defaults(&json, &GeneratorRegistry::new()).is_err());
}

#[test]
fn original_defaults_accept_explicit_aliases_and_pattern_files() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("config.yaml");
    let original = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("demo_patterns/custom_web_access.json");
    std::fs::copy(original, root.path().join("pattern.json")).unwrap();
    std::fs::write(&source, "log_generator:\n  log_types:\n    goloop:\n      module_weights: {CS: 1}\n    fastapi:\n      custom_fields:\n        log_levels: {INFO: 1}\n        endpoints: ['/only']\n    custom:\n      enabled: true\n      pattern_file: pattern.json\n").unwrap();
    let config =
        Configuration::load_with_original_defaults(&source, &GeneratorRegistry::new()).unwrap();
    let goloop = config.log_type("goloop").unwrap().unwrap();
    assert!(goloop["modules"].is_null());
    assert_eq!(goloop["module_weights"]["CS"].as_f64(), Some(1.0));
    let fastapi = config.log_type("fastapi").unwrap().unwrap();
    assert!(fastapi["log_levels"].is_null());
    assert_eq!(
        fastapi["custom_fields"]["endpoints"][0].as_str(),
        Some("/only")
    );
    assert!(config.log_type("custom").unwrap().unwrap()["template"].is_null());
    let explicit_conflict = "log_generator:\n  log_types:\n    goloop:\n      modules: {CS: 1}\n      module_weights: {BM: 1}\n";
    std::fs::write(&source, explicit_conflict).unwrap();
    assert!(
        Configuration::load_with_original_defaults(&source, &GeneratorRegistry::new()).is_err()
    );
}
