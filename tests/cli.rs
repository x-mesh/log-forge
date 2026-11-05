use std::{fs, process::Command};

#[test]
fn haproxy_logs_support_generation_validation_analysis_and_json() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("haproxy_20240102.log");
    let generate = |format: &str, path: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--output", format, "--path"])
            .arg(path)
            .args([
                "--types",
                "haproxy",
                "--total",
                "300",
                "--seed",
                "42",
                "--start-time",
                "2024-01-02 13:14:15",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let result = generate("file", &path);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let records = fs::read_to_string(&path).unwrap();
    assert_eq!(records.lines().count(), 300);
    for line in records.lines() {
        let timers: Vec<u32> = line
            .split_whitespace()
            .nth(5)
            .unwrap()
            .split('/')
            .map(|part| part.parse().unwrap())
            .collect();
        assert_eq!(timers.len(), 5);
        assert_eq!(timers[4], timers[..4].iter().sum::<u32>());
    }
    let validator = log_forge::validation::LogValidator::new("haproxy").unwrap();
    assert!(
        records
            .lines()
            .all(|line| validator.validate(line).is_valid)
    );
    let validation = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["validate", "--input"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        validation.status.success(),
        "{}",
        String::from_utf8_lossy(&validation.stderr)
    );
    let analysis = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["analyze", "--input"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        analysis.status.success(),
        "{}",
        String::from_utf8_lossy(&analysis.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&analysis.stdout).unwrap();
    assert_eq!(stats["haproxy"]["valid_logs"], 300);
    assert_eq!(stats["haproxy"]["time_distribution"]["13"], 300);
    let statuses: u64 = stats["haproxy"]["status_codes"]
        .as_object()
        .unwrap()
        .values()
        .map(|value| value.as_u64().unwrap())
        .sum();
    assert_eq!(statuses, 300);
    let json = root.path().join("records.json");
    let result = generate("json", &json);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for line in fs::read_to_string(json).unwrap().lines() {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        let raw_status: u64 = value["message"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .nth(6)
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(value["http"]["response"]["status_code"], raw_status);
    }
}

#[test]
fn kafka_broker_logs_are_reproducible_valid_and_analyzable() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("kafka_broker_20240102.log");
    let generate = || {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "console",
                "--types",
                "kafka_broker",
                "--total",
                "300",
                "--seed",
                "42",
                "--start-time",
                "2024-01-02 13:14:15",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let first = generate();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, generate().stdout);
    let records = String::from_utf8(first.stdout).unwrap();
    assert_eq!(records.lines().count(), 300);
    assert!(records.contains(" INFO "));
    assert!(records.contains(" WARN "));
    assert!(records.contains(" ERROR "));
    assert!(records.contains("Preparing rebalance"));
    assert!(records.contains("Leader epoch"));
    let validator = log_forge::validation::LogValidator::new("kafka_broker").unwrap();
    assert!(
        records
            .lines()
            .all(|record| validator.validate(record).is_valid)
    );
    fs::write(&path, records).unwrap();
    let analyzed = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["analyze", "--input"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        analyzed.status.success(),
        "{}",
        String::from_utf8_lossy(&analyzed.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&analyzed.stdout).unwrap();
    assert_eq!(stats["kafka_broker"]["valid_logs"], 300);
    assert_eq!(stats["kafka_broker"]["time_distribution"]["13"], 300);
}

#[test]
fn kafka_configuration_controls_broker_topic_partition_and_level() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let body = |settings: &str| {
        format!(
            "log_generator:\n  global:\n    output_format: console\n    total_logs: 80\n    start_time: '2024-01-01'\n  log_types:\n    kafka_broker:\n      enabled: true\n{settings}\n"
        )
    };
    fs::write(&config, body("      broker_ids: [7]\n      topics: [checkout]\n      partition_count: 3\n      log_levels: {ERROR: 1}")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config"])
        .arg(&config)
        .args(["--seed", "9", "--no-monitor"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records = String::from_utf8(output.stdout).unwrap();
    assert_eq!(records.lines().count(), 80);
    for line in records.lines() {
        assert!(line.contains(" ERROR [LogManager broker=7]"), "{line}");
        assert!(
            (0..3).any(|partition| line.contains(&format!("checkout-{partition}"))),
            "{line}"
        );
    }
    for settings in [
        "      topics: []",
        "      topics: ['bad topic']",
        "      topics: ['..']",
        "      broker_ids: []",
        "      partition_count: 0",
        "      log_levels: {FATAL: 1}",
        "      log_levels: []",
    ] {
        fs::write(&config, body(settings)).unwrap();
        let check = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["config", "validate", "--file"])
            .arg(&config)
            .output()
            .unwrap();
        assert!(!check.status.success(), "accepted: {settings}");
    }
}

#[test]
fn kafka_error_levels_survive_all_json_formats() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let outputs: Vec<_> = ["json", "elk_json", "splunk_json"]
        .iter()
        .map(|name| root.path().join(format!("{name}.json")))
        .collect();
    let handlers = ["json", "elk_json", "splunk_json"]
        .iter()
        .zip(&outputs)
        .map(|(format, path)| {
            format!(
                "      - type: file\n        file_path: '{}'\n        format: {format}\n",
                path.display()
            )
        })
        .collect::<String>();
    fs::write(&config, format!("log_generator:\n  global:\n    total_logs: 10\n    start_time: '2024-01-01'\n  log_types:\n    kafka_broker:\n      enabled: true\n      log_levels: {{ERROR: 1}}\n  output:\n    handlers:\n{handlers}")).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config"])
        .arg(&config)
        .arg("--no-monitor")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for (index, path) in outputs.iter().enumerate() {
        let contents = fs::read_to_string(path).unwrap();
        assert_eq!(contents.lines().count(), 10);
        for line in contents.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            let key = if index == 2 { "log_level" } else { "level" };
            assert_eq!(value[key], "ERROR");
        }
    }
}

#[test]
fn generated_syslog_and_redis_keep_severity_in_json() {
    let root = tempfile::tempdir().unwrap();
    for (index, style) in ["pri", "bracket", "plain", "none"].into_iter().enumerate() {
        let path = root.path().join(format!("syslog-{index}.json"));
        let config = root.path().join(format!("syslog-{index}.yaml"));
        fs::write(&config, format!("log_generator:\n  global:\n    output_format: json\n    output_path: '{}'\n    total_logs: 10\n    start_time: '2024-01-01'\n  log_types:\n    syslog:\n      enabled: true\n      facilities: [daemon]\n      severities: [crit]\n      priority_style: {style}\n", path.display())).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--config"])
            .arg(&config)
            .args(["--async-io", "--no-monitor"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{style}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        for line in fs::read_to_string(path).unwrap().lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["level"], "CRITICAL", "{style}: {line}");
        }
    }
    let path = root.path().join("redis.json");
    let config = root.path().join("redis.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: json\n    output_path: '{}'\n    total_logs: 100\n    start_time: '2024-01-01'\n  log_types:\n    redis: {{enabled: true}}\n", path.display())).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config"])
        .arg(&config)
        .args(["--threads", "4", "--no-monitor"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records = fs::read_to_string(path).unwrap();
    assert_eq!(records.lines().count(), 100);
    for line in records.lines() {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        let symbol = value["message"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .nth(5)
            .unwrap();
        let expected = match symbol {
            "*" => "NOTICE",
            "#" => "WARNING",
            "-" => "VERBOSE",
            "." => "DEBUG",
            other => panic!("unknown Redis level: {other}"),
        };
        assert_eq!(value["level"], expected);
    }
}

#[test]
fn redis_configured_levels_control_raw_and_json_output() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("redis.yaml");
    let path = root.path().join("redis.json");
    for (level, symbol) in [
        ("DEBUG", "."),
        ("VERBOSE", "-"),
        ("NOTICE", "*"),
        ("WARNING", "#"),
    ] {
        fs::write(&config, format!("log_generator:\n  global:\n    output_format: json\n    output_path: '{}'\n    total_logs: 5\n  log_types:\n    redis:\n      enabled: true\n      custom_fields:\n        log_levels: {{{level}: 1}}\n", path.display())).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let records = fs::read_to_string(&path).unwrap();
        assert_eq!(records.lines().count(), 5);
        for line in records.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["level"], level);
            assert_eq!(
                value["message"].as_str().unwrap().split_whitespace().nth(5),
                Some(symbol)
            );
        }
        fs::remove_file(&path).unwrap();
    }
    fs::write(&config, "log_generator:\n  log_types:\n    redis:\n      enabled: true\n      log_levels: {ERROR: 1}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["config", "validate", "--file", config.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid Redis log level"));
}

#[test]
fn invalid_float_weights_fail_without_panic_or_output() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let path = root.path().join("output.log");
    for weights in [
        "{DEBUG: .inf}",
        "{DEBUG: .nan}",
        "{DEBUG: 1.0e308, WARNING: 1.0e308}",
    ] {
        fs::write(&config, format!("log_generator:\n  global:\n    output_format: file\n    output_path: '{}'\n    total_logs: 1\n  log_types:\n    redis:\n      log_levels: {weights}\n", path.display())).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("panicked"), "{weights}: {stderr}");
        assert!(!path.exists());
    }
}

#[test]
fn invalid_template_distributions_fail_before_output() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let output_path = root.path().join("output.log");
    for settings in [
        "      frequency_maps:\n        status: {ok: .inf}\n",
        "      field_frequency:\n        distributions:\n          status: {ok: 1.0e308, error: 1.0e308}\n",
        "      correlation_rules:\n        - condition_field: status\n          condition_value: ok\n          target_field: result\n          target_distribution: {slow: .nan}\n",
    ] {
        fs::write(&config, format!("log_generator:\n  global:\n    output_format: file\n    output_path: '{}'\n    total_logs: 1\n  log_types:\n    custom:\n      template: '{{status}} {{result}}'\n{settings}", output_path.display())).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!error.contains("panicked"), "{settings}: {error}");
        assert!(
            error.contains("weights must be finite")
                || error.contains("weight total must be finite"),
            "{settings}: {error}"
        );
        assert!(!output_path.exists());
    }
}

#[test]
fn redis_weighted_levels_are_reproducible_across_threads() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("redis.yaml");
    fs::write(&config, "log_generator:\n  global:\n    output_format: console\n    total_logs: 100\n    start_time: '2024-01-01'\n  log_types:\n    redis:\n      enabled: true\n      log_levels: {DEBUG: 1, WARNING: 1}\n").unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    let parallel = run("4");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(serial.stdout, parallel.stdout);
    let logs = String::from_utf8(serial.stdout).unwrap();
    assert_eq!(logs.lines().count(), 100);
    assert!(
        logs.lines()
            .any(|line| line.split_whitespace().nth(5) == Some("."))
    );
    assert!(
        logs.lines()
            .any(|line| line.split_whitespace().nth(5) == Some("#"))
    );
}

#[test]
fn configured_redis_levels_include_multiple_events() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("redis.yaml");
    for level in ["DEBUG", "VERBOSE", "NOTICE", "WARNING"] {
        fs::write(&config, format!("log_generator:\n  global: {{output_format: console, total_logs: 60}}\n  log_types:\n    redis:\n      log_levels: {{{level}: 1}}\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let logs = String::from_utf8(output.stdout).unwrap();
        let events: std::collections::HashSet<_> = logs
            .lines()
            .map(|line| line.splitn(7, ' ').nth(6).unwrap())
            .collect();
        assert!(events.len() >= 3, "{level}: {events:?}");
    }
}

#[test]
fn original_command_combines_configured_additional_types() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    fs::write(&config, "log_generator:\n  global:\n    output_format: console\n    total_logs: 300\n    start_time: '2024-01-01'\n  log_types:\n    redis:\n      enabled: true\n      log_levels: {WARNING: 1}\n    ssh_auth: {enabled: true}\n    kafka_broker:\n      enabled: true\n      broker_ids: [7]\n      topics: [checkout]\n      partition_count: 1\n      log_levels: {ERROR: 1}\n").unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_log-generator"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--types",
                "redis,ssh_auth,kafka_broker",
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stdout, run().stdout);
    let logs = String::from_utf8(first.stdout).unwrap();
    let mut counts = [0; 3];
    let validators = ["redis", "ssh_auth", "kafka_broker"]
        .map(|kind| log_forge::validation::LogValidator::new(kind).unwrap());
    for line in logs.lines() {
        let matches: Vec<_> = validators
            .iter()
            .enumerate()
            .filter(|(_, validator)| validator.validate(line).is_valid)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(matches.len(), 1, "{line}");
        let index = matches[0];
        counts[index] += 1;
        if index == 0 {
            assert_eq!(line.split_whitespace().nth(5), Some("#"));
        }
        if index == 2 {
            assert!(line.contains(" ERROR ") && line.contains("checkout-0"));
        }
    }
    assert_eq!(counts.iter().sum::<usize>(), 300);
    assert!(counts.iter().all(|count| *count > 0), "{counts:?}");
}

#[test]
fn generated_django_kubernetes_and_goloop_keep_levels_in_json() {
    let root = tempfile::tempdir().unwrap();
    let cases = [
        ("django_request", "      log_levels: {ERROR: 1}", "ERROR"),
        ("django_sql", "", "DEBUG"),
        ("kubernetes_pod", "      log_levels: {WARN: 1}", "WARN"),
        (
            "goloop",
            "      log_levels: {E: 1}\n      format_type: text",
            "ERROR",
        ),
        (
            "goloop",
            "      log_levels: {F: 1}\n      format_type: json",
            "FATAL",
        ),
    ];
    for (index, (kind, settings, expected)) in cases.into_iter().enumerate() {
        let output = root.path().join(format!("{index}.json"));
        let config = root.path().join(format!("{index}.yaml"));
        fs::write(&config, format!("log_generator:\n  global:\n    output_format: json\n    output_path: '{}'\n    total_logs: 3\n    start_time: '2024-01-01'\n  log_types:\n    {kind}:\n      enabled: true\n{settings}\n", output.display())).unwrap();
        let run = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--config"])
            .arg(&config)
            .arg("--no-monitor")
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        let records = fs::read_to_string(output).unwrap();
        assert_eq!(records.lines().count(), 3);
        for line in records.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["level"], expected, "{kind}: {line}");
        }
    }
}

#[test]
fn docker_json_output_retains_generated_level() {
    let root = tempfile::tempdir().unwrap();
    for (index, (settings, expected)) in [
        ("      log_drivers: [json-file]", "ERROR"),
        ("      log_drivers: [journald]", "ERROR"),
        ("      log_drivers: [syslog]", "ERROR"),
        ("      format_type: text", "ERROR"),
    ]
    .into_iter()
    .enumerate()
    {
        let path = root.path().join(format!("{index}.json"));
        let config = root.path().join(format!("{index}.yaml"));
        fs::write(&config, format!("log_generator:\n  global:\n    output_format: json\n    output_path: '{}'\n    total_logs: 10\n    start_time: '2024-01-01'\n  log_types:\n    docker:\n      enabled: true\n      log_levels: {{error: 1}}\n{settings}\n", path.display())).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--config"])
            .arg(&config)
            .arg("--no-monitor")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{settings}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let records = fs::read_to_string(path).unwrap();
        assert_eq!(records.lines().count(), 10);
        for line in records.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["level"], expected, "{settings}: {line}");
        }
    }
}

#[test]
fn docker_level_metadata_is_stable_across_parallel_and_async_output() {
    let root = tempfile::tempdir().unwrap();
    let mut results = Vec::new();
    for (index, (threads, mode)) in [("1", "--sync-io"), ("4", "--sync-io"), ("4", "--async-io")]
        .into_iter()
        .enumerate()
    {
        let raw = root.path().join(format!("{index}.log"));
        let json = root.path().join(format!("{index}.json"));
        let config = root.path().join(format!("{index}.yaml"));
        fs::write(&config, format!("log_generator:\n  global:\n    total_logs: 40\n    start_time: '2024-01-01'\n  log_types:\n    docker:\n      enabled: true\n      log_levels: {{error: 1}}\n      log_drivers: [json-file]\n  output:\n    handlers:\n      - type: file\n        file_path: '{}'\n        format: raw\n      - type: file\n        file_path: '{}'\n        format: json\n", raw.display(), json.display())).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--config"])
            .arg(&config)
            .args(["--threads", threads, mode, "--seed", "42", "--no-monitor"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let raw = fs::read_to_string(raw).unwrap();
        let structured = fs::read_to_string(json).unwrap();
        assert_eq!(raw.lines().count(), 40);
        assert_eq!(structured.lines().count(), 40);
        for (raw_line, structured_line) in raw.lines().zip(structured.lines()) {
            let inner: serde_json::Value = serde_json::from_str(raw_line).unwrap();
            assert_eq!(inner.as_object().unwrap().len(), 3);
            let outer: serde_json::Value = serde_json::from_str(structured_line).unwrap();
            assert_eq!(outer["level"], "ERROR");
            assert_eq!(outer["message"], raw_line);
        }
        results.push((raw, structured));
    }
    assert_eq!(results[0], results[1]);
    assert_eq!(results[0], results[2]);
}

fn monitor_ready_port(message: &str, name: &str) -> u16 {
    assert!(
        message.starts_with("monitor HTTP endpoints ready "),
        "{message}"
    );
    message
        .split_whitespace()
        .find_map(|part| {
            part.strip_prefix(name)
                .and_then(|value| value.strip_prefix('='))
        })
        .expect("missing endpoint port")
        .parse()
        .unwrap()
}

#[test]
fn original_command_name_uses_the_same_cli() {
    let old = env!("CARGO_BIN_EXE_log-generator");
    let new = env!("CARGO_BIN_EXE_log-forge");
    let run = |binary: &str| {
        Command::new(binary)
            .args([
                "start",
                "--output",
                "console",
                "--types",
                "redis",
                "--total",
                "3",
                "--seed",
                "42",
                "--start-time",
                "2024-01-01",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let old_output = run(old);
    let new_output = run(new);
    assert!(
        old_output.status.success(),
        "{}",
        String::from_utf8_lossy(&old_output.stderr)
    );
    assert!(new_output.status.success());
    assert_eq!(old_output.stdout, new_output.stdout);
    let version = Command::new(old).arg("--version").output().unwrap();
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("log-generator "));
    let help = Command::new(old).arg("--help").output().unwrap();
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage: log-generator"));
}

#[test]
fn original_command_merges_file_defaults_and_supports_standalone_mode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 20\n  log_types:\n    redis:\n      enabled: true\n      frequency: 1\n").unwrap();
    for (binary, standalone, inherited) in [
        (env!("CARGO_BIN_EXE_log-generator"), false, true),
        (env!("CARGO_BIN_EXE_log-generator"), true, false),
        (env!("CARGO_BIN_EXE_log-forge"), false, false),
    ] {
        let mut command = Command::new(binary);
        command.args([
            "config",
            "show",
            "--file",
            path.to_str().unwrap(),
            "--format",
            "json",
        ]);
        if standalone {
            command.arg("--standalone-config");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let config: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            config["log_generator"]["log_types"].get("syslog").is_some(),
            inherited
        );
        assert_eq!(
            config["log_generator"]["log_types"]["redis"]["enabled"],
            true
        );
    }
    let output = Command::new(env!("CARGO_BIN_EXE_log-generator"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--types",
            "syslog",
            "--total",
            "1",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap().lines().count(), 1);
}

#[test]
fn standalone_config_requires_an_input_file() {
    for args in [
        vec!["start", "--standalone-config"],
        vec!["config", "show", "--standalone-config"],
        vec!["config", "create", "--standalone-config"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--standalone-config requires"));
    }
}

#[test]
fn repeated_type_options_ignore_whitespace_and_duplicates() {
    let run = |types: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_log-forge"));
        command.args([
            "start",
            "--output",
            "console",
            "--total",
            "30",
            "--seed",
            "42",
            "--start-time",
            "2024-01-01",
            "--no-monitor",
        ]);
        for value in types {
            command.args(["--types", value]);
        }
        command.output().unwrap()
    };
    let expected = run(&["redis,ssh_auth"]);
    let repeated = run(&[" redis, ssh_auth ", "redis", " , "]);
    assert!(expected.status.success());
    assert!(
        repeated.status.success(),
        "{}",
        String::from_utf8_lossy(&repeated.stderr)
    );
    assert_eq!(expected.stdout, repeated.stdout);
    assert!(!run(&[" , "]).status.success());
}

#[test]
fn original_defaults_merge_is_available_for_cli_configuration_and_start() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.yaml");
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 4\n    start_time: '2024-01-01'\n  log_types:\n    nginx_access:\n      custom_fields:\n        status_codes: {503: 1.0}\n").unwrap();
    let show = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["config", "show", "--file"])
        .arg(&path)
        .args(["--format", "json", "--original-defaults"])
        .output()
        .unwrap();
    assert!(
        show.status.success(),
        "{}",
        String::from_utf8_lossy(&show.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(
        document["log_generator"]["log_types"]["nginx_access"]["custom_fields"]["status_codes"]["200"],
        0.7
    );
    assert_eq!(
        document["log_generator"]["log_types"]["nginx_access"]["custom_fields"]["status_codes"]["503"],
        1.0
    );
    assert_eq!(
        document["log_generator"]["log_types"]["syslog"]["enabled"],
        true
    );
    let run = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config"])
        .arg(&path)
        .args(["--original-defaults", "--types", "syslog", "--no-monitor"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let validator = log_forge::validation::LogValidator::new("syslog").unwrap();
    let records = String::from_utf8(run.stdout).unwrap();
    assert_eq!(records.lines().count(), 4);
    assert!(
        records
            .lines()
            .all(|line| validator.validate(line).is_valid)
    );
}

#[test]
fn cli_without_config_uses_original_defaults() {
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["config", "show", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let defaults: serde_json::Value =
        serde_json::from_str(include_str!("../data/original_default_config.json")).unwrap();
    for name in ["nginx_access", "nginx_error", "syslog", "fastapi", "goloop"] {
        assert_eq!(
            document["log_generator"]["log_types"][name]["frequency"],
            defaults["log_generator"]["log_types"][name]["frequency"]
        );
    }
    let generated = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "syslog",
            "--total",
            "100",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let logs = String::from_utf8(generated.stdout).unwrap();
    assert_eq!(logs.lines().count(), 100);
    assert!(logs.lines().all(|line| {
        let priority: u16 = line
            .strip_prefix('<')
            .unwrap()
            .split_once('>')
            .unwrap()
            .0
            .parse()
            .unwrap();
        [0, 2, 3, 4].contains(&(priority / 8)) && [3, 4, 5, 6].contains(&(priority % 8))
    }));
}

#[test]
fn original_command_uses_virtual_time_unless_real_time_is_requested() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    fs::write(&config, "log_generator:\n  global:\n    virtual_time: false\n    output_format: console\n    total_logs: 1\n    start_time: '2020-01-02'\n  log_types:\n    redis: {enabled: true}\n").unwrap();
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_log-generator"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--types",
                "redis",
                "--no-monitor",
            ])
            .args(extra)
            .output()
            .unwrap()
    };
    let virtual_time = run(&[]);
    assert!(
        virtual_time.status.success(),
        "{}",
        String::from_utf8_lossy(&virtual_time.stderr)
    );
    assert!(String::from_utf8_lossy(&virtual_time.stdout).contains("2020"));
    let real_time = run(&["--real-time"]);
    assert!(
        real_time.status.success(),
        "{}",
        String::from_utf8_lossy(&real_time.stderr)
    );
    assert!(!String::from_utf8_lossy(&real_time.stdout).contains("2020"));
}

#[test]
fn original_defaults_validate_all_bundled_configurations() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config");
    let mut paths: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    paths.sort();
    assert_eq!(paths.len(), 9);
    for path in paths {
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["config", "validate", "--file"])
            .arg(&path)
            .arg("--original-defaults")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/goloop_config.yaml");
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config"])
        .arg(path)
        .args([
            "--original-defaults",
            "--types",
            "goloop",
            "--output",
            "console",
            "--virtual-time",
            "--total",
            "100",
            "--start-time",
            "2024-01-01",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records = String::from_utf8(output.stdout).unwrap();
    assert_eq!(records.lines().count(), 100);
    for line in records.lines() {
        let module = line.split('|').nth(4).unwrap();
        assert!(
            ["CS", "BM", "SV", "NM", "fastsync", "TP", "EE"].contains(&module),
            "{line}"
        );
    }
}

#[test]
fn original_default_merge_runs_all_bundled_configurations() {
    let root = tempfile::tempdir().unwrap();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config");
    let mut sources: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    sources.sort();
    assert_eq!(sources.len(), 9);
    for (index, source) in sources.into_iter().enumerate() {
        let mut config: serde_yaml::Value =
            serde_yaml::from_slice(&fs::read(&source).unwrap()).unwrap();
        let monitoring = &mut config["log_generator"]["monitoring"];
        if monitoring.is_mapping() {
            for service in ["metrics", "health_check"] {
                if monitoring[service].is_mapping()
                    && monitoring[service]["enabled"].as_bool() == Some(true)
                {
                    monitoring[service]["port"] = serde_yaml::to_value(0).unwrap();
                }
            }
        }
        let path = root.path().join(format!("{index}.yaml"));
        fs::write(&path, serde_yaml::to_string(&config).unwrap()).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--config"])
            .arg(&path)
            .args([
                "--original-defaults",
                "--output",
                "console",
                "--virtual-time",
                "--total",
                "5",
                "--seed",
                "42",
                "--start-time",
                "2024-01-01",
                "--no-monitor",
            ])
            .current_dir(root.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}: {}",
            source.display(),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            String::from_utf8_lossy(&result.stdout).lines().count() >= 5,
            "{}",
            source.display()
        );
    }
}

#[test]
fn bundled_configurations_generate_valid_records_for_each_default_type() {
    let root = tempfile::tempdir().unwrap();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config");
    let mut sources: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yaml"))
        .collect();
    sources.sort();
    for (index, source) in sources.iter().enumerate() {
        let mut config: serde_yaml::Value =
            serde_yaml::from_slice(&fs::read(source).unwrap()).unwrap();
        if config["log_generator"]["distributed_tracing"].is_mapping() {
            config["log_generator"]["distributed_tracing"]["enabled"] =
                serde_yaml::Value::Bool(false);
        }
        let monitoring = &mut config["log_generator"]["monitoring"];
        if monitoring.is_mapping() {
            for service in ["metrics", "health_check"] {
                if monitoring[service].is_mapping()
                    && monitoring[service]["enabled"].as_bool() == Some(true)
                {
                    monitoring[service]["port"] = serde_yaml::to_value(0).unwrap();
                }
            }
        }
        let path = root.path().join(format!("{index}.yaml"));
        fs::write(&path, serde_yaml::to_string(&config).unwrap()).unwrap();
        for kind in ["nginx_access", "nginx_error", "syslog", "fastapi", "goloop"] {
            let output = Command::new(env!("CARGO_BIN_EXE_log-generator"))
                .args(["start", "--config"])
                .arg(&path)
                .args([
                    "--types",
                    kind,
                    "--output",
                    "console",
                    "--virtual-time",
                    "--total",
                    "10",
                    "--seed",
                    "42",
                    "--start-time",
                    "2024-01-01",
                    "--no-monitor",
                ])
                .current_dir(root.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{} {kind}: {}",
                source.display(),
                String::from_utf8_lossy(&output.stderr)
            );
            let records = String::from_utf8(output.stdout).unwrap();
            assert_eq!(records.lines().count(), 10, "{} {kind}", source.display());
            let validator = log_forge::validation::LogValidator::new(kind).unwrap();
            for record in records.lines() {
                assert!(
                    validator.validate(record).is_valid,
                    "{} {kind}: {record}",
                    source.display()
                );
            }
        }
    }
}

#[test]
fn original_advanced_config_sends_all_handlers_to_local_receivers() {
    use std::io::Read;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => panic!("TCP receiver failed: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        String::from_utf8(bytes).unwrap()
    });
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/advanced_config.yaml");
    let mut config: serde_yaml::Value = serde_yaml::from_slice(&fs::read(source).unwrap()).unwrap();
    let raw = root.path().join("raw.log");
    let json = root.path().join("structured.json");
    let handlers = config["log_generator"]["output"]["handlers"]
        .as_sequence_mut()
        .unwrap();
    handlers[0]["file_path"] = serde_yaml::to_value(&raw).unwrap();
    handlers[1]["file_path"] = serde_yaml::to_value(&json).unwrap();
    handlers[2]["host"] = serde_yaml::to_value("127.0.0.1").unwrap();
    handlers[2]["port"] = serde_yaml::to_value(port).unwrap();
    config["log_generator"]["monitoring"]["metrics"]["port"] = serde_yaml::to_value(0).unwrap();
    config["log_generator"]["monitoring"]["health_check"]["port"] =
        serde_yaml::to_value(0).unwrap();
    let path = root.path().join("config.yaml");
    fs::write(&path, serde_yaml::to_string(&config).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config"])
        .arg(&path)
        .args([
            "--original-defaults",
            "--types",
            "nginx_access",
            "--virtual-time",
            "--total",
            "20",
            "--start-time",
            "2024-01-01",
            "--seed",
            "42",
            "--no-monitor",
        ])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let network = server.join().unwrap();
    let plain = fs::read_to_string(raw).unwrap();
    let structured = fs::read_to_string(json).unwrap();
    let console = String::from_utf8(output.stdout).unwrap();
    let plain: Vec<_> = plain.lines().collect();
    let structured: Vec<serde_json::Value> = structured
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let network: Vec<_> = network.lines().collect();
    let console: Vec<_> = console.lines().collect();
    assert_eq!(
        (plain.len(), structured.len(), network.len(), console.len()),
        (20, 20, 20, 20)
    );
    for index in 0..20 {
        assert_eq!(structured[index]["message"], plain[index]);
        assert!(network[index].starts_with("<14>1 "));
        assert!(network[index].ends_with(plain[index]));
        assert!(console[index].ends_with(plain[index]));
    }
}

fn receive_remote_write(listener: &std::net::TcpListener) -> (String, Vec<u8>) {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::time::{Duration, Instant};
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => panic!("receiver failed: {error}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        assert_ne!(reader.read_line(&mut line).unwrap(), 0);
        if line == "\r\n" {
            break;
        }
        headers.push_str(&line);
    }
    let length: usize = headers
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .unwrap()
        .parse()
        .unwrap();
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    stream
        .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
        .unwrap();
    (headers, body)
}

#[test]
fn help_lists_commands_and_reports_argument_errors_cleanly() {
    for args in [&["--help"][..], &["start", "--help"], &["help", "sample"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains("Usage:"), "{help}");
        if args == ["--help"] {
            for name in [
                "start",
                "config",
                "sample",
                "analyze",
                "validate",
                "list-types",
            ] {
                assert!(help.contains(name), "{help}");
            }
        }
    }
    let error = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--unknown"])
        .output()
        .unwrap();
    assert!(!error.status.success());
    let message = String::from_utf8(error.stderr).unwrap();
    assert!(message.contains("unexpected argument"));
    assert!(!message.contains("ErrorInner"));
}

#[test]
fn generates_reproducible_combined_logs() {
    let path = std::env::temp_dir().join(format!("log-forge-{}.log", std::process::id()));
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "--total",
                "3",
                "--output",
                path.to_str().unwrap(),
                "--seed",
                "42",
                "--start",
                "2024-01-01T00:00:00Z",
            ])
            .status()
            .unwrap()
    };
    assert!(run().success());
    let first = fs::read_to_string(&path).unwrap();
    assert_eq!(first.lines().count(), 3);
    for line in first.lines() {
        assert!(line.contains("[01/Jan/2024:00:00:00 +0000]"));
        assert_eq!(line.matches('"').count(), 6);
    }
    assert!(run().success());
    assert_eq!(first, fs::read_to_string(&path).unwrap());
    fs::remove_file(path).unwrap();
}

