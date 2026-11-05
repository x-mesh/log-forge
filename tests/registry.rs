use chrono::{TimeZone, Utc};
use log_forge::{GeneratorError, GeneratorRegistry, LogGenerator, write_records};

struct Example {
    prefix: String,
    count: u32,
}

#[test]
fn cli_engine_accepts_registry_with_builtin_factories() {
    let _guard = CLI_RUN_LOCK.lock().unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("redis.log");
    let registry = GeneratorRegistry::with_builtins(42).unwrap();
    let args = vec![
        "log-forge".into(),
        "start".into(),
        "--types".into(),
        "redis".into(),
        "--output".into(),
        "file".into(),
        "--path".into(),
        path.to_string_lossy().into_owned(),
        "--total".into(),
        "2".into(),
        "--no-monitor".into(),
    ];
    log_forge::run_with_registry(args, &registry).unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 2);
}

#[test]
fn replaced_builtin_remains_a_conflict_for_cli_engine() {
    let mut registry = GeneratorRegistry::with_builtins(42).unwrap();
    assert!(registry.unregister("redis"));
    registry
        .register("redis", |_| {
            Ok(Box::new(Example {
                prefix: "custom".into(),
                count: 0,
            }))
        })
        .unwrap();
    let args = vec!["log-forge".into(), "list-types".into()];
    let error = log_forge::run_with_registry(args, &registry).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("external generator conflicts with built-in type: redis")
    );
}
impl LogGenerator for Example {
    fn generate(&mut self, time: chrono::DateTime<Utc>) -> Result<String, GeneratorError> {
        self.count += 1;
        Ok(format!(
            "{} {} {}",
            self.prefix,
            time.to_rfc3339(),
            self.count
        ))
    }
    fn pattern(&self) -> &str {
        "^test "
    }
    fn validate(&self, record: &str) -> bool {
        record.starts_with(&self.prefix)
    }
}

#[test]
fn registers_configures_and_runs_external_generator() {
    let mut registry = GeneratorRegistry::new();
    registry
        .register("example", |config| {
            let prefix = config.as_str().ok_or("prefix must be a string")?;
            Ok(Box::new(Example {
                prefix: prefix.into(),
                count: 0,
            }))
        })
        .unwrap();
    assert!(registry.register("example", |_| unreachable!()).is_err());
    assert_eq!(registry.types().collect::<Vec<_>>(), ["example"]);
    let mut instance = registry
        .create("example", &serde_yaml::Value::String("test".into()))
        .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut output = Vec::new();
    assert_eq!(
        write_records(instance.as_mut(), &mut output, [time, time]).unwrap(),
        2
    );
    let output = String::from_utf8(output).unwrap();
    assert_eq!(
        output,
        "test 2024-01-01T00:00:00+00:00 1\ntest 2024-01-01T00:00:00+00:00 2\n"
    );
    assert!(instance.validate(output.lines().next().unwrap()));
    assert!(registry.unregister("example"));
    assert!(
        registry
            .create("example", &serde_yaml::Value::Null)
            .is_err()
    );
}

#[test]
fn external_generator_runs_in_cli_engine_with_configuration() {
    let _guard = CLI_RUN_LOCK.lock().unwrap();
    let root = std::env::temp_dir().join(format!("log-forge-plugin-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let config = root.join("config.yaml");
    let output = root.join("output.log");
    std::fs::write(&config, format!("log_generator:\n  global:\n    output_format: file\n    output_path: '{}'\n    total_logs: 2\n    generation_interval: 1\n    start_time: '2024-01-01'\n  log_types:\n    example:\n      enabled: true\n      frequency: 1\n      plugin_config: custom\n", output.display())).unwrap();
    let mut registry = GeneratorRegistry::with_builtins(42).unwrap();
    registry
        .register("example", |config| {
            Ok(Box::new(Example {
                prefix: config.as_str().ok_or("prefix required")?.into(),
                count: 0,
            }))
        })
        .unwrap();
    let args = vec![
        "log-forge".into(),
        "config".into(),
        "validate".into(),
        "--file".into(),
        config.to_string_lossy().into_owned(),
    ];
    log_forge::run_with_registry(args, &registry).unwrap();
    let args = vec![
        "log-forge".into(),
        "start".into(),
        "--config".into(),
        config.to_string_lossy().into_owned(),
    ];
    log_forge::run_with_registry(args, &registry).unwrap();
    let contents = std::fs::read_to_string(output).unwrap();
    assert_eq!(
        contents,
        "custom 2024-01-01T00:00:00+00:00 1\ncustom 2024-01-01T00:00:01+00:00 2\n"
    );
    let args = vec![
        "log-forge".into(),
        "start".into(),
        "--config".into(),
        config.to_string_lossy().into_owned(),
        "--total".into(),
        "1".into(),
    ];
    log_forge::run_with_registry(args, &registry).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("output.log"))
            .unwrap()
            .lines()
            .count(),
        3
    );
    std::fs::remove_dir_all(root).unwrap();
}
static CLI_RUN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn cli_engine_preserves_structured_generator_fields_in_sync_and_async_output() {
    let _guard = CLI_RUN_LOCK.lock().unwrap();
    struct Structured;
    impl LogGenerator for Structured {
        fn generate(&mut self, _: chrono::DateTime<Utc>) -> Result<String, GeneratorError> {
            Err("text generation unavailable".into())
        }
        fn generate_entry(
            &mut self,
            time: chrono::DateTime<Utc>,
        ) -> Result<Option<log_forge::models::LogEntry>, GeneratorError> {
            let mut entry = log_forge::models::LogEntry::new(time, "example", "WARN", "event");
            entry
                .custom_fields
                .insert("request_id".into(), serde_json::json!("req-42"));
            Ok(Some(entry))
        }
        fn pattern(&self) -> &str {
            ".*"
        }
        fn validate(&self, _: &str) -> bool {
            true
        }
    }
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let mut registry = GeneratorRegistry::new();
    registry
        .register("example", |_| Ok(Box::new(Structured)))
        .unwrap();
    for option in ["--sync-io", "--async-io"] {
        let path = root.path().join(if option == "--sync-io" {
            "sync.jsonl"
        } else {
            "async.jsonl"
        });
        std::fs::write(&config, format!("log_generator:\n  global: {{output_format: json, output_path: '{}', total_logs: 2}}\n  log_types:\n    example: {{enabled: true}}\n", path.display())).unwrap();
        let args = vec![
            "log-forge".into(),
            "start".into(),
            "--config".into(),
            config.to_string_lossy().into_owned(),
            option.into(),
            "--no-monitor".into(),
        ];
        log_forge::run_with_registry(args, &registry).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(text.lines().count(), 2);
        for line in text.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["request_id"], "req-42");
            assert_eq!(value["level"], "WARN");
        }
    }
}

