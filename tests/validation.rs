use log_forge::validation::LogValidator;

#[test]
fn haproxy_accepts_syslog_prefix_and_captured_headers() {
    let validator = LogValidator::new("haproxy").unwrap();
    let record = r#"Feb  6 12:14:14 edge haproxy[1234]: 192.0.2.1:33445 [06/Feb/2024:12:14:14.655] http-in app/srv1 10/0/2/3/15 503 200 - - SC-- 1/1/1/1/0 0/0 {example.test} {} "GET /health HTTP/1.1""#;
    assert!(validator.validate(record).is_valid);
    assert!(
        !validator
            .validate(&record.replace("503 200", "999 200"))
            .is_valid
    );
    assert!(
        !validator
            .validate(&record.replace("10/0/2/3/15", "bad"))
            .is_valid
    );
    let mut analyzer = log_forge::analysis::LogAnalyzer::new("haproxy").unwrap();
    analyzer.observe(record);
    assert_eq!(analyzer.statistics().status_codes.get("503"), Some(&1));
    let no_response = record.replace("503 200", "000 200");
    assert!(validator.validate(&no_response).is_valid);
    analyzer.observe(&no_response);
    assert!(!analyzer.statistics().status_codes.contains_key("000"));
}

#[test]
fn validation_result_preserves_original_record() {
    let validator = LogValidator::new("nginx_access").unwrap();
    let record = "invalid 로그\nsecond line";
    let result = validator.validate(record);
    assert!(!result.is_valid);
    assert_eq!(result.log_entry, record);
    assert_eq!(serde_json::to_value(result).unwrap()["log_entry"], record);
}

#[test]
fn validation_statistics_collect_multiple_types_and_errors() {
    let mut stats = log_forge::validation::ValidationStatistics::default();
    assert_eq!(stats.success_rate(), 0.0);
    assert_eq!(stats.type_success_rate("missing"), 0.0);
    let nginx = LogValidator::new("nginx_access").unwrap();
    let redis = LogValidator::new("redis").unwrap();
    let results = [
        nginx
            .validate("192.168.1.100 - - [25/Dec/2023:10:00:00 +0000] \"GET / HTTP/1.1\" 200 1234"),
        nginx.validate("invalid"),
        redis.validate("123:M 02 Jan 2024 13:14:17.000 * Ready"),
    ];
    for result in &results {
        stats.add_result(result);
    }
    assert_eq!(
        (stats.total_validated, stats.valid_logs, stats.invalid_logs),
        (3, 2, 1)
    );
    assert_eq!(stats.success_rate(), 2.0 / 3.0);
    assert_eq!(stats.type_success_rate("nginx_access"), 0.5);
    assert_eq!(stats.type_success_rate("redis"), 1.0);
    assert_eq!(
        stats.common_errors.values().sum::<u64>(),
        results[1].errors.len() as u64
    );
    assert_eq!(
        stats.average_quality_score,
        results
            .iter()
            .map(|result| result.quality_score)
            .sum::<f64>()
            / 3.0
    );
    let json = serde_json::to_value(stats).unwrap();
    assert_eq!(json["validation_by_type"]["nginx_access"]["invalid"], 1);
}

#[test]
fn validation_statistics_count_each_error_and_handle_empty_state() {
    let mut stats = log_forge::validation::ValidationStatistics::default();
    let empty = serde_json::to_value(&stats).unwrap();
    assert_eq!(empty["total_validated"], 0);
    assert_eq!(empty["common_errors"], serde_json::json!({}));
    assert_eq!(stats.success_rate(), 0.0);
    let validator = LogValidator::new("redis").unwrap();
    let mut first = validator.validate("invalid");
    first.errors = vec!["invalid format".to_owned(), "missing field".to_owned()];
    let mut second = validator.validate("invalid again");
    second.errors = vec!["missing field".to_owned()];
    stats.add_result(&first);
    stats.add_result(&second);
    assert_eq!(stats.common_errors["invalid format"], 1);
    assert_eq!(stats.common_errors["missing field"], 2);
    assert_eq!(stats.type_success_rate("redis"), 0.0);
}