#[test]
fn generates_each_supported_log_type() {
    let path = std::env::temp_dir().join(format!("log-forge-types-{}.log", std::process::id()));
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
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "--total",
                "1",
                "--output",
                path.to_str().unwrap(),
                "--log-type",
                kind,
                "--start",
                "2024-01-01T00:00:00Z",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            !fs::read_to_string(&path).unwrap().trim().is_empty(),
            "{kind}"
        );
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn start_reads_config_and_advances_virtual_time() {
    let root = std::env::temp_dir().join(format!("log-forge-config-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: multi_file\n    output_path: '{}'\n    total_logs: 3\n    generation_interval: 1\n    start_time: '2024-01-01 00:00:00'\n  log_types:\n    nginx_access:\n      enabled: true\n      frequency: 1\n", root.display())).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let logs = fs::read_to_string(root.join("nginx_access_20240101.log")).unwrap();
    assert_eq!(logs.lines().count(), 3);
    assert!(logs.contains("01/Jan/2024:00:00:00 +0000"));
    assert!(logs.contains("01/Jan/2024:00:00:02 +0000"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn multi_file_can_combine_types_and_still_split_by_date() {
    let temp = tempfile::tempdir().unwrap();
    let logs = temp.path().join("logs");
    let config = temp.path().join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: multi_file\n    output_path: '{}'\n    total_logs: 80\n    generation_interval: 3600\n    start_time: '2024-01-01 00:00:00'\n    multi_file:\n      separate_by_type: false\n  log_types:\n    redis:\n      enabled: true\n      frequency: 1\n    ssh_auth:\n      enabled: true\n      frequency: 1\n", logs.display())).unwrap();
    for args in [
        vec!["config", "validate", "--file"],
        vec!["start", "--config"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(args)
            .arg(&config)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let first = fs::read_to_string(logs.join("all_20240101.log")).unwrap();
    assert_eq!(first.lines().count(), 24);
    assert!(first.contains(":M "));
    assert!(first.contains("sshd"));
    assert_eq!(
        fs::read_to_string(logs.join("all_20240102.log"))
            .unwrap()
            .lines()
            .count(),
        24
    );
    assert_eq!(fs::read_dir(&logs).unwrap().count(), 4);
}

#[test]
fn multi_file_datetime_uses_configured_date_format() {
    let temp = tempfile::tempdir().unwrap();
    let logs = temp.path().join("logs");
    let config = temp.path().join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: multi_file\n    output_path: '{}'\n    total_logs: 1\n    start_time: '2024-01-02 03:04:05'\n    multi_file:\n      file_pattern: '{{log_type}}_{{date}}_{{time}}_{{datetime}}.log'\n      date_format: '%Y-%m-%d'\n  log_types:\n    redis:\n      enabled: true\n", logs.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config"])
        .arg(&config)
        .arg("--no-monitor")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        logs.join("redis_2024-01-02_030405_2024-01-02_030405.log")
            .exists()
    );
}

#[test]
fn start_sends_udp_and_rejects_unknown_config() {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "network",
            "--protocol",
            "udp",
            "--host",
            "127.0.0.1",
            "--port",
            &socket.local_addr().unwrap().port().to_string(),
            "--total",
            "1",
            "--types",
            "redis",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut buffer = [0; 1024];
    let count = socket.recv(&mut buffer).unwrap();
    assert!(
        std::str::from_utf8(&buffer[..count])
            .unwrap()
            .contains(":M ")
    );

    let path = std::env::temp_dir().join(format!("log-forge-invalid-{}.yaml", std::process::id()));
    fs::write(
        &path,
        "log_generator:\n  global:\n    unsupported_setting: 100\n",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("unsupported_setting"));
    fs::remove_file(path).unwrap();
}

#[test]
fn stalled_tcp_destination_times_out_without_hanging() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "network",
            "--protocol",
            "tcp",
            "--host",
            "127.0.0.1",
            "--port",
            &port,
            "--network-timeout",
            "0.1",
            "--types",
            "nginx_access",
            "--total",
            "1000000",
            "--async-io",
            "--no-monitor",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let connection = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("server accept failed: {error}");
            }
        }
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("stalled writer did not time out");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    drop(connection);
    assert!(!status.success());
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
    assert!(!stderr.trim().is_empty());
}

#[test]
fn invalid_network_timeout_is_rejected() {
    for value in ["0", "-1", "NaN"] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "network",
                "--types",
                "redis",
                "--total",
                "1",
                "--network-timeout",
                value,
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(!result.status.success());
    }
}

#[test]
fn nginx_custom_fields_are_applied() {
    let root = std::env::temp_dir().join(format!("log-forge-custom-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.yaml");
    let path = root.join("out.log");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: file\n    output_path: '{}'\n    total_logs: 8\n  log_types:\n    nginx_access:\n      patterns: [combined]\n      custom_fields:\n        ip_ranges: ['10.2.3.0/24']\n        status_codes: {{503: 1.0}}\n        user_agents: ['custom-agent']\n        url_patterns: ['/custom']\n", path.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", config.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let logs = fs::read_to_string(path).unwrap();
    assert_eq!(logs.lines().count(), 8);
    for line in logs.lines() {
        assert!(line.starts_with("10.2.3."), "{line}");
        assert!(line.contains(" /custom HTTP/1.1\" 503 "), "{line}");
        assert!(line.ends_with("\"custom-agent\""), "{line}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_server_custom_patterns_and_apache_format_are_applied() {
    let path =
        std::env::temp_dir().join(format!("log-forge-web-custom-{}.yaml", std::process::id()));
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 4\n  log_types:\n    nginx_error:\n      enabled: true\n      error_levels: [warn]\n      error_patterns: ['specific upstream failure']\n    apache_access:\n      enabled: true\n      log_format: common\n      custom_fields:\n        status_codes: {503: 1}\n        url_patterns: ['/apache-only']\n        ip_ranges: ['10.9.0.0/16']\n").unwrap();
    for (kind, expected) in [
        ("nginx_error", "specific upstream failure"),
        ("apache_access", " /apache-only HTTP/1.1\" 503 "),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--types",
                kind,
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        assert_eq!(output.lines().count(), 4);
        for line in output.lines() {
            assert!(line.contains(expected), "{line}");
            if kind == "apache_access" {
                assert_eq!(line.matches('"').count(), 2);
                assert!(line.starts_with("10.9."));
            } else {
                assert!(line.contains("[warn]"));
            }
        }
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn web_error_custom_fields_accept_original_nested_form() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 4\n  log_types:\n    nginx_error:\n      enabled: true\n      custom_fields:\n        error_levels: [warn]\n        error_patterns: ['nested upstream failure']\n    apache_error:\n      enabled: true\n      custom_fields:\n        error_levels: [warn]\n        error_patterns: ['nested upstream failure']\n").unwrap();
    for kind in ["nginx_error", "apache_error"] {
        for defaults in [false, true] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_log-forge"));
            cmd.args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--types",
                kind,
                "--no-monitor",
            ]);
            if defaults {
                cmd.arg("--original-defaults");
            }
            let output = cmd.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let logs = String::from_utf8(output.stdout).unwrap();
            assert_eq!(logs.lines().count(), 4);
            for line in logs.lines() {
                assert!(line.contains("nested upstream failure"), "{line}");
                assert!(line.contains("warn"), "{line}");
            }
        }
    }
    fs::write(&path, "log_generator:\n  log_types:\n    nginx_error:\n      enabled: true\n      error_levels: [error]\n      custom_fields:\n        error_levels: [warn]\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["config", "validate", "--file", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must each appear in one location"));
}