#[test]
fn write_records_supports_structured_generators_as_text() {
    struct Structured;
    impl LogGenerator for Structured {
        fn generate(&mut self, _: chrono::DateTime<Utc>) -> Result<String, GeneratorError> {
            Err("text generation unavailable".into())
        }
        fn generate_entry(
            &mut self,
            time: chrono::DateTime<Utc>,
        ) -> Result<Option<log_forge::models::LogEntry>, GeneratorError> {
            Ok(Some(log_forge::models::LogEntry::new(
                time, "example", "INFO", "event",
            )))
        }
        fn pattern(&self) -> &str {
            ".*"
        }
        fn validate(&self, _: &str) -> bool {
            true
        }
    }
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut output = Vec::new();
    assert_eq!(
        write_records(&mut Structured, &mut output, [time]).unwrap(),
        1
    );
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "2024-01-01T00:00:00+00:00 [INFO] event\n"
    );
}

#[test]
fn cli_can_continue_after_a_generator_error() {
    let _guard = CLI_RUN_LOCK.lock().unwrap();
    struct FailsOnce(bool);
    impl LogGenerator for FailsOnce {
        fn generate(&mut self, _: chrono::DateTime<Utc>) -> Result<String, GeneratorError> {
            if !self.0 {
                self.0 = true;
                return Err("expected temporary failure".into());
            }
            Ok("generated".into())
        }
        fn pattern(&self) -> &str {
            ".*"
        }
        fn validate(&self, _: &str) -> bool {
            true
        }
    }
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let output = root.path().join("output.log");
    std::fs::write(&config, format!("log_generator:\n  global: {{output_format: file, output_path: '{}', total_logs: 2}}\n  log_types:\n    example: {{enabled: true}}\n", output.display())).unwrap();
    let mut registry = GeneratorRegistry::new();
    registry
        .register("example", |_| Ok(Box::new(FailsOnce(false))))
        .unwrap();
    let default_args = vec![
        "log-forge".into(),
        "start".into(),
        "--config".into(),
        config.to_string_lossy().into_owned(),
        "--no-monitor".into(),
    ];
    let error = log_forge::run_with_registry(default_args, &registry).unwrap_err();
    assert!(error.to_string().contains("expected temporary failure"));
    let args = vec![
        "log-forge".into(),
        "start".into(),
        "--config".into(),
        config.to_string_lossy().into_owned(),
        "--continue-on-error".into(),
        "--no-monitor".into(),
    ];
    log_forge::run_with_registry(args, &registry).unwrap();
    assert_eq!(std::fs::read_to_string(output).unwrap().lines().count(), 2);
}

#[test]
fn cli_continue_on_error_attempts_later_output_for_structured_entries() {
    let _guard = CLI_RUN_LOCK.lock().unwrap();
    struct Structured;
    impl LogGenerator for Structured {
        fn generate(&mut self, _: chrono::DateTime<Utc>) -> Result<String, GeneratorError> {
            Err("text generation unavailable".into())
        }
        fn generate_entry(
            &mut self,
            time: chrono::DateTime<Utc>,
        ) -> Result<Option<log_forge::models::LogEntry>, GeneratorError> {
            let mut entry =
                log_forge::models::LogEntry::new(time, "example", "WARN", "structured event");
            entry.source_ip = Some("192.0.2.42".into());
            entry
                .custom_fields
                .insert("source".into(), serde_json::json!("conflicting scalar"));
            Ok(Some(entry))
        }
        fn pattern(&self) -> &str {
            ".*"
        }
        fn validate(&self, _: &str) -> bool {
            true
        }
    }
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let first = root.path().join("first.jsonl");
    let last = root.path().join("last.log");
    std::fs::write(&config, format!("log_generator:\n  global: {{total_logs: 1}}\n  log_types:\n    example: {{enabled: true}}\n  output:\n    handlers:\n      - {{type: file, file_path: '{}', format: json}}\n      - {{type: file, file_path: '{}', format: raw}}\n", first.display(), last.display())).unwrap();
    let mut registry = GeneratorRegistry::new();
    registry
        .register("example", |_| Ok(Box::new(Structured)))
        .unwrap();
    let args = vec![
        "log-forge".into(),
        "start".into(),
        "--config".into(),
        config.to_string_lossy().into_owned(),
        "--continue-on-error".into(),
        "--no-monitor".into(),
    ];
    log_forge::run_with_registry(args, &registry).unwrap();
    assert!(std::fs::read_to_string(first).unwrap().is_empty());
    let text = std::fs::read_to_string(last).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("structured event"));
}