#[test]
fn validation_statistics_track_quality_buckets_and_reset() {
    let validator = LogValidator::new("redis").unwrap();
    let mut stats = log_forge::validation::ValidationStatistics::default();
    for score in [0.0, 49.9, 50.0, 69.9, 70.0, 89.9, 90.0, 100.0] {
        let mut result = validator.validate("invalid");
        result.quality_score = score;
        stats.add_result(&result);
    }
    for bucket in [
        "excellent (90-100)",
        "good (70-89)",
        "fair (50-69)",
        "poor (0-49)",
    ] {
        assert_eq!(stats.quality_distribution[bucket], 2);
    }
    stats.reset();
    assert_eq!(stats.total_validated, 0);
    assert!(stats.quality_distribution.is_empty());
    assert_eq!(stats.average_quality_score, 0.0);
}

#[test]
fn validation_summary_limits_common_errors_and_reports_rates() {
    let validator = LogValidator::new("redis").unwrap();
    let mut stats = log_forge::validation::ValidationStatistics::default();
    let mut result = validator.validate("invalid");
    result.errors = (0..12).map(|index| format!("error{index:02}")).collect();
    stats.add_result(&result);
    let summary = serde_json::to_value(stats.summary()).unwrap();
    assert_eq!(summary["total_validated"], 1);
    assert_eq!(summary["success_rate"], 0.0);
    assert_eq!(summary["common_errors"].as_object().unwrap().len(), 10);
    assert_eq!(summary["quality_distribution"]["poor (0-49)"], 1);
    assert_eq!(stats.common_errors.len(), 12);
}

#[test]
fn validation_summary_keeps_first_seen_errors_when_counts_tie() {
    let validator = LogValidator::new("redis").unwrap();
    let mut stats = log_forge::validation::ValidationStatistics::default();
    let mut result = validator.validate("invalid");
    result.errors = (0..12)
        .rev()
        .map(|index| format!("error{index:02}"))
        .collect();
    stats.add_result(&result);
    let summary = stats.summary();
    assert_eq!(summary.common_errors.len(), 10);
    assert!(summary.common_errors.contains_key("error11"));
    assert!(summary.common_errors.contains_key("error02"));
    assert!(!summary.common_errors.contains_key("error01"));
    assert!(!summary.common_errors.contains_key("error00"));
}
use std::process::Command;

#[test]
fn cli_validates_and_analyzes_custom_types_with_patterns() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("custom.log");
    std::fs::write(&path, "event=login\ninvalid record\n").unwrap();
    let run = |command: &str, kind: &str, pattern: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([command, "--input"])
            .arg(&path)
            .args(["--type", kind, "--pattern", pattern])
            .output()
            .unwrap()
    };
    let validated = run("validate", "custom", r"^event=\w+$");
    assert!(!validated.status.success());
    assert!(String::from_utf8_lossy(&validated.stdout).contains("validated=2 valid=1 invalid=1"));
    let analyzed = run("analyze", "custom", r"^event=\w+$");
    assert!(
        analyzed.status.success(),
        "{}",
        String::from_utf8_lossy(&analyzed.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&analyzed.stdout).unwrap();
    assert_eq!(data["custom"]["valid_logs"], 1);
    assert_eq!(data["custom"]["invalid_logs"], 1);
    assert!(!run("validate", "custom", "(").status.success());
    std::fs::write(&path, "event=login\n").unwrap();
    assert!(
        run("validate", "nginx_access", r"^event=\w+$")
            .status
            .success()
    );
}

#[test]
fn validates_custom_formats_and_required_fields() {
    let mut validator = LogValidator::with_pattern("custom", r"^event=\w+").unwrap();
    validator.add_pattern("legacy", r"^LEGACY ").unwrap();
    validator
        .add_required_field("trace_id", r"\btrace_id=[0-9a-f]{4}\b")
        .unwrap();
    let current = validator.validate("event=login trace_id=abcd");
    assert!(current.is_valid);
    assert!(!current.pattern_matches["legacy"]);
    assert!(current.field_completeness["trace_id"]);
    let legacy = validator.validate("LEGACY event login");
    assert!(legacy.is_valid);
    assert!(legacy.pattern_matches["legacy"]);
    assert!(!legacy.field_completeness["trace_id"]);
    assert!(
        legacy
            .warnings
            .iter()
            .any(|warning| warning.contains("trace_id"))
    );
    assert!(!validator.validate("invalid").is_valid);
    assert!(validator.add_pattern("broken", "(").is_err());
    assert!(validator.add_required_field("broken", "(").is_err());
    assert!(LogValidator::with_pattern("custom", "(").is_err());
}