#[test]
fn web_error_levels_accept_weighted_maps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    for kind in ["nginx_error", "apache_error"] {
        for nested in [false, true] {
            let settings = if nested {
                "      custom_fields:\n        error_levels: {warn: 1, error: 0}\n"
            } else {
                "      error_levels: {warn: 1, error: 0}\n"
            };
            fs::write(&path, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 30\n  log_types:\n    {kind}:\n      enabled: true\n{settings}")).unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_log-generator"))
                .args([
                    "start",
                    "--config",
                    path.to_str().unwrap(),
                    "--types",
                    kind,
                    "--no-monitor",
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{kind}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let logs = String::from_utf8(output.stdout).unwrap();
            assert_eq!(logs.lines().count(), 30);
            assert!(logs.lines().all(|line| line.contains("[warn]")));
        }
    }
    fs::write(&path, "log_generator:\n  log_types:\n    nginx_error:\n      enabled: true\n      error_levels: {error: -1}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-generator"))
        .args(["config", "validate", "--file", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn explicit_empty_weighted_settings_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    for (kind, settings) in [
        ("fastapi", "log_levels: []"),
        ("mysql_query", "query_types: {}"),
        ("kubernetes_event", "event_types: []"),
        ("goloop", "modules: {}"),
    ] {
        fs::write(
            &path,
            format!(
                "log_generator:\n  log_types:\n    {kind}:\n      enabled: true\n      {settings}\n"
            ),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["config", "validate", "--file", path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(!output.status.success(), "accepted {kind}: {settings}");
    }
}

#[test]
fn error_generators_use_multiple_original_message_patterns() {
    let placeholder = regex::Regex::new(r"\{[A-Za-z_]+\}").unwrap();
    for kind in ["nginx_error", "apache_error", "mysql_error"] {
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "console",
                "--types",
                kind,
                "--total",
                "100",
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = String::from_utf8(output.stdout).unwrap();
        let messages: std::collections::HashSet<_> = output
            .lines()
            .map(|line| match kind {
                "nginx_error" => {
                    line.split_once(": *")
                        .unwrap()
                        .1
                        .split_once(' ')
                        .unwrap()
                        .1
                        .split_once(", client:")
                        .unwrap()
                        .0
                }
                "apache_error" => line.split_once("] File ").map_or_else(
                    || {
                        line.split_once("] [client ")
                            .unwrap()
                            .1
                            .split_once("] ")
                            .unwrap()
                            .1
                    },
                    |(_, message)| message,
                ),
                _ => line.split_once("] ").unwrap().1,
            })
            .collect();
        assert!(
            messages.len() >= 5,
            "{kind}: {} distinct messages",
            messages.len()
        );
        assert!(
            messages
                .iter()
                .all(|message| !placeholder.is_match(message)),
            "{kind}: {messages:?}"
        );
    }
}

#[test]
fn size_rotation_preserves_existing_rotated_files() {
    let root = std::env::temp_dir().join(format!("log-forge-rotate-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("out.log");
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "file",
            "--path",
            path.to_str().unwrap(),
            "--types",
            "redis",
            "--total",
            "4",
            "--rotation-size",
            "1",
            "--max-files",
            "2",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("rotation limit reached"));
    for file in [&path, &root.join("out.log.1"), &root.join("out.log.2")] {
        assert!(fs::read_to_string(file).unwrap().contains(":M "));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn nested_rotation_configuration_rotates_and_preserves_files() {
    let root =
        std::env::temp_dir().join(format!("log-forge-nested-rotation-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let output = root.join("out.log");
    let config = root.join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: file\n    output_path: '{}'\n    total_logs: 3\n    log_rotation:\n      enabled: true\n      max_size: '1B'\n      max_files: 3\n      compress: false\n  log_types:\n    redis:\n      enabled: true\n", output.display())).unwrap();
    let validate = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["config", "validate", "--file", config.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        validate.status.success(),
        "{}",
        String::from_utf8_lossy(&validate.stderr)
    );
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", config.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for name in ["out.log", "out.log.1", "out.log.2"] {
        assert!(fs::read_to_string(root.join(name)).unwrap().contains(":M "));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn compressed_rotation_preserves_records_and_existing_files() {
    use std::io::Read;
    let root = std::env::temp_dir().join(format!("log-forge-gzip-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let output = root.join("out.log");
    let previous = root.join("out.log.1");
    fs::write(&previous, "existing backup\n").unwrap();
    let config = root.join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: file\n    output_path: '{}'\n    total_logs: 2\n    log_rotation:\n      enabled: true\n      max_size: 1\n      max_files: 3\n      compress: true\n  log_types:\n    redis:\n      enabled: true\n", output.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--async-io",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fs::read_to_string(&previous).unwrap(), "existing backup\n");
    let compressed_path = root.join("out.log.2.gz");
    let backup_before = fs::read(&compressed_path).unwrap();
    let mut decompressed = String::new();
    flate2::read::GzDecoder::new(fs::File::open(&compressed_path).unwrap())
        .read_to_string(&mut decompressed)
        .unwrap();
    assert!(decompressed.contains(":M "));
    assert!(fs::read_to_string(&output).unwrap().contains(":M "));
    let second = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--async-io",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("rotation limit reached"));
    assert_eq!(fs::read(&compressed_path).unwrap(), backup_before);
    assert_eq!(fs::read_to_string(previous).unwrap(), "existing backup\n");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn rotation_does_not_unlink_a_special_device_symlink() {
    let root =
        std::env::temp_dir().join(format!("log-forge-rotation-device-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("device.log");
    std::os::unix::fs::symlink("/dev/null", &path).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "file",
            "--path",
            path.to_str().unwrap(),
            "--types",
            "redis",
            "--total",
            "2",
            "--rotation-size",
            "1",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn original_default_config_runs() {
    let config =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/default_config.yaml");
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--output",
            "console",
            "--total",
            "20",
            "--seed",
            "1",
            "--start-time",
            "2024-01-01",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8(result.stdout).unwrap().lines().count(),
        20
    );
}

#[test]
fn original_performance_configs_run() {
    for name in [
        "high_performance_config.yaml",
        "ultra_performance_config.yaml",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("config")
            .join(name);
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--output",
                "console",
                "--total",
                "25",
                "--seed",
                "2",
                "--start-time",
                "2024-01-01",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8(result.stdout).unwrap().lines().count(),
            25
        );
    }
}

#[test]
fn parallel_generation_preserves_seeded_output_order() {
    let run = |threads: &str, randomized: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_log-forge"));
        command.args([
            "start",
            "--output",
            "console",
            "--types",
            "nginx_access,syslog,fastapi,django_request",
            "--total",
            "400",
            "--seed",
            "71",
            "--start-time",
            "2024-01-01",
            "--threads",
            threads,
            "--no-monitor",
        ]);
        if randomized {
            command.arg("--random-interval");
        }
        command.output().unwrap()
    };
    for randomized in [false, true] {
        let sequential = run("1", randomized);
        let parallel = run("4", randomized);
        assert!(
            sequential.status.success(),
            "{}",
            String::from_utf8_lossy(&sequential.stderr)
        );
        assert!(
            parallel.status.success(),
            "{}",
            String::from_utf8_lossy(&parallel.stderr)
        );
        assert_eq!(sequential.stdout, parallel.stdout);
    }
    let invalid = run("0", false);
    assert!(!invalid.status.success());
}

#[test]
fn parallel_generation_keeps_custom_counter_sequence() {
    let config = std::env::temp_dir().join(format!(
        "log-forge-parallel-custom-{}.yaml",
        std::process::id()
    ));
    fs::write(&config, "log_generator:\n  global:\n    output_format: console\n    total_logs: 600\n    start_time: '2024-01-01'\n  log_types:\n    custom:\n      enabled: true\n      frequency: 1\n      template: '{counter} {timestamp}'\n    redis:\n      enabled: true\n      frequency: 1\n").unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--threads",
                threads,
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let sequential = run("1");
    let parallel = run("3");
    assert!(sequential.status.success());
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(sequential.stdout, parallel.stdout);
    fs::remove_file(config).unwrap();
}

#[test]
fn real_time_thread_request_reports_sequential_execution() {
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "redis",
            "--total",
            "1",
            "--real-time",
            "--interval",
            "0",
            "--threads",
            "2",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("threads are not applied")
    );
}

#[test]
fn async_output_preserves_order_and_flushes_on_exit() {
    let root = std::env::temp_dir().join(format!("log-forge-async-output-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let run = |path: &std::path::Path, mode: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "file",
                "--path",
                path.to_str().unwrap(),
                "--types",
                "nginx_access,fastapi,django_request",
                "--total",
                "1000",
                "--seed",
                "42",
                "--start-time",
                "2024-01-01",
                "--threads",
                "4",
                "--no-monitor",
                mode,
            ])
            .output()
            .unwrap()
    };
    let sync_path = root.join("sync.log");
    let async_path = root.join("async.log");
    let sync = run(&sync_path, "--sync-io");
    let asynchronous = run(&async_path, "--async-io");
    assert!(
        sync.status.success(),
        "{}",
        String::from_utf8_lossy(&sync.stderr)
    );
    assert!(
        asynchronous.status.success(),
        "{}",
        String::from_utf8_lossy(&asynchronous.stderr)
    );
    assert_eq!(fs::read(sync_path).unwrap(), fs::read(async_path).unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn async_output_reports_broken_pipe() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "nginx_access",
            "--total",
            "100000",
            "--async-io",
            "--no-monitor",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let result = child.wait_with_output().unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .to_lowercase()
            .contains("broken pipe")
    );
}

#[cfg(unix)]
#[test]
fn async_output_flushes_when_stopped() {
    let root = std::env::temp_dir().join(format!("log-forge-async-stop-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("output.log");
    let socket = root.join("control.sock");
    let mut child = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "file",
            "--path",
            path.to_str().unwrap(),
            "--types",
            "redis",
            "--total",
            "0",
            "--real-time",
            "--interval",
            "60",
            "--no-monitor",
            "--async-io",
            "--control-socket",
            socket.to_str().unwrap(),
        ])
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while !socket.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if !socket.exists() {
        child.kill().unwrap();
        child.wait().unwrap();
        panic!("control socket was not created");
    }
    std::thread::sleep(std::time::Duration::from_millis(100));
    let stop = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["stop", "--control-socket", socket.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    assert!(child.wait().unwrap().success());
    assert!(fs::read_to_string(path).unwrap().contains(":M "));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn parallel_unlimited_generation_can_be_stopped() {
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let root = std::env::temp_dir().join(format!("log-forge-parallel-stop-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let socket = root.join("control.sock");
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "file",
                "--path",
                "/dev/null",
                "--types",
                "nginx_access",
                "--total",
                "0",
                "--threads",
                "4",
                "--no-monitor",
                "--control-socket",
                socket.to_str().unwrap(),
            ])
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !socket.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(socket.exists());
    let stop = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["stop", "--control-socket", socket.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    assert!(child.0.wait().unwrap().success());
    assert!(!socket.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn original_basic_and_goloop_configs_run_with_explicit_compatibility_notice() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["basic_config.yaml", "goloop_config.yaml"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("config")
            .join(name);
        let path = if name == "basic_config.yaml" {
            let mut config: serde_yaml::Value =
                serde_yaml::from_slice(&fs::read(&path).unwrap()).unwrap();
            config["log_generator"]["logging"]["file"] = serde_yaml::Value::String(
                temp.path()
                    .join("generator.log")
                    .to_string_lossy()
                    .into_owned(),
            );
            let copy = temp.path().join(name);
            fs::write(&copy, serde_yaml::to_string(&config).unwrap()).unwrap();
            copy
        } else {
            path
        };
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--output",
                "console",
                "--total",
                "4",
                "--virtual-time",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8(result.stdout).unwrap().lines().count(), 4);
        let warning = String::from_utf8(result.stderr).unwrap();
        assert!(!warning.contains("threads"), "{name}: {warning}");
        if name == "basic_config.yaml" {
            assert!(
                fs::read_to_string(temp.path().join("generator.log"))
                    .unwrap()
                    .contains("generation finished: 4 records")
            );
        }
        if name == "goloop_config.yaml" {
            assert!(!warning.contains("async_io"));
        }
    }
}

#[cfg(unix)]
#[test]
fn multifile_output_handles_more_paths_than_open_file_limit() {
    let root = std::env::temp_dir().join(format!("log-forge-file-limit-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let output = root.join("logs");
    let config = root.join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: multi_file\n    output_path: '{}'\n    total_logs: 120\n    generation_interval: 1\n    start_time: '2024-01-01'\n    multi_file:\n      file_pattern: '{{log_type}}_{{date}}.log'\n      date_format: '%Y%m%d_%H%M%S'\n  log_types:\n    redis:\n      enabled: true\n", output.display())).unwrap();
    let result = Command::new("sh")
        .args([
            "-c",
            "ulimit -n 64; exec \"$@\"",
            "sh",
            env!("CARGO_BIN_EXE_log-forge"),
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let files = fs::read_dir(&output)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(files.len(), 120);
    for file in files {
        assert!(fs::read_to_string(file.path()).unwrap().contains(":M "));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn monitor_reports_generation_counts_without_polluting_logs() {
    let run = |extra: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_log-forge"));
        command.args([
            "start", "--output", "console", "--types", "redis", "--total", "3",
        ]);
        command.args(extra).output().unwrap()
    };
    let monitored = run(&[]);
    assert!(monitored.status.success());
    assert_eq!(
        String::from_utf8(monitored.stdout).unwrap().lines().count(),
        3
    );
    let status = String::from_utf8(monitored.stderr).unwrap();
    assert!(status.contains("generated=3"), "{status}");
    assert!(status.contains("redis=3"), "{status}");
    assert!(status.contains("progress=3/3"), "{status}");
    let quiet = run(&["--no-monitor"]);
    assert!(quiet.status.success());
    assert!(quiet.stderr.is_empty());
    let no_progress = run(&["--no-progress"]);
    let status = String::from_utf8(no_progress.stderr).unwrap();
    assert!(status.contains("generated=3"));
    assert!(!status.contains("progress="));
}

#[test]
fn monitor_emits_live_counts_while_waiting_between_records() {
    use std::io::{BufRead, BufReader};
    let mut child = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "redis",
            "--total",
            "2",
            "--real-time",
            "--interval",
            "3",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut line = String::new();
        BufReader::new(stderr).read_line(&mut line).unwrap();
        send.send(line).unwrap();
    });
    let result = receive.recv_timeout(std::time::Duration::from_secs(5));
    let running = child.try_wait().unwrap().is_none();
    if running {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    reader.join().unwrap();
    let line = result.unwrap();
    assert!(running, "monitor emitted only after generation stopped");
    assert!(line.contains("generated=1"), "{line}");
}

#[test]
fn type_settings_and_custom_template_control_output() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-types-config-{}.yaml",
        std::process::id()
    ));
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 1\n    start_time: '2024-01-01'\n  log_types:\n    nginx_error:\n      enabled: true\n      error_levels: [crit]\n    syslog:\n      enabled: true\n      facilities: [local3]\n      severities: [err]\n    fastapi:\n      enabled: true\n      log_levels: [WARNING]\n      endpoints: ['/only']\n    custom:\n      enabled: true\n      template: '{timestamp} [{level}] {message}'\n      fields:\n        timestamp: datetime\n        level: [NOTICE]\n        message: sentence\n").unwrap();
    for (kind, expected) in [
        ("nginx_error", "[crit]"),
        ("syslog", "<155>"),
        ("fastapi", "[WARNING]"),
        ("custom", "2024-01-01 00:00:00 [NOTICE] Request completed"),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_log-forge"));
        command.args(["start", "--config", path.to_str().unwrap(), "--types", kind]);
        if kind == "fastapi" {
            command.args(["--total", "100"]);
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        assert!(output.contains(expected), "{kind}: {output}");
        if kind == "fastapi" {
            assert!(output.contains("/only"), "{output}");
        }
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn goloop_weighted_config_and_json_output() {
    let path = std::env::temp_dir().join(format!("log-forge-goloop-{}.yaml", std::process::id()));
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 8\n    generation_interval: 0\n    start_time: '2024-01-01'\n  log_types:\n    goloop:\n      enabled: true\n      format_type: json\n      height_range: [42, 42]\n      log_levels: {E: 1.0}\n      module_weights: {BM: 1.0}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 8);
    for line in stdout.lines() {
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(record["level"], "E");
        assert_eq!(record["module"], "BM");
        assert!(record["msg"].as_str().unwrap().contains("height=42"));
        assert_eq!(record["timestamp"], "20240101-00:00:00.000000");
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn goloop_nested_custom_modules_override_original_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 4\n  log_types:\n    goloop:\n      enabled: true\n      format_type: json\n      custom_fields:\n        modules: [BM]\n        log_levels: [E]\n").unwrap();
    for defaults in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_log-forge"));
        command.args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--types",
            "goloop",
            "--no-monitor",
        ]);
        if defaults {
            command.arg("--original-defaults");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(stdout.lines().count(), 4);
        for line in stdout.lines() {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(record["module"], "BM");
            assert_eq!(record["level"], "E");
        }
    }
}

#[test]
fn syslog_nested_custom_fields_apply_weights_and_override_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 10\n  log_types:\n    syslog:\n      enabled: true\n      custom_fields:\n        facilities: {local3: 1, auth: 0}\n        severities: {err: 1, info: 0}\n        hostname: custom-host\n        priority_style: bracket\n").unwrap();
    for defaults in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_log-forge"));
        command.args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--types",
            "syslog",
            "--no-monitor",
        ]);
        if defaults {
            command.arg("--original-defaults");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let logs = String::from_utf8(output.stdout).unwrap();
        assert_eq!(logs.lines().count(), 10);
        for line in logs.lines() {
            assert!(line.starts_with("[local3.err]"), "{line}");
            assert!(line.contains(" custom-host backup"), "{line}");
        }
    }
    fs::write(&path, "log_generator:\n  log_types:\n    syslog:\n      enabled: true\n      custom_fields:\n        facilities: {local3: -1}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["config", "validate", "--file", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn syslog_top_level_weighted_fields_support_original_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 8\n  log_types:\n    syslog:\n      enabled: true\n      facilities: {local3: 1, auth: 0}\n      severities: {err: 1, info: 0}\n      priority_style: bracket\n").unwrap();
    for defaults in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_log-forge"));
        command.args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--types",
            "syslog",
            "--no-monitor",
        ]);
        if defaults {
            command.arg("--original-defaults");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let logs = String::from_utf8(output.stdout).unwrap();
        assert_eq!(logs.lines().count(), 8);
        assert!(logs.lines().all(|line| line.starts_with("[local3.err]")));
    }
    fs::write(&path, "log_generator:\n  log_types:\n    syslog:\n      enabled: true\n      facilities: {local3: -1}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["config", "validate", "--file", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn goloop_uses_network_and_blockchain_configuration() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-goloop-settings-{}.yaml",
        std::process::id()
    ));
    for (module, expected) in [
        ("NM", "10.20.30.40:"),
        ("IC", "network_id=0x99 revision=77"),
    ] {
        fs::write(&path, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 3\n  log_types:\n    goloop:\n      enabled: true\n      modules: {{{module}: 1}}\n      network:\n        use_real_ips: false\n        p2p_port: 7200\n        local_ip: 10.20.30.40\n      blockchain:\n        network_id: '0x99'\n        height_range: [42, 42]\n        revision_range: [77, 77]\n")).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--config", path.to_str().unwrap(), "--no-monitor"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        assert_eq!(output.lines().count(), 3);
        for line in output.lines() {
            assert!(line.contains(expected), "{line}");
            if module == "NM" {
                assert!(line.contains(":7200"), "{line}");
            }
        }
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn config_create_validate_show_and_run_json() {
    let root =
        std::env::temp_dir().join(format!("log-forge-config-command-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("config.json");
    let command = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(args)
            .output()
            .unwrap()
    };
    let create = command(&[
        "config",
        "create",
        "--format",
        "json",
        "--file",
        path.to_str().unwrap(),
    ]);
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );
    let validate = command(&["config", "validate", "--file", path.to_str().unwrap()]);
    assert!(
        validate.status.success(),
        "{}",
        String::from_utf8_lossy(&validate.stderr)
    );
    let show = command(&[
        "config",
        "show",
        "--file",
        path.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(
        show.status.success(),
        "{}",
        String::from_utf8_lossy(&show.stderr)
    );
    let shown: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(
        shown["log_generator"]["log_types"]["nginx_access"]["enabled"],
        true
    );
    let run = command(&[
        "start",
        "--config",
        path.to_str().unwrap(),
        "--output",
        "console",
        "--total",
        "1",
    ]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let before = fs::read(&path).unwrap();
    let duplicate = command(&[
        "config",
        "create",
        "--format",
        "json",
        "--file",
        path.to_str().unwrap(),
    ]);
    assert!(!duplicate.status.success());
    assert_eq!(before, fs::read(&path).unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn config_create_builds_parent_directories_without_overwriting() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nested/config/settings.yaml");
    let create = || {
        Command::new(env!("CARGO_BIN_EXE_log-generator"))
            .args(["config", "create", "--file", path.to_str().unwrap()])
            .output()
            .unwrap()
    };
    let first = create();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let before = fs::read(&path).unwrap();
    assert!(!create().status.success());
    assert_eq!(fs::read(&path).unwrap(), before);
    let validate = Command::new(env!("CARGO_BIN_EXE_log-generator"))
        .args(["config", "validate", "--file", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        validate.status.success(),
        "{}",
        String::from_utf8_lossy(&validate.stderr)
    );
}

#[test]
fn file_output_uses_default_filename_for_existing_directory() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("logs");
    fs::create_dir(&directory).unwrap();
    let path = directory.join("generated_logs.log");
    fs::write(&path, "existing record\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-generator"))
        .args([
            "start",
            "--output",
            "file",
            "--path",
            directory.to_str().unwrap(),
            "--types",
            "redis",
            "--total",
            "2",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let content = fs::read_to_string(path).unwrap();
    assert!(content.starts_with("existing record\n"));
    assert_eq!(content.lines().count(), 3);
}

#[test]
fn json_output_uses_default_filename_for_existing_directory() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("generated_logs.json");
    let output = Command::new(env!("CARGO_BIN_EXE_log-generator"))
        .args([
            "start",
            "--output",
            "json",
            "--path",
            root.path().to_str().unwrap(),
            "--types",
            "redis",
            "--total",
            "2",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let contents = fs::read_to_string(path).unwrap();
    assert_eq!(contents.lines().count(), 2);
    for line in contents.lines() {
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(record["log_type"], "redis");
    }
}

#[test]
fn config_create_uses_file_extension_and_rejects_format_conflicts() {
    let root = tempfile::tempdir().unwrap();
    for extension in ["json", "yaml", "yml"] {
        let path = root.path().join(format!("config.{extension}"));
        let output = Command::new(env!("CARGO_BIN_EXE_log-generator"))
            .args(["config", "create", "--file", path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{extension}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new(env!("CARGO_BIN_EXE_log-generator"))
            .args(["config", "validate", "--file", path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{extension}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let path = root.path().join("conflict.yaml");
    let output = Command::new(env!("CARGO_BIN_EXE_log-generator"))
        .args([
            "config",
            "create",
            "--file",
            path.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!path.exists());
}

#[test]
fn custom_template_supports_builtin_fields_and_counters() {
    let path = std::env::temp_dir().join(format!("log-forge-template-{}.yaml", std::process::id()));
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 3\n    start_time: '2024-01-01'\n  log_types:\n    custom:\n      enabled: true\n      template: '{counter:start:100,step:5} {counter:start:100,step:5} {status:200,404} {random_int:min_val:7,max_val:7} {uuid} {iso_timestamp}'\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), 3);
    for (line, expected) in lines.iter().zip([100, 105, 110]) {
        let parts: Vec<_> = line.split_whitespace().collect();
        assert_eq!(parts[0], expected.to_string());
        assert_eq!(parts[1], parts[0]);
        assert!(["200", "404"].contains(&parts[2]));
        assert_eq!(parts[3], "7");
        assert_eq!(parts[4].len(), 36);
        assert!(parts[5].starts_with("2024-01-01T00:00:00"));
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn custom_template_selects_file_lines_and_rejects_missing_files() {
    let root = std::env::temp_dir().join(format!("log-forge-template-file-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let values = root.join("values.txt");
    let config = root.join("config.yaml");
    fs::write(&values, "alpha\n\nbeta\n").unwrap();
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 10\n  log_types:\n    custom:\n      enabled: true\n      template: '{{value:file:{0}}} {{value:file:{0}}}'\n", values.display())).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let result = run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    assert_eq!(output.lines().count(), 10);
    for line in output.lines() {
        assert!(["alpha alpha", "beta beta"].contains(&line), "{line}");
    }
    fs::remove_file(values).unwrap();
    let result = run();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn termination_flushes_buffered_output_and_interrupts_wait() {
    let path = std::env::temp_dir().join(format!("log-forge-signal-{}.log", std::process::id()));
    let mut child = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "file",
            "--path",
            path.to_str().unwrap(),
            "--types",
            "redis",
            "--total",
            "0",
            "--real-time",
            "--interval",
            "60",
        ])
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(path.exists());
    std::thread::sleep(std::time::Duration::from_millis(50));
    let signal = Command::new("sh")
        .args(["-c", "kill -TERM \"$1\"", "sh", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(signal.success());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("generator did not stop promptly");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!(status.success(), "{status}");
    assert!(fs::read_to_string(&path).unwrap().contains(":M "));
    fs::remove_file(path).unwrap();
}

#[cfg(unix)]
#[test]
fn control_socket_reports_pauses_resumes_and_stops_generation() {
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let root = std::env::temp_dir().join(format!("log-forge-control-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let socket = root.join("control.sock");
    let output = root.join("output.log");
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "file",
                "--path",
                output.to_str().unwrap(),
                "--types",
                "redis",
                "--total",
                "0",
                "--real-time",
                "--interval",
                "0.05",
                "--no-monitor",
                "--control-socket",
                socket.to_str().unwrap(),
            ])
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while !socket.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(socket.exists(), "control socket was not created");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
        0o600
    );
    {
        use std::io::{Read, Write};
        let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        stream.write_all(b"status\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(parsed["state"], "running");
    }
    let call = |action: &str| {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([action, "--control-socket", socket.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{action}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap()
    };
    let status = call("status");
    assert_eq!(status["state"], "running");
    assert_eq!(call("pause")["state"], "paused");
    std::thread::sleep(std::time::Duration::from_millis(120));
    let first = call("status")["generated"].as_u64().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(120));
    assert_eq!(call("status")["generated"].as_u64().unwrap(), first);
    assert_eq!(call("resume")["state"], "running");
    std::thread::sleep(std::time::Duration::from_millis(150));
    assert!(call("status")["generated"].as_u64().unwrap() > first);
    assert_eq!(call("stop")["state"], "stopping");
    assert!(child.0.wait().unwrap().success());
    assert!(!socket.exists());
    assert!(fs::read_to_string(output).unwrap().contains(":M "));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn control_socket_does_not_replace_existing_file() {
    let root =
        std::env::temp_dir().join(format!("log-forge-socket-collision-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let socket = root.join("control.sock");
    fs::write(&socket, "existing").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "redis",
            "--total",
            "1",
            "--control-socket",
            socket.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read_to_string(&socket).unwrap(), "existing");
    assert!(result.stdout.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn monitor_streams_status_and_recent_logs() {
    use std::io::{BufRead, BufReader};
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let root =
        std::env::temp_dir().join(format!("log-forge-monitor-socket-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let socket = root.join("control.sock");
    let output = root.join("output.log");
    let mut server = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "file",
                "--path",
                output.to_str().unwrap(),
                "--types",
                "redis",
                "--total",
                "0",
                "--real-time",
                "--interval",
                "0.05",
                "--no-monitor",
                "--control-socket",
                socket.to_str().unwrap(),
            ])
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while !socket.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(socket.exists());
    std::thread::sleep(std::time::Duration::from_millis(150));
    let mut monitor = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "monitor",
                "--control-socket",
                socket.to_str().unwrap(),
                "--refresh",
                "0.05",
                "--show-logs",
            ])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut reader = BufReader::new(monitor.0.stdout.take().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let state: serde_json::Value = serde_json::from_str(&first).unwrap();
    assert_eq!(state["state"], "running");
    assert!(state["generated"].as_u64().unwrap() > 0);
    assert!(
        state["recent_logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["message"].as_str().unwrap().contains(":M ") })
    );
    let stop = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["stop", "--control-socket", socket.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(stop.status.success());
    assert!(server.0.wait().unwrap().success());
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut reader, &mut rest).unwrap();
    assert!(monitor.0.wait().unwrap().success());
    assert!(
        rest.lines()
            .any(|line| line.contains("\"state\":\"disconnected\"")
                || line.contains("\"state\":\"stopping\"")),
        "{rest}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn real_time_does_not_wait_after_last_record() {
    let started = std::time::Instant::now();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "redis",
            "--total",
            "1",
            "--real-time",
            "--interval",
            "3",
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
fn batch_size_flushes_before_generation_finishes() {
    let root = std::env::temp_dir().join(format!("log-forge-batch-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("out.log");
    let config = root.join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: file\n    output_path: '{}'\n    total_logs: 0\n    virtual_time: false\n    generation_interval: 10\n    buffer_size: 65536\n    batch_size: 1\n  performance:\n    batch_size: 100\n    buffer_size: 65536\n  log_types:\n    redis:\n      enabled: true\n", path.display())).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", config.to_str().unwrap()])
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while fs::metadata(&path).map_or(true, |metadata| metadata.len() == 0)
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let flushed = fs::read_to_string(&path).unwrap_or_default();
    let signal = Command::new("sh")
        .args(["-c", "kill -TERM \"$1\"", "sh", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(signal.success());
    let status = child.wait().unwrap();
    assert!(status.success());
    assert!(flushed.contains(":M "));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn syslog_supports_rfc5424_and_priority_styles() {
    let path = std::env::temp_dir().join(format!("log-forge-syslog-{}.yaml", std::process::id()));
    for (format, style, expected) in [
        (
            "5424",
            "pri",
            "<155>1 2024-01-01T00:00:00.000Z host.example backup ",
        ),
        (
            "3164",
            "bracket",
            "[local3.err]Jan 01 00:00:00 host.example backup",
        ),
        (
            "3164",
            "plain",
            "local3.err Jan 01 00:00:00 host.example backup",
        ),
        (
            "5424",
            "none",
            "1 2024-01-01T00:00:00.000Z host.example backup ",
        ),
    ] {
        fs::write(&path, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 1\n    start_time: '2024-01-01'\n  log_types:\n    syslog:\n      enabled: true\n      rfc_format: '{format}'\n      priority_style: {style}\n      hostname: host.example\n      facilities: [local3]\n      severities: [err]\n")).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--config", path.to_str().unwrap()])
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
                .starts_with(expected)
        );
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn syslog_facilities_use_realistic_expanded_patterns() {
    let root = std::env::temp_dir().join(format!(
        "log-forge-syslog-patterns-{}.yaml",
        std::process::id()
    ));
    let placeholders = regex::Regex::new(r"\{[A-Za-z_][A-Za-z_0-9]*(?::[^}]*)?\}").unwrap();
    for facility in [
        "kern", "daemon", "auth", "mail", "cron", "syslog", "authpriv", "user", "local0", "local1",
        "local2", "local3",
    ] {
        fs::write(&root, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 300\n  log_types:\n    syslog:\n      enabled: true\n      facilities: [{facility}]\n      severities: [info]\n")).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                root.to_str().unwrap(),
                "--seed",
                "9",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{facility}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        let messages: std::collections::HashSet<_> = output
            .lines()
            .map(|line| line.split_once(": ").unwrap().1)
            .collect();
        assert!(
            messages.len() > 1,
            "{facility} must produce varied messages"
        );
        assert!(
            messages.iter().all(|line| !placeholders.is_match(line)),
            "{facility}: {messages:?}"
        );
    }
    fs::write(&root, "log_generator: {}\n").unwrap();
    let default = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            root.to_str().unwrap(),
            "--output",
            "console",
            "--types",
            "syslog",
            "--total",
            "200",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(default.status.success());
    let text = String::from_utf8(default.stdout).unwrap();
    let facilities: std::collections::HashSet<u16> = text
        .lines()
        .map(|line| {
            line.strip_prefix('<')
                .unwrap()
                .split_once('>')
                .unwrap()
                .0
                .parse::<u16>()
                .unwrap()
                / 8
        })
        .collect();
    assert!(facilities.len() > 5);
    fs::remove_file(root).unwrap();
}

#[test]
fn docker_generates_json_and_configured_text() {
    let json = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "docker",
            "--total",
            "1",
            "--start-time",
            "2024-01-01",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(json.status.success());
    let data: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(data["time"], "2024-01-01T00:00:00.000Z");
    assert!(["stdout", "stderr"].contains(&data["stream"].as_str().unwrap()));
    assert!(data["log"].as_str().unwrap().ends_with('\n'));
    let path = std::env::temp_dir().join(format!("log-forge-docker-{}.yaml", std::process::id()));
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 2\n  log_types:\n    docker:\n      enabled: true\n      format_type: text\n      log_levels: {error: 1}\n      container_names: [backend]\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", path.to_str().unwrap(), "--no-monitor"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for line in String::from_utf8(result.stdout).unwrap().lines() {
        assert!(line.contains("[ERROR] backend:"), "{line}");
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn kubernetes_levels_namespaces_and_event_types_are_applied() {
    let path =
        std::env::temp_dir().join(format!("log-forge-k8s-config-{}.yaml", std::process::id()));
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 100\n  log_types:\n    kubernetes_pod:\n      enabled: true\n      custom_fields:\n        log_levels: {ERROR: 1}\n        namespaces: [test-ns]\n        pod_name: api-123\n    kubernetes_event:\n      enabled: true\n      custom_fields:\n        event_types: {Warning: 1}\n").unwrap();
    for (kind, expected) in [
        ("kubernetes_pod", " ERROR "),
        ("kubernetes_event", " Warning "),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--types",
                kind,
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        assert_eq!(output.lines().count(), 100);
        for line in output.lines() {
            assert!(line.contains(expected), "{line}");
        }
        if kind == "kubernetes_pod" {
            assert!(output.contains("api-123"));
            assert!(output.contains("test-ns"));
        } else {
            assert!(
                output
                    .lines()
                    .filter_map(|line| line.split_whitespace().nth(2))
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    > 1
            );
        }
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn docker_and_kubernetes_generate_varied_complete_messages() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-container-patterns-{}.yaml",
        std::process::id()
    ));
    let placeholder = regex::Regex::new(r"\{[a-zA-Z_][a-zA-Z_0-9]*\}").unwrap();
    for (kind, level) in [
        ("docker", "error"),
        ("docker", "info"),
        ("kubernetes_pod", "ERROR"),
        ("kubernetes_pod", "INFO"),
    ] {
        fs::write(&path, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 100\n  log_types:\n    {kind}:\n      enabled: true\n      log_levels: {{{level}: 1}}\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--config", path.to_str().unwrap(), "--no-monitor"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let content = String::from_utf8(output.stdout).unwrap();
        let messages: Vec<String> = content
            .lines()
            .map(|line| {
                if kind == "docker" {
                    serde_json::from_str::<serde_json::Value>(line).unwrap()["log"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                } else {
                    line.split_once(&format!(" {level} ")).unwrap().1.to_owned()
                }
            })
            .collect();
        assert_eq!(messages.len(), 100);
        assert!(
            messages
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 5
        );
        assert!(
            messages
                .iter()
                .all(|message| !placeholder.is_match(message)),
            "{kind}: {messages:?}"
        );
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn database_generators_honor_query_types_tables_and_levels() {
    let path =
        std::env::temp_dir().join(format!("log-forge-db-config-{}.yaml", std::process::id()));
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 100\n  log_types:\n    mysql_query:\n      enabled: true\n      custom_fields:\n        query_types: {INSERT: 1}\n        table_names: [audit_events]\n    postgresql:\n      enabled: true\n      custom_fields:\n        log_levels: {LOG: 1}\n        query_types: {UPDATE: 1}\n        table_names: [audit_events]\n").unwrap();
    for kind in ["mysql_query", "postgresql"] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--types",
                kind,
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        assert_eq!(output.lines().count(), 100);
        if kind == "mysql_query" {
            let queries: Vec<_> = output
                .lines()
                .filter(|line| line.contains(" Query "))
                .collect();
            assert!(!queries.is_empty());
            assert!(
                queries
                    .iter()
                    .all(|line| line.contains("INSERT INTO audit_events"))
            );
        } else {
            assert!(output.lines().all(|line| line.contains("[LOG]")));
            assert!(
                output
                    .lines()
                    .any(|line| line.contains("UPDATE audit_events"))
            );
            assert!(!output.contains("SELECT * FROM users"));
        }
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn fastapi_uses_weighted_settings_and_resolves_endpoint_parameters() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-fastapi-config-{}.yaml",
        std::process::id()
    ));
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 100\n  log_types:\n    fastapi:\n      enabled: true\n      custom_fields:\n        log_levels: {INFO: 1}\n        endpoints: ['/api/users/{user_id}']\n        status_codes: {503: 1}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--no-monitor",
            "--seed",
            "42",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.lines().all(|line| line.contains("[INFO]")));
    let requests: Vec<_> = output
        .lines()
        .filter(|line| line.contains("Request completed:"))
        .collect();
    assert!(requests.len() > 30);
    for request in requests {
        assert!(
            request.contains("/api/users/") && request.contains(" - 503 - "),
            "{request}"
        );
        assert!(!request.contains("{user_id}"));
        let duration = request
            .rsplit_once(" - ")
            .unwrap()
            .1
            .trim_end_matches("ms")
            .parse::<u32>()
            .unwrap();
        assert!(duration >= 2000);
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn fastapi_message_patterns_render_for_each_level() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-fastapi-levels-{}.yaml",
        std::process::id()
    ));
    let placeholder = regex::Regex::new(r"\{[a-zA-Z_][a-zA-Z_0-9]*\}").unwrap();
    for level in ["INFO", "DEBUG", "WARNING", "ERROR"] {
        fs::write(&path, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 300\n  log_types:\n    fastapi:\n      enabled: true\n      log_levels: {{{level}: 1}}\n")).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--config", path.to_str().unwrap(), "--no-monitor"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{level}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        assert_eq!(output.lines().count(), 300);
        assert!(
            output.lines().all(|line| !placeholder.is_match(line)),
            "{level}: {output}"
        );
        assert!(
            output
                .lines()
                .filter_map(|line| line.split_once(": ").map(|(_, message)| message))
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 5
        );
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn configured_service_names_generate_deterministic_service_events() {
    let path = std::env::temp_dir().join(format!("log-forge-services-{}.yaml", std::process::id()));
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 100\n    start_time: '2024-01-01'\n  log_types:\n    user_service:\n      enabled: true\n      frequency: 1\n      service_name: user-service\n      operations: [user_registration]\n      log_levels: {INFO: 1}\n      custom_fields:\n        events: ['User {user_id} registered']\n    order_service:\n      enabled: true\n      frequency: 1\n      service_name: order-service\n      operations: [order_creation]\n      log_levels: {INFO: 1}\n      custom_fields:\n        events: ['Order {order_id} created for user {user_id}']\n").unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "23",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let result = run("1");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let parallel = run("4");
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(result.stdout, parallel.stdout);
    let output = String::from_utf8(result.stdout).unwrap();
    assert_eq!(output.lines().count(), 100);
    assert!(output.contains("service=user-service operation=user_registration User "));
    assert!(output.contains("service=order-service operation=order_creation Order "));
    assert!(!output.contains("{user_id}"));
    assert!(!output.contains("{order_id}"));
    let validator = log_forge::validation::LogValidator::new("microservice").unwrap();
    assert!(output.lines().all(|line| validator.validate(line).is_valid));
    fs::remove_file(path).unwrap();
}

#[test]
fn service_traces_follow_dependencies_and_match_across_threads() {
    let path = std::env::temp_dir().join(format!("log-forge-traces-{}.yaml", std::process::id()));
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 4\n    start_time: '2024-01-01'\n  distributed_tracing:\n    enabled: true\n    trace_id_header: X-Trace-Id\n    span_id_header: X-Span-Id\n    service_dependencies:\n      gateway: [users]\n  log_types:\n    gateway_log:\n      enabled: true\n      frequency: 1\n      service_name: gateway\n      custom_fields:\n        events: ['request received']\n    user_log:\n      enabled: true\n      frequency: 0\n      service_name: users\n      custom_fields:\n        events: ['user lookup']\n").unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "7",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(serial.stdout, parallel.stdout);
    let many = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--threads",
                threads,
                "--total",
                "600",
                "--seed",
                "7",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let single_many = many("1");
    let parallel_many = many("4");
    assert!(single_many.status.success() && parallel_many.status.success());
    assert_eq!(single_many.stdout, parallel_many.stdout);
    let output = String::from_utf8(serial.stdout).unwrap();
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), 4);
    let value = |line: &str, key: &str| -> String {
        line.split_whitespace()
            .find_map(|field| field.strip_prefix(&format!("{key}=")))
            .unwrap()
            .to_owned()
    };
    for pair in lines.chunks(2) {
        assert!(pair[0].contains("service=gateway"));
        assert!(pair[1].contains("service=users"));
        assert_eq!(value(pair[0], "trace_id"), value(pair[1], "trace_id"));
        assert_eq!(value(pair[0], "span_id"), value(pair[1], "parent_span_id"));
        assert_ne!(value(pair[0], "span_id"), value(pair[1], "span_id"));
        assert_eq!(value(pair[0], "X-Trace-Id"), value(pair[0], "trace_id"));
        assert_eq!(value(pair[1], "X-Span-Id"), value(pair[1], "span_id"));
    }
    let json_path =
        std::env::temp_dir().join(format!("log-forge-traces-{}.json", std::process::id()));
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--output",
            "json",
            "--path",
            json_path.to_str().unwrap(),
            "--seed",
            "7",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let data = fs::read_to_string(&json_path).unwrap();
    let rows: Vec<serde_json::Value> = data
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows[0]["trace_id"], rows[1]["trace_id"]);
    assert_eq!(rows[0]["span_id"], rows[1]["parent_span_id"]);
    fs::remove_file(json_path).unwrap();
    fs::remove_file(path).unwrap();
}

#[test]
fn service_tracing_rejects_cycles_and_unknown_dependencies() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-trace-invalid-{}.yaml",
        std::process::id()
    ));
    for graph in ["a: [b]\n      b: [a]", "a: [missing]"] {
        fs::write(&path, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 1\n  distributed_tracing:\n    enabled: true\n    service_dependencies:\n      {graph}\n  log_types:\n    service_a:\n      enabled: true\n      service_name: a\n      custom_fields:\n        events: ['a']\n    service_b:\n      enabled: true\n      service_name: b\n      custom_fields:\n        events: ['b']\n")).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["config", "validate", "--file", path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(if graph.contains("missing") {
                "unknown tracing service"
            } else {
                "dependency cycle"
            })
        );
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn django_settings_apply_to_requests_and_sql_models() {
    let path = std::env::temp_dir().join(format!("log-forge-django-{}.yaml", std::process::id()));
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 20\n  log_types:\n    django_request:\n      enabled: true\n      custom_fields:\n        log_levels: {WARNING: 1}\n        status_codes: {404: 1}\n        url_patterns: ['/django-only/']\n    django_sql:\n      enabled: true\n      custom_fields:\n        models: [Ledger]\n").unwrap();
    for kind in ["django_request", "django_sql"] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--types",
                kind,
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        assert_eq!(output.lines().count(), 20);
        for line in output.lines() {
            if kind == "django_request" {
                assert!(
                    line.contains(" WARNING django.request:")
                        && line.contains("/django-only/\" 404 "),
                    "{line}"
                );
            } else {
                assert!(line.contains("\"app_ledger\""), "{line}");
            }
        }
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn json_output_maps_original_fields_and_custom_names() {
    let root = std::env::temp_dir().join(format!("log-forge-json-fields-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let output = root.join("out.json");
    let config = root.join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: json\n    output_path: '{}'\n    total_logs: 1\n    start_time: '2024-01-01'\n    include_raw_message: false\n    field_mapping:\n      message: event.original\n  log_types:\n    nginx_access:\n      enabled: true\n      custom_fields:\n        ip_ranges: ['10.9.8.7/32']\n        status_codes: {{503: 1}}\n", output.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let data: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(output).unwrap()).unwrap();
    assert_eq!(data["@version"], "1");
    assert_eq!(data["@timestamp"], "2024-01-01T00:00:00+00:00");
    assert_eq!(data["type"], "nginx_access");
    assert_eq!(data["source"]["ip"], "10.9.8.7");
    assert_eq!(data["http"]["response"]["status_code"], 503);
    assert!(
        data["event"]["original"]
            .as_str()
            .unwrap()
            .contains("10.9.8.7")
    );
    assert!(data.get("raw_message").is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn json_output_respects_rotation_settings() {
    let root = std::env::temp_dir().join(format!("log-forge-json-rotation-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let output = root.join("out.json");
    let config = root.join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: json\n    output_path: '{}'\n    total_logs: 3\n    log_rotation:\n      enabled: true\n      max_size: 1\n      max_files: 2\n  log_types:\n    redis:\n      enabled: true\n", output.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for name in ["out.json", "out.json.1", "out.json.2"] {
        let record: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(name)).unwrap()).unwrap();
        assert_eq!(record["type"], "redis");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn multiple_handlers_receive_the_same_json_records() {
    use std::io::Read;
    let root = std::env::temp_dir().join(format!("log-forge-fanout-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let reader = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let output = root.join("output.json");
    let config = root.join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    total_logs: 5\n    start_time: '2024-01-01'\n  log_types:\n    redis:\n      enabled: true\n  output:\n    handlers:\n      - type: file\n        file_path: '{}'\n        format: json\n      - type: network\n        protocol: tcp\n        host: 127.0.0.1\n        port: {port}\n        format: json\n", output.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--async-io",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let network = reader.join().unwrap();
    let file = fs::read(output).unwrap();
    assert_eq!(file, network);
    assert_eq!(String::from_utf8(file).unwrap().lines().count(), 5);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn elk_and_splunk_json_handlers_use_their_field_mappings() {
    let root = tempfile::tempdir().unwrap();
    let elk = root.path().join("elk.json");
    let splunk = root.path().join("splunk.json");
    let config = root.path().join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    total_logs: 2\n    start_time: '2024-01-01'\n  log_types:\n    nginx_access: {{enabled: true}}\n  output:\n    handlers:\n      - type: file\n        file_path: '{}'\n        format: elk_json\n      - type: file\n        file_path: '{}'\n        format: splunk_json\n", elk.display(), splunk.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config"])
        .arg(&config)
        .arg("--no-monitor")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let elk_records = fs::read_to_string(elk).unwrap();
    let splunk_records = fs::read_to_string(splunk).unwrap();
    assert_eq!(elk_records.lines().count(), 2);
    assert_eq!(splunk_records.lines().count(), 2);
    for (elk, splunk) in elk_records.lines().zip(splunk_records.lines()) {
        let elk: serde_json::Value = serde_json::from_str(elk).unwrap();
        let splunk: serde_json::Value = serde_json::from_str(splunk).unwrap();
        assert_eq!(elk["message"], splunk["_raw"]);
        assert_eq!(elk["source"]["ip"], splunk["src_ip"]);
        assert!(elk.get("raw_message").is_some());
        assert!(splunk.get("raw_message").is_none());
        assert_eq!(splunk["sourcetype"], "nginx_access");
    }
}

#[test]
fn elk_and_splunk_http_handlers_apply_mapping_and_raw_overrides() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        (0..2)
            .map(|_| receive_remote_write(&listener))
            .collect::<Vec<_>>()
    });
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    total_logs: 1\n    start_time: '2024-01-01'\n    include_raw_message: false\n    field_mapping: {{level: event.severity}}\n  log_types:\n    redis: {{enabled: true}}\n  output:\n    handlers:\n      - type: network\n        protocol: http\n        host: 127.0.0.1\n        port: {port}\n        endpoint: /elk\n        format: elk_json\n        include_raw_message: true\n      - type: network\n        protocol: http\n        host: 127.0.0.1\n        port: {port}\n        endpoint: /splunk\n        format: splunk_json\n")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config"])
        .arg(&config)
        .arg("--no-monitor")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = server.join().unwrap();
    for (index, (headers, body)) in requests.iter().enumerate() {
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("content-type: application/json")
        );
        let value: serde_json::Value = serde_json::from_slice(body).unwrap();
        let raw = if index == 0 {
            &value["message"]
        } else {
            &value["_raw"]
        }
        .as_str()
        .unwrap();
        let expected = match raw.split_whitespace().nth(5).unwrap() {
            "*" => "NOTICE",
            "#" => "WARNING",
            "-" => "VERBOSE",
            "." => "DEBUG",
            other => panic!("unexpected Redis severity: {other}"),
        };
        assert_eq!(value["event"]["severity"], expected);
        if index == 0 {
            assert!(headers.starts_with("POST /elk "));
            assert!(value.get("raw_message").is_some());
        } else {
            assert!(headers.starts_with("POST /splunk "));
            assert!(value.get("raw_message").is_none());
            assert_eq!(value["sourcetype"], "redis");
        }
    }
}

#[test]
fn http_handler_posts_json_and_propagates_server_errors() {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        let mut headers = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            headers.push_str(&line);
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        stream
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        (first, headers, body)
    });
    let root = std::env::temp_dir().join(format!("log-forge-http-{}.yaml", std::process::id()));
    fs::write(&root, format!("log_generator:\n  global:\n    total_logs: 1\n  log_types:\n    redis:\n      enabled: true\n  output:\n    handlers:\n      - type: network\n        protocol: http\n        host: 127.0.0.1\n        port: {port}\n        endpoint: /ingest\n        format: json\n")).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", root.to_str().unwrap(), "--no-monitor"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("HTTP 400"));
    let (request, headers, body) = server.join().unwrap();
    assert_eq!(request, "POST /ingest HTTP/1.1\r\n");
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("content-type: application/json")
    );
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["type"], "redis");
    fs::remove_file(root).unwrap();
}

#[test]
fn http_handler_sends_each_record_on_success() {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let mut records = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while records.len() < 3 {
            let (mut stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                }
                Err(error) => panic!("HTTP accept failed: {error}"),
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            records.push(String::from_utf8(body).unwrap());
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        }
        records
    });
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "network",
            "--protocol",
            "http",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--endpoint",
            "/logs",
            "--types",
            "redis",
            "--total",
            "3",
            "--async-io",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let records = server.join().unwrap();
    assert_eq!(records.len(), 3);
    assert!(records.iter().all(|record| record.contains(":M ")));
}

#[test]
fn invalid_handler_is_rejected_before_any_output_is_created() {
    let root =
        std::env::temp_dir().join(format!("log-forge-invalid-fanout-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let output = root.join("should-not-exist.log");
    let config = root.join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    total_logs: 1\n  log_types:\n    redis:\n      enabled: true\n  output:\n    handlers:\n      - type: file\n        file_path: '{}'\n      - type: network\n        protocol: http\n        host: 127.0.0.1\n        port: 9090\n        format: unsupported\n", output.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!output.exists());
    let override_output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--output",
            "console",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        override_output.status.success(),
        "{}",
        String::from_utf8_lossy(&override_output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&override_output.stderr).contains("output handlers are overridden")
    );
    assert!(!output.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_fanout_failure_preserves_earlier_file_and_skips_later_file() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first.log");
    let last = root.path().join("last.log");
    let config = root.path().join("config.yaml");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Err(error) => panic!("HTTP receiver failed: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let mut bytes = [0_u8; 4096];
        assert!(stream.read(&mut bytes).unwrap() > 0);
        stream
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
    });
    fs::write(&config, format!("log_generator:\n  global: {{total_logs: 1}}\n  log_types:\n    redis: {{enabled: true}}\n  output:\n    handlers:\n      - {{type: file, file_path: '{}'}}\n      - {{type: network, protocol: http, host: 127.0.0.1, port: {port}, endpoint: /logs}}\n      - {{type: file, file_path: '{}'}}\n", first.display(), last.display())).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("400"));
    assert_eq!(fs::read_to_string(first).unwrap().lines().count(), 1);
    assert!(fs::read(last).unwrap().is_empty());
}

#[test]
fn conflicting_json_mappings_fail_validation_before_file_creation() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let output_path = root.path().join("out.json");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: json\n    output_path: '{}'\n    total_logs: 1\n    field_mapping: {{message: '@timestamp'}}\n  log_types:\n    redis: {{enabled: true}}\n", output_path.display())).unwrap();
    for args in [
        vec!["config", "validate", "--file"],
        vec!["start", "--config"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(args)
            .arg(&config)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("conflicting JSON field mappings")
        );
        assert!(!output_path.exists());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--output",
            "console",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output_path.exists());
}

#[test]
fn json_handler_presets_reject_conflicts_before_opening_files() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let output_path = root.path().join("out.json");
    for (format, conflicting_field) in [
        ("json", "type"),
        ("elk_json", "type"),
        ("splunk_json", "sourcetype"),
    ] {
        fs::write(&config, format!("log_generator:\n  global:\n    total_logs: 1\n    field_mapping: {{message: '{conflicting_field}'}}\n  log_types:\n    redis: {{enabled: true}}\n  output:\n    handlers:\n      - type: file\n        file_path: '{}'\n        format: {format}\n", output_path.display())).unwrap();
        for command in ["validate", "start"] {
            let mut process = Command::new(env!("CARGO_BIN_EXE_log-forge"));
            if command == "validate" {
                process.args(["config", "validate", "--file"]);
            } else {
                process.args(["start", "--config"]);
            }
            let output = process.arg(&config).output().unwrap();
            assert!(!output.status.success(), "{format} {command}");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("conflicting JSON field mappings"),
                "{format} {command}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!output_path.exists());
        }
    }
}

#[test]
fn unknown_json_mapping_key_fails_before_output_creation() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let path = root.path().join("output.json");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: json\n    output_path: '{}'\n    total_logs: 1\n    field_mapping: {{mesage: event}}\n  log_types:\n    redis: {{enabled: true}}\n", path.display())).unwrap();
    for args in [
        vec!["config", "validate", "--file"],
        vec!["start", "--config"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(args)
            .arg(&config)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unknown JSON mapping field: mesage")
        );
        assert!(!path.exists());
    }
}

#[test]
fn scenario_invalid_pattern_fails_before_output_creation() {
    let root = tempfile::tempdir().unwrap();
    let scenario = root.path().join("scenario.json");
    let output = root.path().join("out.log");
    fs::write(
        &scenario,
        r#"{"name":"test","patterns":["sample"],"sequence_rules":{}}"#,
    )
    .unwrap();
    let mut choices = vec![serde_json::json!("valid"); 100];
    choices.clear();
    let pattern = serde_json::json!({"id":"sample", "category":"test", "severity":"error", "message_template":"{value}", "context_fields":{"value":choices}});
    fs::write(
        root.path().join("sample.json"),
        serde_json::to_vec(&pattern).unwrap(),
    )
    .unwrap();
    let config = root.path().join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: file\n    output_path: '{}'\n    total_logs: 1\n  log_types:\n    scenario:\n      scenario_file: '{}'\n      patterns_dir: '{}'\n", output.display(), scenario.display(), root.path().display())).unwrap();
    for command in ["validate", "start"] {
        let mut process = Command::new(env!("CARGO_BIN_EXE_log-forge"));
        if command == "validate" {
            process.args(["config", "validate", "--file"]);
        } else {
            process.args(["start", "--config"]);
        }
        let result = process.arg(&config).output().unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("invalid context value"));
        assert!(!output.exists());
    }
}

#[test]
fn scenario_cli_applies_fractional_delays_and_duration() {
    let root = tempfile::tempdir().unwrap();
    let scenario = root.path().join("scenario.json");
    fs::write(&scenario, r#"{"name":"fractional","patterns":["nginx_timeout","nginx_connection_refused"],"sequence_rules":{"escalation_chain":["nginx_timeout","nginx_connection_refused"],"escalation_delay_minutes":[0.25]},"duration_minutes":0.5}"#).unwrap();
    let config = root.path().join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global: {{output_format: console, total_logs: 3, generation_interval: 15, start_time: '2024-01-01'}}\n  log_types:\n    scenario:\n      scenario_file: '{}'\n", scenario.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config"])
        .arg(config)
        .arg("--no-monitor")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("pattern=nginx_timeout"));
    assert!(lines[1].contains("pattern=nginx_connection_refused"));
    assert!(lines[2].contains("pattern=nginx_connection_refused"));
    let path = root.path().join("scenario_sample.log");
    fs::write(&path, &output).unwrap();
    for action in ["validate", "analyze"] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([action, "--input"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{action}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn scenario_generates_escalating_errors_from_pattern_files() {
    let root = std::env::temp_dir().join(format!("log-forge-scenario-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    for (id, message) in [
        ("slow", "slow {service}"),
        ("timeout", "timeout {service}"),
        ("failed", "failed {service}"),
    ] {
        let pattern = serde_json::json!({"id": id, "category": "app", "severity": "error", "message_template": message, "description": "test", "context_fields": {"service": ["api"]}});
        fs::write(
            root.join(format!("{id}.json")),
            serde_json::to_vec(&pattern).unwrap(),
        )
        .unwrap();
    }
    let scenario = root.join("scenario.json");
    fs::write(&scenario, r#"{"name":"cascade","description":"test","category":"app","patterns":["slow","timeout","failed"],"sequence_rules":{"start_pattern":"slow","escalation_chain":["slow","timeout","failed"],"escalation_delay_minutes":[1,2]},"duration_minutes":3,"escalation_probability":1.0}"#).unwrap();
    let config = root.join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 3\n    generation_interval: 60\n    start_time: '2024-01-01'\n  log_types:\n    scenario:\n      enabled: true\n      scenario_file: '{}'\n      patterns_dir: '{}'\n", scenario.display(), root.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", config.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), 3);
    for (line, expected) in lines.iter().zip(["slow api", "timeout api", "failed api"]) {
        assert!(line.contains(expected), "{line}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn original_scenario_resolves_builtin_and_custom_patterns() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let config = std::env::temp_dir().join(format!(
        "log-forge-builtin-scenario-{}.yaml",
        std::process::id()
    ));
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 4\n    generation_interval: 120\n    start_time: '2024-01-01'\n  log_types:\n    scenario:\n      enabled: true\n      scenario_file: '{}'\n      patterns_dir: '{}'\n", fixtures.join("error_scenarios/microservice_cascade_failure.json").display(), fixtures.join("error_patterns").display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", config.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), 4);
    assert!(lines[0].contains("pattern=mysql_connection_lost"));
    assert!(lines[1].contains("pattern=api_gateway_timeout"));
    assert!(lines[3].contains("pattern=kubernetes_pod_evicted"));
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("Using built-in error pattern: mysql_connection_lost")
    );
    fs::remove_file(config).unwrap();
}

#[test]
fn scenario_frequency_skips_attempts_and_keeps_virtual_time() {
    let root = std::env::temp_dir().join(format!(
        "log-forge-scenario-frequency-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let scenario = root.join("scenario.json");
    fs::write(&scenario, r#"{"name":"rate-test","patterns":["nginx_timeout"],"sequence_rules":{},"duration_minutes":30}"#).unwrap();
    let config = root.join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 8\n    generation_interval: 1\n    start_time: '2024-01-01'\n  log_types:\n    scenario:\n      enabled: true\n      scenario_file: '{}'\n      error_frequency:\n        base_frequency: 30\n        peak_hours: []\n        burst_probability: 0\n", scenario.display())).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let result = run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), 8);
    let timestamp = |line: &str| {
        chrono::DateTime::parse_from_rfc3339(line.split_whitespace().next().unwrap()).unwrap()
    };
    assert!((timestamp(lines[7]) - timestamp(lines[0])).num_seconds() > 7);
    assert_eq!(output.as_bytes(), run().stdout);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn prometheus_handler_runs_from_config_with_async_output() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        (0..4)
            .map(|_| receive_remote_write(&listener))
            .collect::<Vec<_>>()
    });
    let config =
        std::env::temp_dir().join(format!("log-forge-prometheus-{}.yaml", std::process::id()));
    fs::write(&config, format!("log_generator:\n  global:\n    total_logs: 3\n    batch_size: 1\n  log_types:\n    redis:\n      enabled: true\n  output:\n    handlers:\n      - type: network\n        protocol: http\n        host: 127.0.0.1\n        port: {port}\n        endpoint: /api/v1/write\n        format: prometheus\n")).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--async-io",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 4);
    let (headers, body) = &requests[0];
    assert!(headers.starts_with("POST /api/v1/write HTTP/1.1\r\n"));
    assert!(headers.contains("Content-Encoding: snappy\r\n"));
    let decoded = snap::raw::Decoder::new().decompress_vec(body).unwrap();
    assert!(
        decoded
            .windows("log_forge_records_total".len())
            .any(|part| part == b"log_forge_records_total")
    );
    let marker = snap::raw::Decoder::new()
        .decompress_vec(&requests[3].1)
        .unwrap();
    assert!(
        marker
            .windows(8)
            .any(|bytes| bytes == 0x7ff0_0000_0000_0002_u64.to_le_bytes())
    );
    fs::remove_file(config).unwrap();
}

#[test]
fn microservice_events_use_configured_queries_calls_databases_and_containers() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-service-details-{}.yaml",
        std::process::id()
    ));
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 400\n    start_time: '2024-01-01'\n  log_types:\n    user_service:\n      enabled: true\n      service_name: user-service\n      custom_fields:\n        events: ['user event']\n        db_operations: ['SELECT * FROM users']\n        external_calls: ['Calling inventory-service for {product_id}']\n        gateway_calls: ['Calling Stripe API']\n    database_logs:\n      enabled: true\n      databases:\n        - {name: user-db, type: postgresql}\n        - {name: product-db, type: mongodb}\n      custom_fields:\n        events: ['Query on {db_name}']\n    container_logs:\n      enabled: true\n      containers: ['user-service-{instance}', 'redis-{instance}']\n      custom_fields:\n        events: ['Container {container_name} started']\n").unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "17",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(serial.stdout, parallel.stdout);
    let output = String::from_utf8(serial.stdout).unwrap();
    for expected in [
        "SELECT * FROM users",
        "Calling inventory-service",
        "Calling Stripe API",
        "Query on user-db",
        "Query on product-db",
        "Container user-service-",
        "Container redis-",
    ] {
        assert!(output.contains(expected), "missing {expected}");
    }
    assert!(!output.contains("{instance}"));
    assert!(!output.contains("{product_id}"));
    fs::remove_file(path).unwrap();
}

#[test]
fn gateway_uses_configured_routing_status_and_latency() {
    let path = std::env::temp_dir().join(format!("log-forge-gateway-{}.yaml", std::process::id()));
    let config = "log_generator:\n  global:\n    output_format: console\n    total_logs: 60\n    start_time: '2024-01-01'\n  log_types:\n    api_gateway:\n      enabled: true\n      service_name: api-gateway\n      endpoints: ['/api/users/*']\n      custom_fields:\n        upstream_services: [user-service]\n        status_codes: {201: 1}\n        response_times:\n          fast: {min: 42, max: 42, weight: 1}\n";
    fs::write(&path, config).unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "19",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(serial.stdout, parallel.stdout);
    let output = String::from_utf8(serial.stdout).unwrap();
    assert_eq!(output.lines().count(), 60);
    for line in output.lines() {
        assert!(line.contains("service=api-gateway"));
        assert!(line.contains("endpoint=/api/users/"));
        assert!(line.contains("upstream=user-service status=201 response_time_ms=42"));
        assert!(!line.contains('*'));
    }
    fs::write(
        &path,
        config.replace("min: 42, max: 42", "min: 43, max: 42"),
    )
    .unwrap();
    let invalid = run("1");
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("invalid gateway settings"));
    fs::remove_file(path).unwrap();
}

#[test]
fn configured_error_library_generates_weighted_patterns_and_rejects_invalid_entries() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-error-library-{}.yaml",
        std::process::id()
    ));
    let config = "log_generator:\n  global:\n    output_format: console\n    total_logs: 40\n    start_time: '2024-01-01'\n  log_types:\n    error_pattern: {enabled: true, frequency: 1}\n  error_patterns:\n    database_errors:\n      - {pattern: 'Database {db_name} failed', frequency: 1, severity: critical}\n      - {pattern: 'Never selected', frequency: 0, severity: warning}\n";
    fs::write(&path, config).unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "9",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(serial.stdout, parallel.stdout);
    let output = String::from_utf8(serial.stdout).unwrap();
    assert_eq!(output.lines().count(), 40);
    let validator = log_forge::validation::LogValidator::new("error_pattern").unwrap();
    assert!(output.lines().all(|line| validator.validate(line).is_valid));
    assert!(output.lines().all(|line| line.contains("[CRITICAL] category=database_errors Database app-db failed")));
    fs::write(&path, config.replace("{db_name}", "{unknown_field}")).unwrap();
    let invalid = run("1");
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unknown built-in pattern field"));
    fs::remove_file(path).unwrap();
}

#[test]
fn performance_memory_limit_checks_usage_and_rejects_invalid_sizes() {
    let root = std::env::temp_dir().join(format!("log-forge-memory-limit-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("config.yaml");
    let output = root.join("output.log");
    let config = format!(
        "log_generator:\n  global:\n    output_format: file\n    output_path: '{}'\n    total_logs: 3\n    start_time: '2024-01-01'\n  performance:\n    memory_limit: 1GB\n  log_types:\n    redis: {{enabled: true}}\n",
        output.display()
    );
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--config", path.to_str().unwrap(), "--no-monitor"])
            .output()
            .unwrap()
    };
    fs::write(&path, config.replace("1GB", "1B")).unwrap();
    let low = run();
    assert!(!low.status.success());
    assert!(String::from_utf8_lossy(&low.stderr).contains("memory limit exceeded"));
    assert!(!output.exists());
    fs::write(&path, config.replace("1GB", "0B")).unwrap();
    let zero = run();
    assert!(!zero.status.success());
    assert!(
        String::from_utf8_lossy(&zero.stderr).contains("memory_limit must be greater than zero")
    );
    fs::write(&path, &config).unwrap();
    let valid = run();
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    assert_eq!(fs::read_to_string(&output).unwrap().lines().count(), 3);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn configured_monitor_serves_metrics_and_health_without_console_statistics() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};
    let metrics_port = 0;
    let health_port = 0;
    let path = std::env::temp_dir().join(format!(
        "log-forge-monitor-http-{}.yaml",
        std::process::id()
    ));
    fs::write(&path, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 10\n    generation_interval: 0.2\n    virtual_time: false\n  log_types:\n    redis: {{enabled: true}}\n  monitoring:\n    enable_statistics: false\n    statistics_interval: 0.1\n    metrics:\n      enabled: true\n      format: prometheus\n      endpoint: /metrics\n      port: {metrics_port}\n      labels: {{environment: test}}\n    health_check:\n      enabled: true\n      endpoint: /health\n      port: {health_port}\n")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", path.to_str().unwrap(), "--no-monitor"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr_reader = BufReader::new(child.stderr.take().unwrap());
    let mut ready = String::new();
    stderr_reader.read_line(&mut ready).unwrap();
    let metrics_port = monitor_ready_port(&ready, "metrics");
    let health_port = monitor_ready_port(&ready, "health");
    assert_ne!(metrics_port, health_port);
    let get = |port, endpoint: &str| {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("monitor connection failed: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        if endpoint == "/metrics" {
            std::thread::sleep(Duration::from_millis(100));
        }
        write!(stream, "GET {endpoint} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    };
    let metrics = get(metrics_port, "/metrics");
    assert!(metrics.starts_with("HTTP/1.1 200 OK"));
    assert!(metrics.contains("log_forge_records_total{log_type=\"redis\",environment=\"test\"}"));
    assert!(get(health_port, "/health").ends_with("ok\n"));
    assert!(get(health_port, "/unknown").starts_with("HTTP/1.1 404"));
    let status = child.wait().unwrap();
    let mut stderr = String::new();
    stderr_reader.read_to_string(&mut stderr).unwrap();
    assert!(status.success(), "{stderr}");
    assert!(!stderr.contains("generated="));
    fs::remove_file(path).unwrap();
}

#[test]
fn gateway_service_metrics_report_generated_requests_and_reject_unsupported_metrics() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};
    let port = 0;
    let path = std::env::temp_dir().join(format!(
        "log-forge-service-metrics-{}.yaml",
        std::process::id()
    ));
    let config = format!(
        "log_generator:\n  global:\n    output_format: console\n    total_logs: 10\n    generation_interval: 0.2\n    virtual_time: false\n  log_types:\n    api_gateway:\n      enabled: true\n      service_name: gateway\n      endpoints: ['/users/*']\n      custom_fields:\n        upstream_services: [users]\n        status_codes: {{503: 1}}\n        response_times:\n          slow: {{min: 42, max: 42, weight: 1}}\n  monitoring:\n    enable_statistics: false\n    metrics: {{enabled: true, port: {port}}}\n    service_metrics:\n      - service: gateway\n        metrics: [request_count, response_time, error_rate]\n"
    );
    fs::write(&path, &config).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", path.to_str().unwrap(), "--no-monitor"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr_reader = BufReader::new(child.stderr.take().unwrap());
    let mut ready = String::new();
    stderr_reader.read_line(&mut ready).unwrap();
    let port = monitor_ready_port(&ready, "metrics");
    let deadline = Instant::now() + Duration::from_secs(5);
    let response = loop {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(stream, "GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            if response.contains("log_forge_service_response_time_ms{service=\"gateway\"} 42") {
                break response;
            }
        }
        assert!(
            Instant::now() < deadline,
            "metrics did not include a request"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(response.contains("log_forge_service_requests_total{service=\"gateway\"} "));
    assert!(response.contains("log_forge_service_error_ratio{service=\"gateway\"} 1"));
    assert!(child.wait().unwrap().success());
    fs::write(
        &path,
        config.replace("request_count, response_time, error_rate", "unknown_metric"),
    )
    .unwrap();
    let invalid = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", path.to_str().unwrap(), "--no-monitor"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(
        String::from_utf8_lossy(&invalid.stderr)
            .contains("unsupported service metric: unknown_metric")
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn user_service_metrics_are_exposed_from_generated_events() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};
    let port = 0;
    let path = std::env::temp_dir().join(format!(
        "log-forge-user-metrics-{}.yaml",
        std::process::id()
    ));
    fs::write(&path, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 10\n    generation_interval: 0.2\n    virtual_time: false\n  log_types:\n    user_service:\n      enabled: true\n      service_name: user-service\n      custom_fields:\n        events: ['User {{user_id}} registered successfully']\n  monitoring:\n    enable_statistics: false\n    metrics: {{enabled: true, port: {port}}}\n    service_metrics:\n      - service: user-service\n        metrics: [active_users, registration_rate, auth_failures]\n")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", path.to_str().unwrap(), "--no-monitor"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr_reader = BufReader::new(child.stderr.take().unwrap());
    let mut ready = String::new();
    stderr_reader.read_line(&mut ready).unwrap();
    let port = monitor_ready_port(&ready, "metrics");
    let deadline = Instant::now() + Duration::from_secs(5);
    let response = loop {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(stream, "GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            if response.contains("log_forge_service_active_users{service=\"user-service\"} 1") {
                break response;
            }
        }
        assert!(Instant::now() < deadline, "no active user appeared");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        response.contains("log_forge_service_registrations_per_second{service=\"user-service\"}")
    );
    assert!(response.contains("log_forge_service_auth_failures_total{service=\"user-service\"} 0"));
    let status = child.wait().unwrap();
    let mut errors = String::new();
    stderr_reader.read_to_string(&mut errors).unwrap();
    assert!(status.success(), "{errors}");
    fs::remove_file(path).unwrap();
}

#[test]
fn timed_rotation_uses_backup_count_and_size_rotation_uses_max_files() {
    let root =
        std::env::temp_dir().join(format!("log-forge-rotation-counts-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("timed.log");
    let config = root.join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: file\n    output_path: '{}'\n    total_logs: 3\n    generation_interval: 1.1\n    virtual_time: false\n    log_rotation:\n      enabled: true\n      rotation_time: 1\n      max_files: 3\n      backup_count: 1\n  log_types:\n    redis: {{enabled: true}}\n", path.display())).unwrap();
    let run = |file: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--config", file.to_str().unwrap(), "--no-monitor"])
            .output()
            .unwrap()
    };
    let timed = run(&config);
    assert!(!timed.status.success());
    assert!(String::from_utf8_lossy(&timed.stderr).contains("rotation limit reached"));
    assert!(root.join("timed.log.1").exists());
    assert!(!root.join("timed.log.2").exists());
    assert!(!fs::read(&path).unwrap().is_empty());
    let size_path = root.join("size.log");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: file\n    output_path: '{}'\n    total_logs: 3\n    log_rotation:\n      enabled: true\n      max_size: 1\n      rotation_time: 3600\n      max_files: 3\n      backup_count: 1\n  log_types:\n    redis: {{enabled: true}}\n", size_path.display())).unwrap();
    let size = run(&config);
    assert!(
        size.status.success(),
        "{}",
        String::from_utf8_lossy(&size.stderr)
    );
    assert!(root.join("size.log.1").exists());
    assert!(root.join("size.log.2").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn apache_virtual_host_is_emitted_and_ip_is_parsed_after_host() {
    let root = std::env::temp_dir().join(format!("log-forge-apache-vhost-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.yaml");
    fs::write(&config, "log_generator:\n  global:\n    output_format: console\n    total_logs: 5\n    start_time: '2024-01-01'\n  log_types:\n    apache_access:\n      enabled: true\n      virtual_hosts: ['192.0.2.5']\n      custom_fields:\n        ip_ranges: ['10.0.0.1/32']\n        status_codes: {201: 1}\n").unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--no-monitor",
            ])
            .args(args)
            .output()
            .unwrap()
    };
    let raw = run(&[]);
    assert!(
        raw.status.success(),
        "{}",
        String::from_utf8_lossy(&raw.stderr)
    );
    let output = String::from_utf8(raw.stdout).unwrap();
    let mut analyzer = log_forge::analysis::LogAnalyzer::new("apache_access").unwrap();
    for line in output.lines() {
        assert!(line.starts_with("192.0.2.5 10.0.0.1 "), "{line}");
        assert!(
            log_forge::validation::LogValidator::new("apache_access")
                .unwrap()
                .validate(line)
                .is_valid
        );
        analyzer.observe(line);
    }
    let stats = analyzer.statistics();
    assert_eq!(stats.valid_logs, 5);
    assert_eq!(stats.ip_addresses.get("10.0.0.1"), Some(&5));
    let json_path = root.join("out.json");
    let json = run(&["--output", "json", "--path", json_path.to_str().unwrap()]);
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    for line in fs::read_to_string(json_path).unwrap().lines() {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(value["source"]["ip"], "10.0.0.1");
    }
    fs::write(
        &config,
        "log_generator:\n  log_types:\n    apache_access:\n      virtual_hosts: ['bad host']\n",
    )
    .unwrap();
    assert!(!run(&[]).status.success());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn syslog_facility_patterns_render_original_template_fields() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-syslog-custom-patterns-{}.yaml",
        std::process::id()
    ));
    let config = "log_generator:\n  global:\n    output_format: console\n    total_logs: 90\n    start_time: '2024-01-01'\n  log_types:\n    syslog:\n      enabled: true\n      facilities: [kern, auth, cron]\n      severities: [info]\n      rfc_format: '5424'\n      message_patterns:\n        kern: ['Killed {process_name} pid={pid} address={address} sp={sp} error={error}']\n        auth: ['Failed password for {username} from {ip} port {port}']\n        cron: ['({username}) CMD ({command}) file={crontab}']\n";
    fs::write(&path, config).unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "91",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(serial.stdout, parallel.stdout);
    let logs = String::from_utf8(serial.stdout).unwrap();
    for expected in [
        " - Killed ",
        " - Failed password for user",
        " - (user",
        ") CMD (",
        ") file=",
    ] {
        assert!(logs.contains(expected), "missing {expected}");
    }
    assert!(!logs.contains('{'));
    let validator = log_forge::validation::LogValidator::new("syslog").unwrap();
    assert!(logs.lines().all(|line| validator.validate(line).is_valid));
    fs::write(&path, config.replace("{process_name}", "{unknown_field}")).unwrap();
    let invalid = run("1");
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unknown built-in syslog field"));
    fs::remove_file(path).unwrap();
}

#[test]
fn fastapi_configures_response_distribution_and_identifiers() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-fastapi-distribution-{}.yaml",
        std::process::id()
    ));
    let config = "log_generator:\n  global:\n    output_format: console\n    total_logs: 120\n    start_time: '2024-01-01'\n  log_types:\n    fastapi:\n      enabled: true\n      log_levels: {INFO: 1}\n      endpoints: ['/users/{user_id}']\n      response_time_distribution:\n        fast: {min: 17, max: 17, weight: 1}\n        slow: {min: 999, max: 999, weight: 0}\n      include_request_id: false\n      include_user_id: true\n      include_correlation_id: true\n      custom_fields:\n        status_codes: {503: 1}\n";
    fs::write(&path, config).unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "8",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(serial.stdout, parallel.stdout);
    let logs = String::from_utf8(serial.stdout).unwrap();
    let validator = log_forge::validation::LogValidator::new("fastapi").unwrap();
    let fields = regex::Regex::new(
        r"/users/(\d+) - 503 - 17ms user_id=(\d+) correlation_id=([0-9a-f]{32})$",
    )
    .unwrap();
    let requests: Vec<_> = logs
        .lines()
        .filter(|line| line.contains("Request completed:"))
        .collect();
    assert!(requests.len() > 30);
    assert!(logs.lines().all(|line| validator.validate(line).is_valid));
    for line in requests {
        let captures = fields.captures(line).unwrap_or_else(|| panic!("{line}"));
        assert_eq!(&captures[1], &captures[2]);
        assert!(line.contains("[INFO] Request completed:"));
    }
    fs::write(
        &path,
        config.replace("min: 17, max: 17", "min: 18, max: 17"),
    )
    .unwrap();
    let invalid = run("1");
    assert!(!invalid.status.success());
    assert!(
        String::from_utf8_lossy(&invalid.stderr)
            .contains("invalid response_time_distribution range")
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn django_components_generate_configured_sql_and_security_events() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-django-components-{}.yaml",
        std::process::id()
    ));
    let config = "log_generator:\n  global:\n    output_format: console\n    total_logs: 120\n    start_time: '2024-01-01'\n  log_types:\n    django:\n      enabled: true\n      log_levels: {INFO: 1}\n      components: [django.request, django.db.backends, django.security]\n      include_sql_queries: true\n      sql_query_patterns: ['SELECT * FROM accounts WHERE id = %s']\n";
    fs::write(&path, config).unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(serial.stdout, parallel.stdout);
    let output = String::from_utf8(serial.stdout).unwrap();
    assert!(output.contains("django.request:"));
    assert!(output.contains("django.db.backends:"));
    assert!(output.contains("SELECT * FROM accounts WHERE id = %s"));
    assert!(output.contains("django.security:"));
    let validator = log_forge::validation::LogValidator::new("django").unwrap();
    assert!(output.lines().all(|line| validator.validate(line).is_valid));
    fs::write(
        &path,
        config.replace("include_sql_queries: true", "include_sql_queries: false"),
    )
    .unwrap();
    let invalid = run("1");
    assert!(!invalid.status.success());
    assert!(
        String::from_utf8_lossy(&invalid.stderr)
            .contains("sql_query_patterns requires include_sql_queries")
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn docker_drivers_apply_container_patterns_and_stable_identity() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-docker-driver-{}.yaml",
        std::process::id()
    ));
    for driver in ["json-file", "syslog", "journald"] {
        fs::write(&path, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 30\n    start_time: '2024-01-01'\n  log_types:\n    docker:\n      enabled: true\n      container_names: ['web-app-{{instance_id}}']\n      log_drivers: [{driver}]\n      container_patterns:\n        web-app: ['Processing request: {{method}} {{path}}']\n")).unwrap();
        let run = |threads: &str| {
            Command::new(env!("CARGO_BIN_EXE_log-forge"))
                .args([
                    "start",
                    "--config",
                    path.to_str().unwrap(),
                    "--threads",
                    threads,
                    "--seed",
                    "11",
                    "--no-monitor",
                ])
                .output()
                .unwrap()
        };
        let serial = run("1");
        assert!(
            serial.status.success(),
            "{driver}: {}",
            String::from_utf8_lossy(&serial.stderr)
        );
        let parallel = run("4");
        assert!(parallel.status.success());
        assert_eq!(serial.stdout, parallel.stdout);
        let logs = String::from_utf8(serial.stdout).unwrap();
        let validator = log_forge::validation::LogValidator::new("docker").unwrap();
        let mut names = std::collections::BTreeSet::new();
        for line in logs.lines() {
            assert!(validator.validate(line).is_valid, "{driver}: {line}");
            assert!(line.contains("Processing request:"));
            assert!(!line.contains("{instance_id}"));
            match driver {
                "json-file" => {
                    let record: serde_json::Value = serde_json::from_str(line).unwrap();
                    assert!(
                        record["log"]
                            .as_str()
                            .unwrap()
                            .contains("Processing request:")
                    );
                }
                "journald" => {
                    let record: serde_json::Value = serde_json::from_str(line).unwrap();
                    names.insert(record["CONTAINER_NAME"].as_str().unwrap().to_owned());
                    assert_eq!(record["CONTAINER_ID_FULL"].as_str().unwrap().len(), 64);
                }
                _ => {
                    let name = line.split_whitespace().nth(3).unwrap();
                    names.insert(name.to_owned());
                }
            }
        }
        if driver != "json-file" {
            assert_eq!(names.len(), 1);
            assert!(names.iter().next().unwrap().starts_with("web-app-"));
        }
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn docker_container_id_and_name_can_be_set_in_custom_fields() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    fs::write(&config, "log_generator:\n  global:\n    output_format: console\n    total_logs: 2\n    start_time: '2024-01-01'\n  log_types:\n    docker:\n      enabled: true\n      log_drivers: [journald]\n      custom_fields:\n        container_id: short\n        container_name: checkout\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config"])
        .arg(&config)
        .arg("--no-monitor")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records = String::from_utf8(output.stdout).unwrap();
    assert_eq!(records.lines().count(), 2);
    for line in records.lines() {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(value["CONTAINER_ID"], "short");
        assert_eq!(value["CONTAINER_ID_FULL"], "short");
        assert_eq!(value["CONTAINER_NAME"], "checkout");
    }
}

#[test]
fn kubernetes_example_patterns_generate_consistent_event_identity() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-kubernetes-patterns-{}.yaml",
        std::process::id()
    ));
    let config = "log_generator:\n  global:\n    output_format: console\n    total_logs: 100\n    start_time: '2024-01-01'\n  log_types:\n    kubernetes:\n      enabled: true\n      namespaces: [monitoring]\n      pod_patterns: ['web-{deployment}-{replica_id}']\n      event_patterns:\n        - 'Pod {pod_name} scheduled to node {node_name}'\n        - 'Liveness probe failed for container {container_name}'\n";
    fs::write(&path, config).unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "31",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(serial.stdout, parallel.stdout);
    let output = String::from_utf8(serial.stdout).unwrap();
    assert!(output.contains("Normal Scheduled Pod/monitoring/web-app-"));
    assert!(output.contains("Warning Unhealthy Pod/monitoring/web-app-"));
    let validator = log_forge::validation::LogValidator::new("kubernetes").unwrap();
    for line in output.lines() {
        assert!(validator.validate(line).is_valid, "{line}");
        if line.contains(" scheduled to node ") {
            let pod = line
                .split("Pod/monitoring/")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap();
            assert!(line.contains(&format!("Pod {pod} scheduled to node")));
        }
    }
    fs::write(&path, config.replace("{node_name}", "{unknown}")).unwrap();
    assert!(!run("1").status.success());
    fs::remove_file(path).unwrap();
}

#[test]
fn handlers_support_human_console_and_syslog_network() {
    use std::net::UdpSocket;
    use std::time::Duration;
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let port = socket.local_addr().unwrap().port();
    let root =
        std::env::temp_dir().join(format!("log-forge-readable-syslog-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.yaml");
    let file = root.join("raw.log");
    fs::write(&config, format!("log_generator:\n  global:\n    total_logs: 1\n    start_time: '2024-01-01'\n  log_types:\n    redis: {{enabled: true}}\n  output:\n    handlers:\n      - type: file\n        file_path: '{}'\n      - type: network\n        protocol: udp\n        host: 127.0.0.1\n        port: {port}\n        format: syslog\n      - type: console\n        colored: true\n        format: human_readable\n", file.display())).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let console = String::from_utf8(output.stdout).unwrap();
    assert!(console.starts_with("[2024-01-01 00:00:00] [REDIS] "));
    assert!(!console.contains('\u{1b}'));
    let raw = fs::read_to_string(file).unwrap();
    assert!(console.contains(raw.trim_end()));
    let mut packet = [0; 4096];
    let (length, _) = socket.recv_from(&mut packet).unwrap();
    let wire = std::str::from_utf8(&packet[..length]).unwrap();
    assert!(wire.starts_with("<14>1 2024-01-01T00:00:00+00:00 - redis - - - "));
    assert!(wire.ends_with(&raw));
    fs::write(&config, format!("log_generator:\n  global: {{total_logs: 1}}\n  log_types:\n    redis: {{enabled: true}}\n  output:\n    handlers:\n      - type: file\n        file_path: '{}'\n        colored: true\n", root.join("invalid.log").display())).unwrap();
    let invalid = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(!root.join("invalid.log").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn advanced_performance_settings_apply_precedence_and_warn_for_python_gc() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-performance-extra-{}.yaml",
        std::process::id()
    ));
    let config = "log_generator:\n  global:\n    output_format: console\n    total_logs: 8\n    start_time: '2024-01-01'\n  log_types:\n    redis: {enabled: true}\n  performance:\n    threads: 4\n    thread_pool_size: 2\n    buffer_size: 10000\n    io_buffer_size: 65536\n    gc_threshold: 1000\n    lazy_loading: false\n";
    fs::write(&path, config).unwrap();
    let show = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["config", "show", "--file", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        show.status.success(),
        "{}",
        String::from_utf8_lossy(&show.stderr)
    );
    assert!(String::from_utf8_lossy(&show.stderr).contains("gc_threshold is not applied"));
    let shown: serde_yaml::Value = serde_yaml::from_slice(&show.stdout).unwrap();
    assert_eq!(
        shown["log_generator"]["global"]["buffer_size"].as_u64(),
        Some(65536)
    );
    assert_eq!(
        shown["log_generator"]["global"]["thread_pool_size"].as_u64(),
        Some(2)
    );
    assert_eq!(
        shown["log_generator"]["global"]["lazy_loading"].as_bool(),
        Some(false)
    );
    let run = |threads| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "7",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let single = run("1");
    let multi = run("4");
    assert!(single.status.success());
    assert!(
        multi.status.success(),
        "{}",
        String::from_utf8_lossy(&multi.stderr)
    );
    assert_eq!(single.stdout, multi.stdout);
    fs::write(
        &path,
        config.replace("thread_pool_size: 2", "thread_pool_size: 0"),
    )
    .unwrap();
    let invalid = run("1");
    assert!(!invalid.status.success());
    assert!(
        String::from_utf8_lossy(&invalid.stderr)
            .contains("thread_pool_size and io_buffer_size must be greater than zero")
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn custom_template_applies_integer_correlation() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    fs::write(&config, "log_generator:\n  global: {output_format: console, total_logs: 4, start_time: '2024-01-01'}\n  log_types:\n    custom:\n      template: '{rows} {duration}'\n      frequency_maps:\n        rows: {'1001': 1}\n        duration: {normal: 1}\n      correlation_rules:\n        - {condition_field: rows, condition_value: '1000', condition_operator: greater_than, target_field: duration, target_distribution: {slow: 1}}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config"])
        .arg(config)
        .args(["--no-monitor"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "1001 slow\n".repeat(4)
    );
}

#[test]
fn field_frequency_validates_and_applies_integer_correlation() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let output = root.path().join("out.log");
    let source = format!(
        "log_generator:\n  global:\n    output_format: file\n    output_path: '{}'\n    total_logs: 3\n    start_time: '2024-01-01'\n  log_types:\n    custom:\n      template: '{{rows}} {{duration}}'\n      field_frequency:\n        distributions:\n          rows: {{'1001': 1}}\n          duration: {{normal: 1}}\n        correlation_rules:\n          - {{condition_field: rows, condition_value: '1000', condition_operator: greater_than, target_field: duration, target_distribution: {{slow: 1}}}}\n",
        output.display()
    );
    fs::write(&config, &source).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--config"])
            .arg(&config)
            .arg("--no-monitor")
            .output()
            .unwrap()
    };
    let result = run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        fs::read_to_string(&output).unwrap(),
        "1001 slow\n".repeat(3)
    );
    fs::remove_file(&output).unwrap();
    fs::write(
        &config,
        source.replace("condition_value: '1000'", "condition_value: invalid"),
    )
    .unwrap();
    let result = run();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("invalid integer correlation threshold")
    );
    assert!(!output.exists());
}

#[test]
fn original_advanced_config_validates_and_generates_with_console_override() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/advanced_config.yaml");
    let temp = tempfile::tempdir().unwrap();
    let mut config: serde_yaml::Value = serde_yaml::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["log_generator"]["logging"]["file"] = serde_yaml::Value::String(
        temp.path()
            .join("generator.log")
            .to_string_lossy()
            .into_owned(),
    );
    let path = temp.path().join("advanced.yaml");
    fs::write(&path, serde_yaml::to_string(&config).unwrap()).unwrap();
    let valid = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["config", "validate", "--file", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    let generated = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--output",
            "console",
            "--total",
            "50",
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    assert!(!generated.stdout.is_empty());
    let warnings = String::from_utf8_lossy(&generated.stderr);
    assert!(warnings.contains("gc_threshold is not applied"));
    assert!(warnings.contains("output handlers are overridden"));
}

#[test]
fn fixed_interval_overrides_config_and_conflicts_with_random_flag() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-fixed-interval-{}.yaml",
        std::process::id()
    ));
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 3\n    generation_interval: 10\n    random_interval: true\n    start_time: '2024-01-01'\n  log_types:\n    redis: {enabled: true}\n").unwrap();
    let run = |flags: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--config", path.to_str().unwrap(), "--no-monitor"])
            .args(flags)
            .output()
            .unwrap()
    };
    let fixed = run(&["--fixed-interval"]);
    assert!(
        fixed.status.success(),
        "{}",
        String::from_utf8_lossy(&fixed.stderr)
    );
    let logs = String::from_utf8(fixed.stdout).unwrap();
    let lines: Vec<_> = logs.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("00:00:00.000"));
    assert!(lines[1].contains("00:00:10.000"));
    assert!(lines[2].contains("00:00:20.000"));
    assert!(
        !run(&["--fixed-interval", "--random-interval"])
            .status
            .success()
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn sample_aliases_use_configured_django_and_kubernetes_settings() {
    let path = std::env::temp_dir().join(format!(
        "log-forge-sample-alias-{}.yaml",
        std::process::id()
    ));
    fs::write(&path, "log_generator:\n  global:\n    start_time: '2024-01-01'\n  log_types:\n    django:\n      enabled: true\n      components: [django.security]\n      log_levels: {WARNING: 1}\n    kubernetes:\n      enabled: true\n      namespaces: [sample-ns]\n      pod_patterns: ['api-{deployment}-{replica_id}']\n      event_patterns: ['Pod {pod_name} scheduled to node {node_name}']\n").unwrap();
    for (kind, expected) in [
        ("django", "WARNING django.security:"),
        ("kubernetes", "Pod/sample-ns/api-app-"),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "sample",
                "--type",
                kind,
                "--count",
                "3",
                "--config",
                path.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let logs = String::from_utf8(result.stdout).unwrap();
        assert_eq!(logs.lines().count(), 3);
        assert!(logs.lines().all(|line| line.contains(expected)), "{logs}");
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn internal_logging_respects_format_level_and_appends() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let logfile = root.path().join("logs/internal.log");
    let source = format!(
        "log_generator:\n  global:\n    output_format: console\n    total_logs: 2\n  log_types:\n    redis: {{enabled: true}}\n  logging:\n    level: DEBUG\n    format: '%(name)s %(levelname)s %(funcName)s:%(lineno)d %(message)s %%'\n    file: '{}'\n    loggers:\n      log_generator.core: INFO\n",
        logfile.display()
    );
    fs::write(&config, &source).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let content = fs::read_to_string(&logfile).unwrap();
    assert_eq!(content.lines().count(), 2);
    assert!(content.contains("log_generator.core INFO run:"));
    assert!(content.contains("generation started %"));
    assert!(content.contains("generation finished: 2 records %"));
    assert!(run().status.success());
    assert_eq!(fs::read_to_string(&logfile).unwrap().lines().count(), 4);
    fs::write(
        &config,
        source.replace("log_generator.core: INFO", "log_generator.core: WARNING"),
    )
    .unwrap();
    assert!(run().status.success());
    assert_eq!(fs::read_to_string(&logfile).unwrap().lines().count(), 4);
    fs::write(&config, source.replace("%(message)s", "%(unknown)s")).unwrap();
    let invalid = run();
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unsupported logging format field"));
}

#[test]
fn nginx_defaults_use_weighted_methods_statuses_and_varied_fields() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("empty.yaml");
    fs::write(&config, "log_generator: {}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--output",
            "console",
            "--types",
            "nginx_access",
            "--total",
            "5000",
            "--start-time",
            "2024-01-01",
            "--seed",
            "77",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    let regex = regex::Regex::new(r#"^(\S+) - - \[[^\]]+\] \"(\w+) ([^ ]+) HTTP/1.1\" (\d+) (\d+) \"([^\"]+)\" \"([^\"]+)\"$"#).unwrap();
    let mut private = 0;
    let mut gets = 0;
    let mut success = 0;
    let mut dynamic = 0;
    let mut agents = std::collections::HashSet::new();
    let mut referers = std::collections::HashSet::new();
    for line in output.lines() {
        let captures = regex.captures(line).unwrap_or_else(|| panic!("{line}"));
        let ip: std::net::Ipv4Addr = captures[1].parse().unwrap();
        private += usize::from(ip.is_private());
        gets += usize::from(&captures[2] == "GET");
        let status: u16 = captures[4].parse().unwrap();
        let size: u64 = captures[5].parse().unwrap();
        success += usize::from(status == 200);
        match status {
            404 => assert!((150..=500).contains(&size)),
            500.. => assert!((100..=300).contains(&size)),
            301 | 302 => assert!((200..=400).contains(&size)),
            _ => assert!((500..=50000).contains(&size)),
        }
        dynamic += usize::from(
            captures[3].starts_with("/api/users/")
                || captures[3].starts_with("/products/")
                || captures[3].starts_with("/search?q=")
                || captures[3].starts_with("/category/")
                || captures[3].starts_with("/page/"),
        );
        referers.insert(captures[6].to_owned());
        agents.insert(captures[7].to_owned());
    }
    assert!((3200..=3800).contains(&private), "{private}");
    assert!((3400..=3900).contains(&gets), "{gets}");
    assert!((2900..=3400).contains(&success), "{success}");
    assert!((800..=1200).contains(&dynamic), "{dynamic}");
    assert!(agents.len() >= 9);
    assert!(referers.len() >= 6);
}

#[test]
fn apache_defaults_use_weighted_methods_statuses_and_varied_fields() {
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "apache_access",
            "--total",
            "5000",
            "--start-time",
            "2024-01-01",
            "--seed",
            "71",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    let regex = regex::Regex::new(r#"^(\S+) - (\S+) \[[^\]]+\] \"(\w+) ([^ ]+) HTTP/1.1\" (\d+) (\d+) \"([^\"]+)\" \"([^\"]+)\"$"#).unwrap();
    let (mut private, mut gets, mut success, mut authenticated, mut dynamic) = (0, 0, 0, 0, 0);
    let mut agents = std::collections::HashSet::new();
    let mut referers = std::collections::HashSet::new();
    for line in output.lines() {
        let captures = regex.captures(line).unwrap_or_else(|| panic!("{line}"));
        private += usize::from(
            captures[1]
                .parse::<std::net::Ipv4Addr>()
                .unwrap()
                .is_private(),
        );
        authenticated += usize::from(&captures[2] != "-");
        gets += usize::from(&captures[3] == "GET");
        let status: u16 = captures[5].parse().unwrap();
        let size: u64 = captures[6].parse().unwrap();
        success += usize::from(status == 200);
        match status {
            404 => assert!((200..=600).contains(&size)),
            500.. => assert!((150..=400).contains(&size)),
            301 | 302 => assert!((250..=500).contains(&size)),
            _ => assert!((800..=75000).contains(&size)),
        }
        dynamic += usize::from(
            captures[4].starts_with("/user/")
                || captures[4].starts_with("/product/")
                || captures[4].starts_with("/search.php?")
                || captures[4].starts_with("/category/")
                || captures[4].starts_with("/page.php?")
                || captures[4].starts_with("/wp-content/uploads/")
                    && captures[4] != *"/wp-content/uploads/image.jpg",
        );
        referers.insert(captures[7].to_owned());
        agents.insert(captures[8].to_owned());
    }
    assert!((2700..=3300).contains(&private), "{private}");
    assert!((3500..=4000).contains(&gets), "{gets}");
    assert!((3500..=4000).contains(&success), "{success}");
    assert!((150..=350).contains(&authenticated), "{authenticated}");
    assert!((800..=1200).contains(&dynamic), "{dynamic}");
    assert!(agents.len() >= 9);
    assert!(referers.len() >= 7);
}

#[test]
fn web_error_logs_use_weighted_levels_and_varied_identifiers() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("empty.yaml");
    fs::write(&config, "log_generator: {}\n").unwrap();
    for (kind, min, max) in [("nginx_error", 2150, 2650), ("apache_error", 2000, 2500)] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--output",
                "console",
                "--types",
                kind,
                "--total",
                "5000",
                "--start-time",
                "2024-01-01",
                "--seed",
                "33",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let logs = String::from_utf8(result.stdout).unwrap();
        let validator = log_forge::validation::LogValidator::new(kind).unwrap();
        let mut error_count = 0;
        let mut levels = std::collections::HashSet::new();
        let mut identifiers = std::collections::HashSet::new();
        for line in logs.lines() {
            assert!(validator.validate(line).is_valid, "{line}");
            let level = if kind == "nginx_error" {
                line.split('[').nth(1).unwrap().split(']').next().unwrap()
            } else {
                line.split('[').nth(2).unwrap().split(']').next().unwrap()
            };
            levels.insert(level.to_owned());
            error_count += usize::from(level == "error");
            identifiers.insert(if kind == "nginx_error" {
                line.split(": *").next().unwrap().to_owned()
            } else {
                line.split("[pid ")
                    .nth(1)
                    .unwrap()
                    .split(']')
                    .next()
                    .unwrap()
                    .to_owned()
            });
        }
        assert!((min..=max).contains(&error_count), "{kind}: {error_count}");
        assert!(levels.len() >= 6, "{kind}: {levels:?}");
        assert!(identifiers.len() > 1000);
    }
    let path = std::env::temp_dir().join(format!(
        "log-forge-apache-error-config-{}.yaml",
        std::process::id()
    ));
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 4}\n  log_types:\n    apache_error:\n      enabled: true\n      error_levels: [crit]\n      error_patterns: ['custom failure']\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", path.to_str().unwrap(), "--no-monitor"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .all(|line| line.contains("[crit]") && !line.contains("custom failure"))
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn redis_and_ssh_generate_distinct_valid_event_types() {
    for (kind, expected) in [
        (
            "redis",
            &[
                "Ready to accept connections",
                "Background saving",
                "DB saved on disk",
                "clients connected",
                "asks for synchronization",
            ][..],
        ),
        (
            "ssh_auth",
            &[
                "Failed password",
                "Accepted password",
                "Accepted publickey",
                "Invalid user",
                "Connection closed",
            ][..],
        ),
    ] {
        let run = |threads: &str| {
            Command::new(env!("CARGO_BIN_EXE_log-forge"))
                .args([
                    "start",
                    "--output",
                    "console",
                    "--types",
                    kind,
                    "--total",
                    "200",
                    "--seed",
                    "12",
                    "--start-time",
                    "2024-01-01",
                    "--threads",
                    threads,
                    "--no-monitor",
                ])
                .output()
                .unwrap()
        };
        let serial = run("1");
        assert!(
            serial.status.success(),
            "{}",
            String::from_utf8_lossy(&serial.stderr)
        );
        let parallel = run("4");
        assert!(parallel.status.success());
        assert_eq!(serial.stdout, parallel.stdout);
        let logs = String::from_utf8(serial.stdout).unwrap();
        let validator = log_forge::validation::LogValidator::new(kind).unwrap();
        assert!(logs.lines().all(|line| validator.validate(line).is_valid));
        for event in expected {
            assert!(logs.contains(event), "{kind}: missing {event}");
        }
    }
}

#[test]
fn custom_templates_support_original_synthetic_field_types() {
    let fields = [
        "ip_address",
        "ipv4",
        "ipv6",
        "mac_address",
        "domain_name",
        "url",
        "uri",
        "port",
        "user_agent",
        "browser",
        "platform",
        "http_method",
        "http_status_code",
        "http_status_phrase",
        "content_type",
        "referer",
        "username",
        "email",
        "first_name",
        "last_name",
        "full_name",
        "company",
        "country",
        "city",
        "latitude",
        "longitude",
        "timestamp",
        "iso_timestamp",
        "unix_timestamp",
        "date",
        "time",
        "file_name",
        "file_path",
        "file_extension",
        "mime_type",
        "response_time:min:3,max:3",
        "file_size",
        "bandwidth",
        "cpu_usage",
        "memory_usage",
        "session_id",
        "api_key",
        "jwt_token",
        "log_level",
        "error_code",
        "process_id",
        "thread_id",
        "database_name",
        "table_name",
        "query_duration",
        "rows_affected",
    ];
    let template = fields
        .iter()
        .map(|name| format!("{{{name}}}"))
        .collect::<Vec<_>>()
        .join("|");
    let config = serde_yaml::from_str::<serde_yaml::Value>(&format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 20\n    start_time: '2024-01-01'\n  log_types:\n    custom:\n      enabled: true\n      template: '{}'\n", template)).unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("custom.yaml");
    fs::write(&path, serde_yaml::to_string(&config).unwrap()).unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "71",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(parallel.status.success());
    assert_eq!(serial.stdout, parallel.stdout);
    let logs = String::from_utf8(serial.stdout).unwrap();
    for line in logs.lines() {
        let values: Vec<_> = line.split('|').collect();
        assert_eq!(values.len(), fields.len());
        assert!(values.iter().all(|value| !value.is_empty()));
        assert!(values[0].parse::<std::net::Ipv4Addr>().is_ok());
        assert!(values[1].parse::<std::net::Ipv4Addr>().is_ok());
        assert!(values[2].parse::<std::net::Ipv6Addr>().is_ok());
        assert_eq!(values[35], "3.00");
        assert_eq!(values[40].len(), 36);
        assert_eq!(values[41].len(), 32);
        assert_eq!(values[42].split('.').count(), 3);
        assert!((0.0..=100.0).contains(&values[38].parse::<f64>().unwrap()));
    }
    assert_eq!(logs.lines().count(), 20);
}

#[test]
fn custom_pattern_files_load_original_json_and_select_yaml_patterns() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let original = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("demo_patterns/api_request_detailed.json");
    fs::copy(original, root.path().join("api.json")).unwrap();
    fs::write(&config, "log_generator:\n  global:\n    output_format: console\n    total_logs: 30\n    start_time: '2024-01-01'\n  log_types:\n    custom:\n      enabled: true\n      pattern_file: api.json\n").unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(parallel.status.success());
    assert_eq!(serial.stdout, parallel.stdout);
    let logs = String::from_utf8(serial.stdout).unwrap();
    for line in logs.lines() {
        assert!(line.contains(" [API] "));
        assert!(line.contains(" /api/v1/"));
        assert!(line.contains(" - Status: "));
        assert!(line.contains(" - User: user"));
        assert!(!line.contains('{'));
    }
    fs::write(root.path().join("patterns.yaml"), "patterns:\n  - name: alpha\n    template: 'alpha {v}'\n    fields:\n      v: {type: choice, options: [x]}\n  - name: beta\n    template: 'beta {v}'\n    fields:\n      v: {type: choice, options: [y]}\n").unwrap();
    fs::write(&config, "log_generator:\n  global: {output_format: console, total_logs: 2}\n  log_types:\n    custom:\n      pattern_file: patterns.yaml\n      pattern_name: beta\n").unwrap();
    let selected = run("1");
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert!(
        String::from_utf8(selected.stdout)
            .unwrap()
            .lines()
            .all(|line| line == "beta y")
    );
    let yaml = fs::read_to_string(&config)
        .unwrap()
        .replace("      pattern_name: beta\n", "");
    fs::write(&config, yaml).unwrap();
    let invalid = run("1");
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("pattern_name is required"));
    assert!(invalid.stdout.is_empty());
}

#[test]
fn custom_templates_support_weighted_values_and_correlations() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.yaml");
    let config = "log_generator:\n  global:\n    output_format: console\n    total_logs: 1000\n    start_time: '2024-01-01'\n  log_types:\n    custom:\n      template: '{status} {latency} {status}'\n      frequency_maps:\n        status: {'200': 0.75, '500': 0.25}\n        latency: {normal: 1}\n      correlation_rules:\n        - condition_field: status\n          condition_value: '500'\n          target_field: latency\n          target_distribution: {slow: 1}\n          probability: 1\n";
    fs::write(&path, config).unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "11",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(parallel.status.success());
    assert_eq!(serial.stdout, parallel.stdout);
    let logs = String::from_utf8(serial.stdout).unwrap();
    let mut errors = 0;
    for line in logs.lines() {
        match line {
            "200 normal 200" => {}
            "500 slow 500" => errors += 1,
            _ => panic!("{line}"),
        }
    }
    assert!((200..=300).contains(&errors), "{errors}");
    fs::write(
        &path,
        config.replace("{status} {latency} {status}", "{latency} {status}"),
    )
    .unwrap();
    let invalid = run("1");
    assert!(!invalid.status.success());
    assert!(
        String::from_utf8_lossy(&invalid.stderr)
            .contains("correlation condition must appear before target")
    );
    fs::write(&path, config.replace("probability: 1", "probability: 1.1")).unwrap();
    assert!(!run("1").status.success());
}

#[test]
fn original_historical_and_multifile_examples_create_reproducible_files() {
    let root = tempfile::tempdir().unwrap();
    for example in ["historical_logs_config.yaml", "multi_file_config.yaml"] {
        let config = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("config")
            .join(example);
        let validate = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["config", "validate", "--file", config.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            validate.status.success(),
            "{example}: {}",
            String::from_utf8_lossy(&validate.stderr)
        );
        let mut results = Vec::new();
        for run in 0..2 {
            let output = root.path().join(format!("{example}-{run}"));
            let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
                .args([
                    "start",
                    "--config",
                    config.to_str().unwrap(),
                    "--path",
                    output.to_str().unwrap(),
                    "--total",
                    "60",
                    "--start-time",
                    "2024-12-08 09:00:00",
                    "--seed",
                    "52",
                    "--no-monitor",
                ])
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{example}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let mut files = std::collections::BTreeMap::new();
            for entry in fs::read_dir(output).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name().into_string().unwrap();
                assert!(name.ends_with("_20241208.log"), "{name}");
                let contents = fs::read_to_string(entry.path()).unwrap();
                assert!(!contents.is_empty());
                files.insert(name, contents);
            }
            assert!(files.len() >= 3);
            results.push(files);
        }
        assert_eq!(results[0], results[1], "{example}");
    }
}

#[test]
fn bundled_multi_file_examples_preserve_type_and_date() {
    let root = tempfile::tempdir().unwrap();
    let configs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config");
    for example in [
        "default_config.yaml",
        "high_performance_config.yaml",
        "ultra_performance_config.yaml",
        "historical_logs_config.yaml",
        "multi_file_config.yaml",
    ] {
        let output_dir = root.path().join(example);
        let result = Command::new(env!("CARGO_BIN_EXE_log-generator"))
            .args(["start", "--config"])
            .arg(configs.join(example))
            .args(["--path"])
            .arg(&output_dir)
            .args([
                "--types",
                "nginx_access,syslog",
                "--total",
                "100",
                "--start-time",
                "2024-12-08 09:00:00",
                "--seed",
                "52",
                "--no-monitor",
            ])
            .current_dir(root.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{example}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let mut count = 0;
        for entry in fs::read_dir(&output_dir).unwrap() {
            let entry = entry.unwrap();
            let filename = entry.file_name().to_string_lossy().into_owned();
            let expected_date = if example == "ultra_performance_config.yaml" {
                "_20241208_090000.log"
            } else {
                "_20241208.log"
            };
            assert!(filename.ends_with(expected_date), "{example}: {filename}");
            let kind = if filename.contains("nginx_access") {
                "nginx_access"
            } else if filename.contains("syslog") {
                "syslog"
            } else {
                panic!("unexpected file: {filename}")
            };
            let validator = log_forge::validation::LogValidator::new(kind).unwrap();
            let records = fs::read_to_string(entry.path()).unwrap();
            for record in records.lines() {
                assert!(validator.validate(record).is_valid, "{example}: {record}");
                count += 1;
            }
        }
        assert_eq!(count, 100, "{example}");
    }
}

#[test]
fn original_multi_file_example_rotates_without_losing_records() {
    let root = tempfile::tempdir().unwrap();
    let output_dir = root.path().join("logs");
    let config =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/multi_file_config.yaml");
    let output = Command::new(env!("CARGO_BIN_EXE_log-generator"))
        .args(["start", "--config"])
        .arg(config)
        .args(["--path"])
        .arg(&output_dir)
        .args([
            "--types",
            "nginx_access",
            "--total",
            "8000",
            "--start-time",
            "2024-12-08 09:00:00",
            "--seed",
            "52",
            "--no-monitor",
        ])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let active = output_dir.join("nginx_access_20241208.log");
    let rotated = output_dir.join("nginx_access_20241208.log.1");
    assert!(active.is_file());
    assert!(rotated.is_file());
    let mut total = 0;
    for entry in fs::read_dir(&output_dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(name.starts_with("nginx_access_20241208.log"), "{name}");
        total += fs::read_to_string(entry.path()).unwrap().lines().count();
    }
    assert_eq!(total, 8000);
}

#[test]
fn original_microservices_example_sends_all_handlers_to_local_receivers() {
    use std::io::Read;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};
    let temp = tempfile::tempdir().unwrap();
    let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
    let tcp_port = tcp.local_addr().unwrap().port();
    let http = TcpListener::bind("127.0.0.1:0").unwrap();
    let http_port = http.local_addr().unwrap().port();
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/microservices_config.yaml");
    let mut config: serde_yaml::Value = serde_yaml::from_slice(&fs::read(source).unwrap()).unwrap();
    let handlers = config["log_generator"]["output"]["handlers"]
        .as_sequence_mut()
        .unwrap();
    handlers[0]["host"] = serde_yaml::Value::String("127.0.0.1".into());
    handlers[0]["port"] = serde_yaml::to_value(tcp_port).unwrap();
    let output_path = temp.path().join("microservices.json");
    handlers[1]["file_path"] =
        serde_yaml::Value::String(output_path.to_string_lossy().into_owned());
    handlers[2]["host"] = serde_yaml::Value::String("127.0.0.1".into());
    handlers[2]["port"] = serde_yaml::to_value(http_port).unwrap();
    config["log_generator"]["monitoring"]["metrics"]["port"] = serde_yaml::to_value(0).unwrap();
    let config_path = temp.path().join("microservices.yaml");
    fs::write(&config_path, serde_yaml::to_string(&config).unwrap()).unwrap();
    let receive = |listener: TcpListener| {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    break stream;
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => panic!("receiver failed: {error}"),
            }
        }
    };
    let tcp_worker = std::thread::spawn(move || {
        let mut stream = receive(tcp);
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let http_worker = std::thread::spawn(move || {
        (0..2)
            .map(|_| receive_remote_write(&http))
            .collect::<Vec<_>>()
    });
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config_path.to_str().unwrap(),
            "--total",
            "50",
            "--start-time",
            "2024-01-01",
            "--seed",
            "8",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let tcp_bytes = tcp_worker.join().unwrap();
    assert_eq!(tcp_bytes, fs::read(output_path).unwrap());
    let lines = String::from_utf8(tcp_bytes).unwrap();
    assert_eq!(lines.lines().count(), 50);
    for line in lines.lines() {
        assert!(serde_json::from_str::<serde_json::Value>(line).is_ok());
    }
    let requests = http_worker.join().unwrap();
    assert_eq!(requests.len(), 2);
    let (headers, body) = &requests[0];
    assert!(headers.contains("Content-Encoding: snappy\r\n"));
    let decoded = snap::raw::Decoder::new().decompress_vec(body).unwrap();
    assert!(
        decoded
            .windows(b"log_forge_records_total".len())
            .any(|window| window == b"log_forge_records_total")
    );
    let marker = snap::raw::Decoder::new()
        .decompress_vec(&requests[1].1)
        .unwrap();
    assert!(
        marker
            .windows(8)
            .any(|bytes| bytes == 0x7ff0_0000_0000_0002_u64.to_le_bytes())
    );
}

#[test]
fn slow_query_logs_have_varied_durations_queries_and_valid_counts() {
    let mysql_regex = regex::Regex::new(
        r"Query_time: ([\d.]+) Lock_time: ([\d.]+) Rows_sent: (\d+) Rows_examined: (\d+)",
    )
    .unwrap();
    let postgres_regex = regex::Regex::new(r"duration: ([\d.]+) ms statement:").unwrap();
    for kind in ["mysql_slow", "postgresql_slow"] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "console",
                "--types",
                kind,
                "--total",
                "400",
                "--start-time",
                "2024-01-01",
                "--seed",
                "31",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let logs = String::from_utf8(result.stdout).unwrap();
        let (duration_regex, query_marker) = if kind == "mysql_slow" {
            (&mysql_regex, "SET timestamp=")
        } else {
            (&postgres_regex, "statement: ")
        };
        let mut durations = Vec::new();
        for capture in duration_regex.captures_iter(&logs) {
            let value: f64 = capture[1].parse().unwrap();
            durations.push(if kind == "mysql_slow" {
                value
            } else {
                value / 1000.0
            });
            if kind == "mysql_slow" {
                assert!(capture[4].parse::<u64>().unwrap() >= capture[3].parse::<u64>().unwrap());
            }
        }
        assert_eq!(durations.len(), 400);
        assert!(durations.iter().all(|value| (1.0..=300.0).contains(value)));
        assert!(durations.iter().any(|value| *value > 60.0));
        assert!(logs.contains("JOIN"));
        assert!(logs.contains("UPDATE"));
        assert!(logs.contains("DELETE"));
        assert!(logs.contains(query_marker));
    }
    let root = tempfile::tempdir().unwrap();
    for kind in ["mysql_slow", "postgresql_slow"] {
        let config = root.path().join(format!("{kind}.yaml"));
        fs::write(&config, format!("log_generator:\n  global: {{output_format: console, total_logs: 10}}\n  log_types:\n    {kind}:\n      table_names: [audit_events]\n      query_types: {{UPDATE: 1}}\n")).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        assert_eq!(output.matches("UPDATE audit_events SET").count(), 10);
    }
}

#[test]
fn service_operations_match_example_events_or_use_explicit_mapping() {
    let root = tempfile::tempdir().unwrap();
    let config_path = root.path().join("services.yaml");
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/microservices_config.yaml");
    let mut config: serde_yaml::Value = serde_yaml::from_slice(&fs::read(source).unwrap()).unwrap();
    let generator = config["log_generator"].as_mapping_mut().unwrap();
    for key in ["output", "monitoring", "distributed_tracing"] {
        generator.remove(serde_yaml::Value::String(key.into()));
    }
    fs::write(&config_path, serde_yaml::to_string(&config).unwrap()).unwrap();
    let run = |name: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config_path.to_str().unwrap(),
                "--output",
                "console",
                "--types",
                name,
                "--total",
                "120",
                "--seed",
                "19",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let cases: &[(&str, &[(&str, &str)])] = &[
        (
            "user_service",
            &[
                ("registered successfully", "user_registration"),
                ("Authentication failed", "user_authentication"),
                ("Profile updated", "profile_update"),
                ("Password changed", "password_change"),
                (" deleted", "user_deletion"),
            ],
        ),
        (
            "order_service",
            &[
                (" created for user", "order_creation"),
                (" updated: status", "order_update"),
                (" cancelled by user", "order_cancellation"),
                ("Inventory check failed", "inventory_check"),
                ("Payment processed", "payment_processing"),
            ],
        ),
        (
            "product_service",
            &[
                ("Product search:", "product_search"),
                ("Inventory updated", "inventory_update"),
                ("Price updated", "price_update"),
                ("Recommendations generated", "product_recommendation"),
            ],
        ),
        (
            "payment_service",
            &[
                ("Payment authorized", "payment_authorization"),
                ("Payment captured", "payment_capture"),
                ("Refund processed", "refund_processing"),
                ("Fraud detected", "fraud_detection"),
            ],
        ),
        (
            "notification_service",
            &[
                ("Email sent", "email_sending"),
                ("SMS sent", "sms_sending"),
                ("Push notification sent", "push_notification"),
                ("Webhook delivered", "webhook_delivery"),
            ],
        ),
        (
            "message_queue",
            &[
                ("Message published", "message_published"),
                ("Message consumed", "message_consumed"),
                ("Queue '", "queue_created"),
                (" connected to queue", "consumer_connected"),
                (" disconnected from queue", "consumer_disconnected"),
            ],
        ),
    ];
    for (service, mappings) in cases {
        let output = run(service);
        assert!(
            output.status.success(),
            "{service}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let logs = String::from_utf8(output.stdout).unwrap();
        for (needle, operation) in *mappings {
            let lines: Vec<_> = logs.lines().filter(|line| line.contains(needle)).collect();
            assert!(!lines.is_empty(), "{service}: {needle}");
            assert!(
                lines
                    .iter()
                    .all(|line| line.contains(&format!("operation={operation} "))),
                "{service}: {needle}: {lines:?}"
            );
        }
    }
    fs::write(&config_path, "log_generator:\n  global: {output_format: console, total_logs: 30}\n  log_types:\n    service:\n      service_name: service\n      operations: [create, delete]\n      custom_fields:\n        events:\n          - {operation: create, message: 'created item'}\n          - {operation: delete, message: 'deleted item'}\n          - 'unmapped event'\n").unwrap();
    let output = run("service");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let logs = String::from_utf8(output.stdout).unwrap();
    for (message, operation) in [
        ("created item", "create"),
        ("deleted item", "delete"),
        ("unmapped event", "event"),
    ] {
        let lines: Vec<_> = logs.lines().filter(|line| line.contains(message)).collect();
        assert!(!lines.is_empty());
        assert!(
            lines
                .iter()
                .all(|line| line.contains(&format!("operation={operation} ")))
        );
    }
    let invalid = fs::read_to_string(&config_path)
        .unwrap()
        .replace("operation: create", "operation: unknown");
    fs::write(&config_path, invalid).unwrap();
    let output = run("service");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("invalid or undeclared event operation")
    );
}

#[test]
fn nested_network_settings_send_udp_and_cli_overrides_port() {
    use std::net::UdpSocket;
    use std::time::Duration;
    let first = UdpSocket::bind("127.0.0.1:0").unwrap();
    let second = UdpSocket::bind("127.0.0.1:0").unwrap();
    first
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    second
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let first_port = first.local_addr().unwrap().port();
    let second_port = second.local_addr().unwrap().port();
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("network.yaml");
    let source = format!(
        "log_generator:\n  global:\n    output_format: network\n    total_logs: 1\n    network_host: ignored.invalid\n    network_port: 1\n    network:\n      host: 127.0.0.1\n      port: {first_port}\n  log_types:\n    redis: {{enabled: true}}\n"
    );
    fs::write(&config, &source).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--no-monitor",
            ])
            .args(args)
            .output()
            .unwrap()
    };
    let output = run(&[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut bytes = [0; 2048];
    let received = first.recv(&mut bytes).unwrap();
    assert!(
        std::str::from_utf8(&bytes[..received])
            .unwrap()
            .contains(":M ")
    );
    let output = run(&["--port", &second_port.to_string()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let received = second.recv(&mut bytes).unwrap();
    assert!(
        std::str::from_utf8(&bytes[..received])
            .unwrap()
            .contains(":M ")
    );
    fs::write(
        &config,
        source.replace(&format!("port: {first_port}"), "port: 0"),
    )
    .unwrap();
    let invalid = run(&[]);
    assert!(!invalid.status.success());
    assert!(
        String::from_utf8_lossy(&invalid.stderr).contains("network host and port are required")
    );
}

#[test]
fn tcp_handler_reconnects_after_server_resets_connection() {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::time::Duration;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let receiver = std::thread::spawn(move || {
        let (first, _) = listener.accept().unwrap();
        first
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(first);
        let mut one = String::new();
        reader.read_line(&mut one).unwrap();
        let first = reader.into_inner();
        socket2::SockRef::from(&first)
            .set_linger(Some(Duration::ZERO))
            .unwrap();
        drop(first);
        let (second, _) = listener.accept().unwrap();
        second
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(second);
        let mut two = String::new();
        reader.read_line(&mut two).unwrap();
        (one, two)
    });
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("tcp.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: network\n    total_logs: 2\n    virtual_time: false\n    generation_interval: 0.3\n    network:\n      host: 127.0.0.1\n      port: {port}\n      protocol: tcp\n      max_retries: 2\n      retry_delay: 0.01\n  log_types:\n    redis: {{enabled: true}}\n")).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("reconnecting"));
    let (one, two) = receiver.join().unwrap();
    let validator = log_forge::validation::LogValidator::new("redis").unwrap();
    assert!(validator.validate(one.trim_end()).is_valid);
    assert!(validator.validate(two.trim_end()).is_valid);
}

#[test]
fn invalid_tcp_retry_settings_fail_before_other_handlers_open() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("unused.log");
    let config = root.path().join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    total_logs: 1\n    network_retry_delay: -1\n  log_types:\n    redis: {{enabled: true}}\n  output:\n    handlers:\n      - type: file\n        file_path: '{}'\n      - type: network\n        host: 127.0.0.1\n        port: 12345\n        protocol: tcp\n", output.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("retry_delay must be finite and nonnegative")
    );
    assert!(!output.exists());
}

#[test]
fn postgresql_generates_level_specific_messages_and_varied_sql() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("postgres.yaml");
    let validator = log_forge::validation::LogValidator::new("postgresql").unwrap();
    for level in ["LOG", "INFO", "WARNING", "ERROR", "FATAL", "PANIC"] {
        fs::write(&config, format!("log_generator:\n  global: {{output_format: console, total_logs: 150, start_time: '2024-01-01'}}\n  log_types:\n    postgresql:\n      log_levels: {{{level}: 1}}\n      table_names: [audit_events]\n")).unwrap();
        let run = |threads: &str| {
            Command::new(env!("CARGO_BIN_EXE_log-forge"))
                .args([
                    "start",
                    "--config",
                    config.to_str().unwrap(),
                    "--seed",
                    "42",
                    "--threads",
                    threads,
                    "--no-monitor",
                ])
                .output()
                .unwrap()
        };
        let serial = run("1");
        assert!(
            serial.status.success(),
            "{}",
            String::from_utf8_lossy(&serial.stderr)
        );
        let parallel = run("4");
        assert!(parallel.status.success());
        assert_eq!(serial.stdout, parallel.stdout);
        let logs = String::from_utf8(serial.stdout).unwrap();
        assert!(
            logs.lines()
                .all(|line| validator.validate(line).is_valid
                    && line.contains(&format!("[{level}]")))
        );
        match level {
            "LOG" => {
                assert!(logs.contains("SELECT"));
                assert!(logs.contains("checkpoint"));
                assert!(logs.contains("automatic vacuum"));
            }
            "INFO" => {
                assert!(logs.contains("autovacuum launcher started"));
                assert!(logs.contains("background writer process started"));
            }
            "WARNING" => {
                assert!(logs.contains("Connection reset by peer"));
                assert!(logs.contains("no transaction in progress"));
            }
            "ERROR" => {
                assert!(logs.contains("duplicate key"));
                assert!(logs.contains("serialization failure"));
            }
            "FATAL" => {
                assert!(logs.contains("authentication failed"));
                assert!(logs.contains("too many connections"));
            }
            _ => {
                assert!(logs.contains("corrupted page"));
                assert!(logs.contains("invalid pages"));
            }
        }
    }
}