#[test]
fn validates_original_examples_and_reports_invalid_values() {
    let validator = LogValidator::new("nginx_access").unwrap();
    let valid = validator
        .validate("192.168.1.100 - - [25/Dec/2023:10:00:00 +0000] \"GET / HTTP/1.1\" 200 1234");
    assert!(valid.is_valid);
    assert!(valid.field_completeness.values().all(|present| *present));
    assert!(valid.quality_score > 90.0);
    let invalid = validator.validate("invalid log format");
    assert!(!invalid.is_valid);
    assert!(invalid.quality_score < 30.0);
    let warning = validator
        .validate("999.999.999.999 - - [25/Dec/2023:10:00:00 +0000] \"GET / HTTP/1.1\" 700 1234");
    assert!(warning.warnings.iter().any(|w| w.contains("IP address")));
    assert!(warning.warnings.iter().any(|w| w.contains("status code")));
    assert!(LogValidator::new("unknown").is_err());
    assert!(
        !LogValidator::new("docker")
            .unwrap()
            .validate("{\"log\":\"only\"}")
            .is_valid
    );
    let goloop = LogValidator::new("goloop").unwrap();
    assert!(
        goloop
            .validate(
                r#"{"level":"D","timestamp":"20240101-00:00:00.000000","module":"CS","msg":"ok"}"#
            )
            .is_valid
    );
    assert!(!goloop.validate(r#"{"level":"D"}"#).is_valid);
}

#[test]
fn validates_all_generated_builtin_formats() {
    for kind in [
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
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "console",
                "--types",
                kind,
                "--total",
                "1",
                "--start-time",
                "2024-01-01",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let value = String::from_utf8(result.stdout).unwrap();
        let validation = LogValidator::new(kind).unwrap().validate(value.trim_end());
        assert!(
            validation.is_valid,
            "{kind}: {}: {:?}",
            value, validation.errors
        );
    }
}

#[test]
fn cli_validates_directory_and_reports_invalid_lines() {
    let root = std::env::temp_dir().join(format!("log-forge-validation-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("nginx_access_20240101.log");
    std::fs::write(
        &input,
        "192.168.1.100 - - [25/Dec/2023:10:00:00 +0000] \"GET / HTTP/1.1\" 200 1234\ninvalid log\n",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["validate", "--input", root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("validated=2 valid=1 invalid=1")
    );
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("nginx_access_20240101.log:2")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_validates_multiline_mysql_slow_records() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-validation-slow-{}.log",
        std::process::id()
    ));
    let generate = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "file",
            "--path",
            path.to_str().unwrap(),
            "--types",
            "mysql_slow",
            "--total",
            "3",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(generate.status.success());
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "validate",
            "--input",
            path.to_str().unwrap(),
            "--type",
            "mysql_slow",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("validated=3 valid=3 invalid=0")
    );
    let analyzed = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "analyze",
            "--input",
            path.to_str().unwrap(),
            "--type",
            "mysql_slow",
        ])
        .output()
        .unwrap();
    assert!(
        analyzed.status.success(),
        "{}",
        String::from_utf8_lossy(&analyzed.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&analyzed.stdout).unwrap();
    assert_eq!(stats["mysql_slow"]["total_logs"], 3);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn cli_groups_django_tracebacks_as_records() {
    let root = std::env::temp_dir().join(format!(
        "log-forge-django-validation-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("django.log");
    let config = root.join("config.yaml");
    std::fs::write(&config, format!("log_generator:\n  global:\n    output_format: file\n    output_path: '{}'\n    total_logs: 2\n  log_types:\n    django_request:\n      enabled: true\n      log_levels: {{ERROR: 1}}\n", path.display())).unwrap();
    let generated = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "validate",
            "--input",
            path.to_str().unwrap(),
            "--type",
            "django_request",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("validated=2 valid=2 invalid=0")
    );
    let analyzed = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "analyze",
            "--input",
            path.to_str().unwrap(),
            "--type",
            "django_request",
        ])
        .output()
        .unwrap();
    assert!(analyzed.status.success());
    let stats: serde_json::Value = serde_json::from_slice(&analyzed.stdout).unwrap();
    assert_eq!(stats["django_request"]["total_logs"], 2);
    std::fs::remove_dir_all(root).unwrap();
}