#[test]
fn goloop_covers_original_module_patterns_and_sequence_formats() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("goloop.yaml");
    fs::write(&config, "log_generator:\n  global: {output_format: console, total_logs: 1700, start_time: '2024-01-01'}\n  log_types:\n    goloop:\n      format_type: json\n      height_range: [42, 42]\n").unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--threads",
                threads,
                "--seed",
                "41",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(parallel.status.success());
    assert_eq!(serial.stdout, parallel.stdout);
    let data: serde_json::Value =
        serde_json::from_str(include_str!("../data/goloop_patterns.json")).unwrap();
    let mut seen = std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    let mut missing_sequence = false;
    let output = String::from_utf8(serial.stdout).unwrap();
    let validator = log_forge::validation::LogValidator::new("goloop").unwrap();
    for line in output.lines() {
        assert!(validator.validate(line).is_valid, "{line}");
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        let module = record["module"].as_str().unwrap();
        let msg = record["msg"].as_str().unwrap();
        let allowed = data["log_patterns"][module].as_array().unwrap();
        let found = allowed
            .iter()
            .filter_map(|item| item.as_str())
            .find(|pattern| {
                let prefix = pattern
                    .split_once(' ')
                    .filter(|(head, _)| head.contains(".go:"))
                    .map_or(*pattern, |(_, rest)| rest);
                msg.starts_with(prefix)
            })
            .unwrap_or_else(|| panic!("{module}: {msg}"));
        seen.entry(module.into()).or_default().insert(found.into());
        missing_sequence |= record["sequence"] == "-";
        assert!(
            data["sub_modules"][module]
                .as_array()
                .unwrap()
                .contains(&record["sub_module"])
        );
    }
    assert_eq!(seen.len(), 17);
    assert!(seen.values().all(|patterns| patterns.len() >= 2));
    assert!(missing_sequence);
    let text_config = fs::read_to_string(&config)
        .unwrap()
        .replace("format_type: json", "format_type: text");
    fs::write(&config, text_config).unwrap();
    let text = run("1");
    assert!(text.status.success());
    let output = String::from_utf8(text.stdout).unwrap();
    assert!(output.contains("|6df7|-|"));
    assert!(output.lines().all(|line| validator.validate(line).is_valid));
}

#[test]
fn custom_field_frequency_applies_business_hours_and_timed_bursts() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("custom.yaml");
    let source = "log_generator:\n  global:\n    output_format: console\n    total_logs: 5\n    start_time: '2024-01-01 08:59:58'\n    generation_interval: 1\n  log_types:\n    custom:\n      template: '{counter} {status} {status} {outcome}'\n      field_inactive_value: OFF\n      field_frequency:\n        distributions:\n          status: {'200': 1}\n          outcome: {normal: 1}\n        correlation_rules:\n          - {condition_field: status, condition_value: '500', target_field: outcome, target_distribution: {critical: 1}}\n        time_patterns:\n          status: {business_hours_only: true}\n        burst_patterns:\n          status: {burst_probability: 1, burst_duration_seconds: 2, burst_values: ['500']}\n";
    fs::write(&config, source).unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--seed",
                "9",
                "--threads",
                threads,
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(serial.stdout, parallel.stdout);
    assert!(String::from_utf8_lossy(&parallel.stderr).contains("stateful generator runs"));
    assert_eq!(
        String::from_utf8(serial.stdout).unwrap(),
        "1 OFF OFF normal\n2 OFF OFF normal\n3 500 500 critical\n4 500 500 critical\n5 500 500 critical\n"
    );
    let invalid = source.replace(
        "status: {business_hours_only: true}",
        "missing: {business_hours_only: true}",
    );
    fs::write(&config, invalid).unwrap();
    let result = run("1");
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("field_frequency field not in template")
    );
    assert!(result.stdout.is_empty());
}

#[test]
fn stateful_custom_frequency_interleaves_with_builtin_generators() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("mixed.yaml");
    fs::write(&config, "log_generator:\n  global: {output_format: console, total_logs: 60, start_time: '2024-01-01'}\n  log_types:\n    custom:\n      frequency: 1\n      template: 'CUSTOM {counter} {status}'\n      field_frequency:\n        distributions: {status: {ok: 1}}\n    redis: {frequency: 1}\n").unwrap();
    let run = |threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--seed",
                "11",
                "--threads",
                threads,
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = run("1");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    let parallel = run("4");
    assert!(parallel.status.success());
    assert_eq!(serial.stdout, parallel.stdout);
    let logs = String::from_utf8(serial.stdout).unwrap();
    let mut expected = 1;
    let mut redis = 0;
    for line in logs.lines() {
        if line.starts_with("CUSTOM ") {
            assert_eq!(line, format!("CUSTOM {expected} ok"));
            expected += 1;
        } else {
            assert!(line.contains(":M "));
            redis += 1;
        }
    }
    assert!(expected > 1 && redis > 0);
}

#[test]
fn custom_template_accepts_original_single_value_options() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.yaml");
    fs::write(&path, "log_generator:\n  global:\n    output_format: console\n    total_logs: 2\n    start_time: '2024-01-01'\n  log_types:\n    custom:\n      enabled: true\n      template: '{counter:ignored} {random_int:ignored}'\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--config", path.to_str().unwrap(), "--no-monitor"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 2);
    for (line, expected) in lines.iter().zip([1, 2]) {
        let fields: Vec<_> = line.split_whitespace().collect();
        assert_eq!(fields[0], expected.to_string());
        assert!((1..=100).contains(&fields[1].parse::<u32>().unwrap()));
    }
}

#[test]
fn django_sql_default_models_include_original_framework_tables() {
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "django_sql",
            "--total",
            "1000",
            "--start-time",
            "2024-01-01",
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("app_contenttype"));
    assert!(output.contains("app_logentry"));
}

#[test]
fn database_generators_include_original_default_tables() {
    for (kind, tables) in [
        (
            "mysql_query",
            &["categories", "logs", "permissions", "articles"][..],
        ),
        ("mysql_slow", &["categories", "logs"][..]),
        (
            "postgresql",
            &["categories", "audit_log", "permissions", "articles"][..],
        ),
        ("postgresql_slow", &["categories", "audit_log"][..]),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "console",
                "--types",
                kind,
                "--total",
                "1000",
                "--start-time",
                "2024-01-01",
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        for table in tables {
            assert!(output.contains(table), "{kind} missing {table}");
        }
    }
}

#[test]
fn slow_queries_keep_original_subquery_structure() {
    for kind in ["mysql_slow", "postgresql_slow"] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "console",
                "--types",
                kind,
                "--total",
                "1000",
                "--start-time",
                "2024-01-01",
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        assert!(
            output.contains("_id FROM"),
            "{kind} missing foreign key subquery"
        );
        let interval = if kind == "mysql_slow" {
            "INTERVAL 1 YEAR"
        } else {
            "INTERVAL '1 year'"
        };
        assert!(
            output.contains(interval),
            "{kind} missing original subquery interval"
        );
    }
}

#[test]
fn mysql_general_logs_include_original_admin_commands() {
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "mysql_query",
            "--total",
            "1000",
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    for command in [" Init DB ", " Field List ", " Refresh "] {
        assert!(output.contains(command), "missing {command}");
    }
    let validator = log_forge::validation::LogValidator::new("mysql_query").unwrap();
    assert!(output.lines().all(|line| validator.validate(line).is_valid));
}

#[test]
fn mysql_queries_include_original_select_and_show_variants() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("mysql.yaml");
    for (query, variants) in [
        ("SELECT", &[" JOIN ", " WHERE "][..]),
        (
            "SHOW",
            &[
                "SHOW TABLES",
                "SHOW DATABASES",
                "SHOW COLUMNS FROM",
                "SHOW INDEX FROM",
            ][..],
        ),
    ] {
        fs::write(&config, format!("log_generator:\n  global: {{output_format: console, total_logs: 500}}\n  log_types:\n    mysql_query:\n      query_types: {{{query}: 1}}\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        for variant in variants {
            assert!(text.contains(variant), "{query} missing {variant}");
        }
    }
    fs::write(&config, "log_generator:\n  global: {output_format: console, total_logs: 100}\n  log_types:\n    mysql_query:\n      query_types: {SELECT: 1}\n      table_names: [audit_events]\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn database_foreign_keys_do_not_include_schema_names() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.yaml");
    for kind in ["mysql_query", "mysql_slow", "postgresql_slow"] {
        fs::write(&path, format!("log_generator:\n  global: {{output_format: console, total_logs: 1000}}\n  log_types:\n    {kind}:\n      query_types: {{SELECT: 1, DELETE: 1}}\n      table_names: [app.users, app.orders]\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            !text.contains("t2.app."),
            "{kind} contains a schema in a column name"
        );
        assert!(
            !text.contains("SELECT app."),
            "{kind} contains a schema in a column name"
        );
        assert!(
            !text.contains("SELECT DISTINCT app."),
            "{kind} contains a schema in a column name"
        );
        assert!(
            text.contains("user_id") || text.contains("order_id"),
            "{kind} did not emit a foreign key"
        );
    }
}

#[test]
fn mysql_write_queries_use_original_varied_columns() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("mysql.yaml");
    for query in ["INSERT", "UPDATE", "DELETE"] {
        fs::write(&config, format!("log_generator:\n  global: {{output_format: console, total_logs: 300}}\n  log_types:\n    mysql_query:\n      query_types: {{{query}: 1}}\n      table_names: [audit_events]\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        match query {
            "INSERT" => {
                assert!(text.contains(", "));
                assert!(text.contains("NOW()"));
            }
            "UPDATE" => {
                assert!(text.contains(" SET email = "));
                assert!(text.contains(" SET updated_at = NOW()"));
            }
            _ => assert!(text.contains(" WHERE email = ")),
        }
    }
}

#[test]
fn mysql_slow_insert_matches_original_bulk_insert_shape() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 1}\n  log_types:\n    mysql_slow:\n      table_names: [audit_events]\n      query_types: {INSERT: 1}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("INSERT INTO audit_events (name, email, created_at)"));
    assert!(text.contains("CONCAT('user_', id, '@example.com'), NOW()"));
}

#[test]
fn postgresql_statements_use_original_field_and_join_shapes() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("postgres.yaml");
    for query in ["SELECT", "INSERT", "UPDATE"] {
        fs::write(&config, format!("log_generator:\n  global: {{output_format: console, total_logs: 600}}\n  log_types:\n    postgresql:\n      log_levels: {{LOG: 1}}\n      query_types: {{{query}: 1}}\n      table_names: [app.users, app.orders]\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        match query {
            "SELECT" => {
                assert!(text.contains("_id = t2.id"));
                assert!(!text.contains("t1.id = t2.id"));
            }
            "INSERT" => {
                assert!(text.contains("$1, $2)"));
                assert!(text.contains("$1, $2, $3, $4)"));
            }
            _ => {
                assert!(text.contains(" SET email = $1"));
                assert!(text.contains(" SET password_hash = $1"));
            }
        }
    }
}

#[test]
fn django_sql_includes_original_query_shapes_and_arguments() {
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "django_sql",
            "--total",
            "1000",
            "--seed",
            "42",
            "--start-time",
            "2024-01-01",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    for expected in [
        " INNER JOIN ",
        " ORDER BY ",
        "\"email\", \"created_at\"",
        "\"updated_at\" = %s",
        "args=(True,)",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
}

#[test]
fn django_sql_arguments_match_placeholder_count() {
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "django_sql",
            "--total",
            "1000",
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    for line in text.lines() {
        let (query, args) = line.split_once("; args=").unwrap();
        let arguments = args.strip_prefix('(').unwrap().strip_suffix(')').unwrap();
        let count = arguments.trim_end_matches(',').split(", ").count();
        assert_eq!(query.matches("%s").count(), count, "{line}");
    }
}

#[test]
fn django_requests_include_original_method_user_and_latency_ranges() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("django.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 3000}\n  log_types:\n    django_request:\n      log_levels: {INFO: 1}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("\"PATCH "));
    assert!(text.contains("\"HEAD "));
    assert!(text.contains("user: AnonymousUser"));
    let times = regex::Regex::new(r"\[(\d+)ms\]").unwrap();
    assert!(
        times
            .captures_iter(&text)
            .any(|c| c[1].parse::<u32>().unwrap() > 2000)
    );
}

#[test]
fn django_tracebacks_include_original_exception_and_source_variants() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("django.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 200}\n  log_types:\n    django_request:\n      log_levels: {ERROR: 1}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    for expected in [
        "DoesNotExist: User matching query does not exist.",
        "KeyError: 'required_field'",
        "AttributeError: 'NoneType' object has no attribute 'name'",
        "data = json.loads(request.body)",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
}

#[test]
fn django_authenticated_usernames_vary() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("django.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 100}\n  log_types:\n    django_request:\n      log_levels: {INFO: 1}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    let expression = regex::Regex::new(r"\(user: ([^,]+), ip:").unwrap();
    let users: std::collections::BTreeSet<_> = expression
        .captures_iter(&text)
        .map(|capture| capture[1].to_owned())
        .filter(|user| user != "AnonymousUser")
        .collect();
    assert!(users.len() > 1, "{users:?}");
}

#[test]
fn fastapi_default_requests_include_original_methods_and_endpoints() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fastapi.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 1000}\n  log_types:\n    fastapi:\n      log_levels: {INFO: 1}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    for expected in [
        "Request completed: PATCH ",
        "Request completed: OPTIONS ",
        "/api/v1/health",
        "/openapi.json",
        "/api/v1/auth/refresh",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
}

#[test]
fn fastapi_default_latency_respects_status_code() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fastapi.yaml");
    let pattern = regex::Regex::new(r"Request completed: .* - (\d+) - (\d+)ms").unwrap();
    for (status, low, high) in [
        (200, 50, 1500),
        (404, 10, 100),
        (400, 20, 200),
        (500, 2000, 30000),
    ] {
        fs::write(&path, format!("log_generator:\n  global: {{output_format: console, total_logs: 200}}\n  log_types:\n    fastapi:\n      log_levels: {{INFO: 1}}\n      custom_fields:\n        status_codes: {{{status}: 1}}\n")).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let text = String::from_utf8(result.stdout).unwrap();
        let mut checked = 0;
        for capture in pattern.captures_iter(&text) {
            let actual_status = capture[1].parse::<u16>().unwrap();
            let latency = capture[2].parse::<u64>().unwrap();
            assert_eq!(actual_status, status);
            assert!((low..=high).contains(&latency), "{status}: {latency}");
            checked += 1;
        }
        assert!(checked > 0, "no request logs for {status}");
    }
}

#[test]
fn fastapi_application_messages_vary_original_fields() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fastapi.yaml");
    for (level, variants) in [
        (
            "INFO",
            &[
                "Database query executed: SELECT",
                "Database query executed: INSERT",
                "Database query executed: UPDATE",
                "Database query executed: DELETE",
            ][..],
        ),
        (
            "ERROR",
            &[
                "invalid_credentials",
                "account_locked",
                "token_expired",
                "Service unavailable",
                "Missing required field",
            ][..],
        ),
    ] {
        fs::write(&path, format!("log_generator:\n  global: {{output_format: console, total_logs: 2000}}\n  log_types:\n    fastapi:\n      log_levels: {{{level}: 1}}\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        for variant in variants {
            assert!(text.contains(variant), "{level} missing {variant}");
        }
    }
}

#[test]
fn fastapi_deprecation_date_follows_virtual_time() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fastapi.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 500, start_time: '2040-01-01'}\n  log_types:\n    fastapi:\n      log_levels: {WARNING: 1}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    let expression = regex::Regex::new(r"will be deprecated on (\d{4}-\d{2}-\d{2})").unwrap();
    let dates: Vec<_> = expression
        .captures_iter(&text)
        .map(|capture| capture[1].to_owned())
        .collect();
    assert!(!dates.is_empty());
    assert!(
        dates.iter().all(|date| date.as_str() > "2040-01-01"),
        "{dates:?}"
    );
}

#[test]
fn fastapi_dynamic_endpoints_vary_with_seeded_data() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fastapi.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 3000, start_time: '2024-01-01'}\n  log_types:\n    fastapi:\n      log_levels: {INFO: 1}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    for expression in [
        r"/api/v1/search\?q=([^ ]+)",
        r"/api/v1/categories/([^ ]+)",
        r"/api/v1/reports/(\d{4}-\d{2}-\d{2})",
    ] {
        let re = regex::Regex::new(expression).unwrap();
        let values: std::collections::BTreeSet<_> = re
            .captures_iter(&text)
            .map(|item| item[1].to_owned())
            .collect();
        assert!(values.len() > 1, "{expression}: {values:?}");
    }
}

#[test]
fn docker_application_fields_vary_with_original_ranges() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("docker.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 1000}\n  log_types:\n    docker:\n      format_type: text\n      log_levels: {info: 1}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    let pool = regex::Regex::new(r"Connection pool size: (\d+)").unwrap();
    let pools: std::collections::BTreeSet<u32> = pool
        .captures_iter(&text)
        .map(|v| v[1].parse().unwrap())
        .collect();
    assert!(
        pools.len() > 1 && pools.iter().all(|v| (5..=50).contains(v)),
        "{pools:?}"
    );
    for service in ["database", "cache", "api", "auth"] {
        assert!(
            text.contains(&format!("Health check passed for service {service}")),
            "{service}"
        );
    }
}

#[test]
fn kubernetes_pod_messages_vary_original_fields() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("pod.yaml");
    for level in ["INFO", "ERROR"] {
        fs::write(&path, format!("log_generator:\n  global: {{output_format: console, total_logs: 1000}}\n  log_types:\n    kubernetes_pod:\n      log_levels: {{{level}: 1}}\n")).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let text = String::from_utf8(result.stdout).unwrap();
        let expression = if level == "INFO" {
            r"probe succeeded for (\w+)"
        } else {
            r"exited with code (\d+)"
        };
        let regex = regex::Regex::new(expression).unwrap();
        let values: std::collections::BTreeSet<_> = regex
            .captures_iter(&text)
            .map(|c| c[1].to_owned())
            .collect();
        assert!(values.len() > 1, "{level}: {values:?}");
    }
}

#[test]
fn kubernetes_events_include_original_object_kinds() {
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "kubernetes_event",
            "--total",
            "1000",
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    for kind in [
        "Pod",
        "Node",
        "Service",
        "Deployment",
        "ReplicaSet",
        "ConfigMap",
        "Secret",
        "PersistentVolume",
        "PersistentVolumeClaim",
    ] {
        assert!(text.contains(&format!(" {kind}/")), "missing {kind}");
    }
    let validator = log_forge::validation::LogValidator::new("kubernetes_event").unwrap();
    assert!(text.lines().all(|line| validator.validate(line).is_valid));
}

#[test]
fn kubernetes_warning_events_include_original_message_variants() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("event.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 2000}\n  log_types:\n    kubernetes_event:\n      event_types: {Warning: 1}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    for expected in [
        "Insufficient cpu",
        "Insufficient memory",
        "Error: ImagePullBackOff",
        "Error: CrashLoopBackOff",
        "Error: CreateContainerConfigError",
        "Liveness probe failed:",
        "Readiness probe failed:",
        "Get timeout",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
}

#[test]
fn kubernetes_event_json_level_matches_event_type() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("event.yaml");
    let output = root.path().join("event.jsonl");
    for (kind, expected) in [("Normal", "INFO"), ("Warning", "WARNING")] {
        if output.exists() {
            fs::remove_file(&output).unwrap();
        }
        fs::write(&config, format!("log_generator:\n  global: {{output_format: json, output_path: '{}', total_logs: 20}}\n  log_types:\n    kubernetes_event:\n      event_types: {{{kind}: 1}}\n", output.display())).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let contents = fs::read_to_string(&output).unwrap();
        for line in contents.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["level"], expected);
        }
    }
}

#[test]
fn kubernetes_default_pod_identity_is_stable_across_threads() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("pod.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 500, start_time: '2024-01-01'}\n  log_types:\n    kubernetes_pod:\n      log_levels: {ERROR: 1}\n").unwrap();
    let generate = |threads| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--seed",
                "42",
                "--threads",
                threads,
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let serial = generate("1");
    let parallel = generate("4");
    assert!(
        serial.status.success(),
        "{}",
        String::from_utf8_lossy(&serial.stderr)
    );
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(serial.stdout, parallel.stdout);
    let text = String::from_utf8(serial.stdout).unwrap();
    let expression =
        regex::Regex::new(r"Pod (\S+) in namespace (\S+) failed readiness probe").unwrap();
    let identities: std::collections::BTreeSet<_> = expression
        .captures_iter(&text)
        .map(|capture| (capture[1].to_owned(), capture[2].to_owned()))
        .collect();
    assert_eq!(identities.len(), 1, "{identities:?}");
}

#[test]
fn web_access_sizes_follow_original_status_ranges() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("access.yaml");
    let re = regex::Regex::new(r#"\"[A-Z]+ [^\"]+\" (\d+) (\d+)"#).unwrap();
    for (kind, status, low, high) in [
        ("nginx_access", 200, 500, 50000),
        ("nginx_access", 404, 150, 500),
        ("nginx_access", 301, 200, 400),
        ("nginx_access", 503, 100, 300),
        ("apache_access", 200, 800, 75000),
        ("apache_access", 404, 200, 600),
        ("apache_access", 301, 250, 500),
        ("apache_access", 503, 150, 400),
    ] {
        fs::write(&path, format!("log_generator:\n  global: {{output_format: console, total_logs: 100}}\n  log_types:\n    {kind}:\n      custom_fields:\n        status_codes: {{{status}: 1}}\n")).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let text = String::from_utf8(result.stdout).unwrap();
        let matches: Vec<_> = re.captures_iter(&text).collect();
        assert_eq!(matches.len(), 100, "{kind} {status}");
        for capture in matches {
            assert_eq!(capture[1].parse::<u16>().unwrap(), status);
            let size = capture[2].parse::<u32>().unwrap();
            assert!((low..=high).contains(&size), "{kind} {status}: {size}");
        }
    }
}

#[test]
fn web_access_dynamic_referers_vary() {
    let pattern = regex::Regex::new(r"https://ref(\d+)\.example\.test/").unwrap();
    for kind in ["nginx_access", "apache_access"] {
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "console",
                "--types",
                kind,
                "--total",
                "1000",
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        let domains: std::collections::BTreeSet<_> = pattern
            .captures_iter(&text)
            .map(|capture| capture[1].to_owned())
            .collect();
        assert!(domains.len() > 1, "{kind}: {domains:?}");
    }
}

#[test]
fn web_access_generates_original_dynamic_url_shapes() {
    let cases: &[(&str, &[&str])] = &[
        (
            "nginx_access",
            &[
                "/api/users/",
                "/products/",
                "/search?q=",
                "/category/",
                "/page/",
            ],
        ),
        (
            "apache_access",
            &[
                "/user/",
                "/product/",
                "/search.php?q=",
                "/category/",
                "/page.php?id=",
                "/wp-content/uploads/",
            ],
        ),
    ];
    for (kind, paths) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "console",
                "--types",
                kind,
                "--total",
                "2000",
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        for path in *paths {
            assert!(text.contains(path), "{kind} missing {path}");
        }
    }
}

#[test]
fn web_error_critical_levels_use_original_critical_messages() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("error.yaml");
    for (kind, level) in [
        ("nginx_error", "crit"),
        ("nginx_error", "alert"),
        ("apache_error", "crit"),
        ("apache_error", "alert"),
        ("apache_error", "emerg"),
    ] {
        fs::write(&path, format!("log_generator:\n  global: {{output_format: console, total_logs: 20}}\n  log_types:\n    {kind}:\n      error_levels: [{level}]\n      error_patterns: ['custom-pattern-only']\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{kind} {level}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("custom-pattern-only"), "{kind} {level}");
    }
}

#[test]
fn nginx_emerg_retains_original_custom_error_pattern_behavior() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nginx.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 20}\n  log_types:\n    nginx_error:\n      error_levels: [emerg]\n      error_patterns: [custom-marker]\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 20);
    assert!(
        text.lines()
            .all(|line| line.contains("[emerg]") && line.contains("custom-marker"))
    );
}

#[test]
fn web_error_identifiers_and_modules_follow_original_ranges() {
    let nginx = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "nginx_error",
            "--total",
            "1000",
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        nginx.status.success(),
        "{}",
        String::from_utf8_lossy(&nginx.stderr)
    );
    let text = String::from_utf8(nginx.stdout).unwrap();
    let re = regex::Regex::new(r"\] (\d+)#(\d+): \*(\d+)").unwrap();
    let values: Vec<_> = re.captures_iter(&text).collect();
    assert_eq!(values.len(), 1000);
    for capture in values {
        assert!((1000..=99999).contains(&capture[1].parse::<u32>().unwrap()));
        assert!((0..=999).contains(&capture[2].parse::<u32>().unwrap()));
        assert!((1..=99999).contains(&capture[3].parse::<u32>().unwrap()));
    }
    let apache = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "apache_error",
            "--total",
            "1000",
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        apache.status.success(),
        "{}",
        String::from_utf8_lossy(&apache.stderr)
    );
    let text = String::from_utf8(apache.stdout).unwrap();
    for module in [
        "core",
        "mod_ssl",
        "mod_rewrite",
        "mod_php",
        "mod_fcgid",
        "mod_security",
        "mod_dir",
        "mod_auth",
    ] {
        assert!(text.contains(&format!("[{module}:error]")), "{module}");
    }
}

#[test]
fn mysql_error_messages_vary_original_fields() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mysql.yaml");
    for (level, expected) in [
        (
            "Note",
            &[
                "port: 3306",
                "port: 3307",
                "port: 3308",
                "Version: '8.0.20'",
                "Version: '8.0.35'",
            ][..],
        ),
        (
            "Error",
            &[
                "using password: YES",
                "using password: NO",
                "Table 'production.",
                "Table 'test.",
            ][..],
        ),
    ] {
        fs::write(&path, format!("log_generator:\n  global: {{output_format: console, total_logs: 2000}}\n  log_types:\n    mysql_error:\n      log_levels: {{{level}: 1}}\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                path.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        for value in expected {
            assert!(text.contains(value), "{level} missing {value}");
        }
    }
}

#[test]
fn mysql_error_json_preserves_note_level() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("mysql.yaml");
    let path = root.path().join("mysql.jsonl");
    fs::write(&config, format!("log_generator:\n  global: {{output_format: json, output_path: '{}', total_logs: 100}}\n  log_types:\n    mysql_error:\n      log_levels: {{Note: 1}}\n", path.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let contents = fs::read_to_string(path).unwrap();
    assert_eq!(contents.lines().count(), 100);
    for line in contents.lines() {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(value["level"], "NOTE");
    }
}

#[test]
fn postgresql_json_preserves_log_and_panic_levels() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("postgres.yaml");
    let path = root.path().join("postgres.jsonl");
    for level in ["LOG", "PANIC"] {
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        fs::write(&config, format!("log_generator:\n  global: {{output_format: json, output_path: '{}', total_logs: 50}}\n  log_types:\n    postgresql:\n      log_levels: {{{level}: 1}}\n", path.display())).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents.lines().count(), 50);
        for line in contents.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["level"], level);
        }
    }
}

#[test]
fn syslog_kernel_numeric_fields_use_original_ranges() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("syslog.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 500}\n  log_types:\n    syslog:\n      facilities: [kern]\n      message_patterns:\n        kern: ['cpu={cpu} ttl={ttl} dport={dport} buf={bufsize}']\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    let expression = regex::Regex::new(r"cpu=(\d+) ttl=(\d+) dport=(\d+) buf=(\d+)").unwrap();
    let captures: Vec<_> = expression.captures_iter(&text).collect();
    assert_eq!(captures.len(), 500);
    for c in captures {
        assert!((0..=7).contains(&c[1].parse::<u32>().unwrap()), "{c:?}");
        assert!((32..=255).contains(&c[2].parse::<u32>().unwrap()), "{c:?}");
        assert!((1..=65535).contains(&c[3].parse::<u32>().unwrap()), "{c:?}");
        assert!(
            [65536, 131072, 262144].contains(&c[4].parse::<u32>().unwrap()),
            "{c:?}"
        );
    }
}

#[test]
fn syslog_network_fields_use_original_choices() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("syslog.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 500}\n  log_types:\n    syslog:\n      facilities: [kern]\n      message_patterns:\n        kern: ['iface={iface} state={link_state} proto={proto} flag={tcp_flags}']\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    for value in [
        "iface=eth0",
        "iface=ens33",
        "iface=enp0s3",
        "iface=wlan0",
        "state=up",
        "state=down",
        "proto=TCP",
        "proto=UDP",
        "proto=ICMP",
        "flag=SYN,ACK",
        "flag=RST",
    ] {
        assert!(text.contains(value), "missing {value}");
    }
}

#[test]
fn syslog_mail_and_identity_fields_vary() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("syslog.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 100}\n  log_types:\n    syslog:\n      facilities: [mail]\n      message_patterns:\n        mail: ['from={from} to={to} message={message_id} user={user}']\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    for field in ["from", "to", "message", "user"] {
        let regex = regex::Regex::new(&format!(r"{field}=(\S+)")).unwrap();
        let values: std::collections::BTreeSet<_> = regex
            .captures_iter(&text)
            .map(|capture| capture[1].to_owned())
            .collect();
        assert!(values.len() > 1, "{field}: {values:?}");
    }
}

#[test]
fn syslog_repeated_fields_keep_same_generated_value() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("syslog.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 100}\n  log_types:\n    syslog:\n      facilities: [auth]\n      message_patterns:\n        auth: ['user={user} copy={user} id={message_id} id_copy={message_id}']\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let re = regex::Regex::new(r"user=(\S+) copy=(\S+) id=(\S+) id_copy=(\S+)").unwrap();
    let values: Vec<_> = re.captures_iter(&text).collect();
    assert_eq!(values.len(), 100);
    for capture in values {
        assert_eq!(&capture[1], &capture[2]);
        assert_eq!(&capture[3], &capture[4]);
    }
}

#[test]
fn syslog_rfc5424_uses_original_auth_tags_and_optional_fields() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("syslog.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 300, start_time: '2024-01-01'}\n  log_types:\n    syslog:\n      facilities: [auth]\n      severities: [info]\n      rfc_format: '5424'\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    let mut tags = std::collections::BTreeSet::new();
    let mut processes = std::collections::BTreeSet::new();
    let mut ids = std::collections::BTreeSet::new();
    for line in text.lines() {
        let parts: Vec<_> = line.split_whitespace().collect();
        tags.insert(parts[3]);
        processes.insert(parts[4]);
        ids.insert(parts[5]);
    }
    for tag in ["sshd", "sudo", "su", "login"] {
        assert!(tags.contains(tag), "{tags:?}");
    }
    assert!(
        processes.contains("-") && processes.len() > 1,
        "{processes:?}"
    );
    assert!(
        ids.contains("-") && ids.iter().any(|id| *id != "-"),
        "{ids:?}"
    );
}

#[test]
fn web_error_json_normalizes_critical_levels() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let path = root.path().join("output.jsonl");
    for (kind, level, expected) in [
        ("nginx_error", "crit", "CRITICAL"),
        ("nginx_error", "emerg", "EMERGENCY"),
        ("apache_error", "alert", "ALERT"),
    ] {
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        fs::write(&config, format!("log_generator:\n  global: {{output_format: json, output_path: '{}', total_logs: 10}}\n  log_types:\n    {kind}:\n      error_levels: [{level}]\n", path.display())).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--seed",
                "42",
                "--no-monitor",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 10);
        for line in text.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["level"], expected);
        }
    }
}

#[test]
fn fastapi_slow_query_dates_follow_virtual_time() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fastapi.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 1000, start_time: '2040-01-01'}\n  log_types:\n    fastapi:\n      log_levels: {WARNING: 1}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    let re = regex::Regex::new(
        r"Slow query detected: SELECT \* FROM orders WHERE created_at > '(\d{4}-\d{2}-\d{2})'",
    )
    .unwrap();
    let dates: std::collections::BTreeSet<_> = re
        .captures_iter(&text)
        .map(|capture| capture[1].to_owned())
        .collect();
    assert!(dates.len() > 1, "{dates:?}");
    assert!(dates.iter().all(|date| date.as_str() <= "2040-01-01"));
}

#[test]
fn fastapi_debug_bodies_include_varied_original_fields() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fastapi.yaml");
    fs::write(&path, "log_generator:\n  global: {output_format: console, total_logs: 1000}\n  log_types:\n    fastapi:\n      log_levels: {DEBUG: 1}\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            path.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    let mut request_ids = std::collections::BTreeSet::new();
    let mut response_ids = std::collections::BTreeSet::new();
    let mut saw_headers = false;
    for line in text.lines() {
        if let Some((_, body)) = line.split_once("Request body: ") {
            let value: serde_json::Value = serde_json::from_str(body).unwrap();
            request_ids.insert(value["user_id"].as_u64().unwrap());
        }
        if let Some((_, body)) = line.split_once("Response body: ") {
            let value: serde_json::Value = serde_json::from_str(body).unwrap();
            response_ids.insert(value["data"]["id"].as_u64().unwrap());
        }
        if let Some((_, body)) = line.split_once("Request headers: ") {
            let value: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(value["Content-Type"], "application/json");
            saw_headers = true;
        }
    }
    assert!(saw_headers && request_ids.len() > 1 && response_ids.len() > 1);
}

#[test]
fn fastapi_debug_nested_json_survives_structured_output() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("fastapi.yaml");
    let output = root.path().join("fastapi.jsonl");
    fs::write(&config, format!("log_generator:\n  global: {{output_format: json, output_path: '{}', total_logs: 500}}\n  log_types:\n    fastapi:\n      log_levels: {{DEBUG: 1}}\n", output.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let contents = fs::read_to_string(output).unwrap();
    let mut request_count = 0;
    let mut response_count = 0;
    for line in contents.lines() {
        let outer: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(outer["level"], "DEBUG");
        let message = outer["message"].as_str().unwrap();
        if let Some((_, body)) = message.split_once("Request body: ") {
            let inner: serde_json::Value = serde_json::from_str(body).unwrap();
            assert!(inner["user_id"].as_u64().is_some());
            request_count += 1;
        }
        if let Some((_, body)) = message.split_once("Response body: ") {
            let inner: serde_json::Value = serde_json::from_str(body).unwrap();
            assert!(inner["data"]["id"].as_u64().is_some());
            response_count += 1;
        }
    }
    assert!(request_count > 0 && response_count > 0);
}

#[test]
fn django_dynamic_search_and_category_paths_vary() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("django.yaml");
    fs::write(&config, "log_generator:\n  global: {output_format: console, total_logs: 3000}\n  log_types:\n    django_request:\n      log_levels: {INFO: 1}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    for expression in [r#"/search/\?q=([^" ]+)"#, r#"/category/([^/" ]+)/"#] {
        let re = regex::Regex::new(expression).unwrap();
        let values: std::collections::BTreeSet<_> = re
            .captures_iter(&text)
            .map(|capture| capture[1].to_owned())
            .collect();
        assert!(values.len() > 1, "{expression}: {values:?}");
    }
}

#[test]
fn syslog_rfc3164_uses_original_auth_tags_with_optional_pid() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("syslog.yaml");
    fs::write(&config, "log_generator:\n  global: {output_format: console, total_logs: 500, start_time: '2024-01-01'}\n  log_types:\n    syslog:\n      facilities: [auth]\n      severities: [info]\n      rfc_format: '3164'\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--seed",
            "42",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    let regex = regex::Regex::new(r" (sshd|sudo|su|login)(?:\[(\d+)\])?: ").unwrap();
    let captures: Vec<_> = regex.captures_iter(&text).collect();
    assert_eq!(captures.len(), 500);
    let tags: std::collections::BTreeSet<_> = captures.iter().map(|c| c[1].to_owned()).collect();
    assert_eq!(tags.len(), 4);
    assert!(captures.iter().any(|c| c.get(2).is_none()));
    assert!(captures.iter().any(|c| c.get(2).is_some()));
    let validator = log_forge::validation::LogValidator::new("syslog").unwrap();
    assert!(text.lines().all(|line| validator.validate(line).is_valid));
}

#[test]
fn continue_on_error_continues_after_http_failure() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        for index in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut data = Vec::new();
            let mut chunk = [0; 4096];
            while !data.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0);
                data.extend_from_slice(&chunk[..count]);
            }
            let header_end = data
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap()
                + 4;
            let headers = String::from_utf8_lossy(&data[..header_end]);
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            while data.len() - header_end < length {
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0);
                data.extend_from_slice(&chunk[..count]);
            }
            let status = if index == 0 {
                "503 Service Unavailable"
            } else {
                "200 OK"
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        }
    });
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "network",
            "--protocol",
            "http",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--endpoint",
            "/logs",
            "--types",
            "redis",
            "--total",
            "2",
            "--continue-on-error",
            "--async-io",
            "--threads",
            "4",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("output error for redis: HTTP 503"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("threads are not applied"));
}

#[test]
fn continue_on_error_supports_async_output() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("not-created.log");
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "file",
            "--path",
            path.to_str().unwrap(),
            "--types",
            "redis",
            "--total",
            "1",
            "--continue-on-error",
            "--async-io",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(path).unwrap().lines().count(), 1);
}

#[test]
fn continue_on_error_parallel_generation_matches_single_thread() {
    let run = |threads| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "console",
                "--types",
                "redis",
                "--total",
                "300",
                "--seed",
                "42",
                "--start-time",
                "2024-01-01",
                "--threads",
                threads,
                "--continue-on-error",
                "--no-monitor",
            ])
            .output()
            .unwrap()
    };
    let single = run("1");
    let parallel = run("4");
    assert!(
        single.status.success(),
        "{}",
        String::from_utf8_lossy(&single.stderr)
    );
    assert!(
        parallel.status.success(),
        "{}",
        String::from_utf8_lossy(&parallel.stderr)
    );
    assert_eq!(single.stdout, parallel.stdout);
    assert!(!String::from_utf8_lossy(&parallel.stderr).contains("threads are not applied"));
}

#[test]
fn parallel_generation_advances_time_after_failed_records() {
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "fastapi",
            "--total",
            "100",
            "--seed",
            "42",
            "--start-time",
            "+262142-12-30 00:00:00",
            "--interval",
            "0.001",
            "--threads",
            "4",
            "--continue-on-error",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("generation error"));
    let records = String::from_utf8(output.stdout).unwrap();
    let timestamps: Vec<_> = records
        .lines()
        .map(|line| line.split_once(" [").unwrap().0)
        .collect();
    assert_eq!(timestamps.len(), 100);
    assert!(timestamps.windows(2).all(|pair| pair[0] < pair[1]));
}

#[cfg(unix)]
#[test]
fn continue_on_error_reports_count_through_control_status() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("control.sock");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (failed_tx, failed_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for index in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut data = Vec::new();
            let mut chunk = [0; 4096];
            while !data.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0);
                data.extend_from_slice(&chunk[..count]);
            }
            let header_end = data
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap()
                + 4;
            let headers = String::from_utf8_lossy(&data[..header_end]);
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            while data.len() - header_end < length {
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0);
                data.extend_from_slice(&chunk[..count]);
            }
            let status = if index == 0 {
                "503 Service Unavailable"
            } else {
                "200 OK"
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            if index == 0 {
                failed_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        }
    });
    let child = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "network",
            "--protocol",
            "http",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--endpoint",
            "/logs",
            "--types",
            "redis",
            "--total",
            "2",
            "--continue-on-error",
            "--control-socket",
            socket.to_str().unwrap(),
            "--no-monitor",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    failed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    let observed = loop {
        let status = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["status", "--control-socket", socket.to_str().unwrap()])
            .output()
            .unwrap();
        if status.status.success() {
            let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
            if value["error_count"] == 1 && value["generated"] == 1 {
                break Some(value);
            }
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    release_tx.send(()).unwrap();
    let output = child.wait_with_output().unwrap();
    server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status = observed.expect("error count did not become visible");
    assert_eq!(status["generated"], 1);
}

#[cfg(unix)]
#[test]
fn persistent_output_errors_can_be_stopped_through_control() {
    use std::{
        net::TcpListener,
        process::{Child, Stdio},
        thread,
        time::{Duration, Instant},
    };
    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("control.sock");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "network",
                "--protocol",
                "http",
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--endpoint",
                "/logs",
                "--network-timeout",
                "0.1",
                "--types",
                "redis",
                "--total",
                "0",
                "--continue-on-error",
                "--control-socket",
                socket.to_str().unwrap(),
                "--no-monitor",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !socket.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(socket.exists(), "control socket did not start");
    drop(listener);
    let status = loop {
        let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["status", "--control-socket", socket.to_str().unwrap()])
            .output()
            .unwrap();
        if output.status.success() {
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            if value["error_count"].as_u64().unwrap_or(0) >= 2 {
                break value;
            }
        }
        assert!(
            Instant::now() < deadline,
            "persistent errors did not become visible"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert!(status["generated"].as_u64().unwrap() >= 1);
    let stopped = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["stop", "--control-socket", socket.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        stopped.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "generator did not stop");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn continue_on_error_attempts_every_output_once_per_generated_record() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use std::time::Duration;
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first.log");
    let last = root.path().join("last.log");
    let config = root.path().join("config.yaml");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let running = Arc::new(AtomicBool::new(true));
    let requests = Arc::new(AtomicUsize::new(0));
    let active = Arc::clone(&running);
    let count = Arc::clone(&requests);
    let server = std::thread::spawn(move || {
        while active.load(Ordering::SeqCst) {
            let (mut stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut data = Vec::new();
            let mut chunk = [0; 4096];
            while !data.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0);
                data.extend_from_slice(&chunk[..read]);
            }
            let header_end = data
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap()
                + 4;
            let headers = String::from_utf8_lossy(&data[..header_end]);
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            while data.len() - header_end < length {
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0);
                data.extend_from_slice(&chunk[..read]);
            }
            let index = count.fetch_add(1, Ordering::SeqCst);
            let status = if index == 0 {
                "503 Service Unavailable"
            } else {
                "200 OK"
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        }
    });
    fs::write(&config, format!("log_generator:\n  global: {{total_logs: 1}}\n  log_types:\n    redis: {{enabled: true}}\n  output:\n    handlers:\n      - {{type: file, file_path: '{}'}}\n      - {{type: network, protocol: http, host: 127.0.0.1, port: {port}, endpoint: /logs}}\n      - {{type: file, file_path: '{}'}}\n", first.display(), last.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-generator"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--types",
            "redis",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    running.store(false, Ordering::SeqCst);
    server.join().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read_to_string(first).unwrap().lines().count(), 1);
    assert_eq!(fs::read_to_string(last).unwrap().lines().count(), 1);
}

#[test]
fn original_command_can_stop_on_output_error() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let result = Command::new(env!("CARGO_BIN_EXE_log-generator"))
        .args([
            "start",
            "--output",
            "network",
            "--protocol",
            "http",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--types",
            "redis",
            "--total",
            "1",
            "--fail-fast",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("unexpected argument"));
}

#[test]
fn continue_on_error_survives_batch_flush_failure() {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("records.log");
    let config = root.path().join("config.yaml");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut count = 0;
        while count < 3 && Instant::now() < deadline {
            let (mut stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut data = Vec::new();
            let mut chunk = [0; 4096];
            while !data.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0);
                data.extend_from_slice(&chunk[..read]);
            }
            let end = data
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap()
                + 4;
            let headers = String::from_utf8_lossy(&data[..end]);
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            while data.len() - end < length {
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0);
                data.extend_from_slice(&chunk[..read]);
            }
            let status = if count == 0 {
                "400 Bad Request"
            } else {
                "204 No Content"
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            count += 1;
        }
        count
    });
    fs::write(&config, format!("log_generator:\n  global: {{total_logs: 2, batch_size: 1}}\n  log_types:\n    redis: {{enabled: true}}\n  output:\n    handlers:\n      - {{type: network, protocol: http, host: 127.0.0.1, port: {port}, endpoint: /api/v1/write, format: prometheus}}\n      - {{type: file, file_path: '{}'}}\n", output.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--continue-on-error",
            "--async-io",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    let requests = server.join().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("flush error"));
    assert_eq!(requests, 3);
    assert_eq!(fs::read_to_string(output).unwrap().lines().count(), 2);
}

#[test]
fn continue_on_error_reports_close_failure_and_closes_other_outputs() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::time::Duration;

    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("records.log");
    let config = root.path().join("config.yaml");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut length = 0;
        loop {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length: ") {
                length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        stream
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
    });
    fs::write(&config, format!("log_generator:\n  global: {{total_logs: 1}}\n  log_types:\n    redis: {{enabled: true}}\n  output:\n    handlers:\n      - {{type: network, protocol: http, host: 127.0.0.1, port: {port}, endpoint: /api/v1/write, format: prometheus}}\n      - {{type: file, file_path: '{}'}}\n", output.display())).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--continue-on-error",
            "--async-io",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("close error"));
    assert_eq!(fs::read_to_string(output).unwrap().lines().count(), 1);
}
