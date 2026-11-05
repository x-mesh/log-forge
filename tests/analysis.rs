use log_forge::analysis::{
    LogAnalyzer, SamplingStrategy, compare_log_sets, generate_samples, quality_report,
};

#[test]
fn scenario_analysis_uses_header_level_not_message_text() {
    let mut analyzer = LogAnalyzer::new("scenario").unwrap();
    analyzer.observe(
        "2024-01-01T00:00:00+00:00 [trace] scenario=example pattern=sample ERROR in message",
    );
    let stats = analyzer.statistics();
    assert_eq!(stats.valid_logs, 1);
    assert_eq!(stats.log_levels.get("TRACE"), Some(&1));
    assert!(!stats.log_levels.contains_key("ERROR"));
}

#[test]
fn structured_error_analysis_uses_header_level() {
    for (kind, record) in [
        (
            "error_library",
            "2024-01-01T00:00:00+00:00 [trace] category=test pattern=one ERROR in message",
        ),
        (
            "error_pattern",
            "2024-01-01T00:00:00.000Z [trace] category=test ERROR in message",
        ),
        (
            "microservice",
            "2024-01-01T00:00:00.000Z [trace] service=api operation=test ERROR in message",
        ),
    ] {
        let mut analyzer = LogAnalyzer::new(kind).unwrap();
        analyzer.observe(record);
        let stats = analyzer.statistics();
        assert_eq!(stats.valid_logs, 1, "{kind}");
        assert_eq!(stats.log_levels.get("TRACE"), Some(&1), "{kind}");
        assert!(!stats.log_levels.contains_key("ERROR"), "{kind}");
    }
}

#[test]
fn scenario_stratified_samples_distinguish_trace_and_error_headers() {
    let records: Vec<String> = [
        "2024-01-01T00:00:00+00:00 [TRACE] scenario=example pattern=one ERROR in message",
        "2024-01-01T00:00:01+00:00 [TRACE] scenario=example pattern=two ERROR in message",
        "2024-01-01T00:00:02+00:00 [ERROR] scenario=example pattern=three failure",
        "2024-01-01T00:00:03+00:00 [ERROR] scenario=example pattern=four failure",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let sample =
        generate_samples(&records, "scenario", 2, SamplingStrategy::Stratified, 42).unwrap();
    assert!(sample.logs.iter().any(|record| record.contains("[TRACE]")));
    assert!(sample.logs.iter().any(|record| record.contains("[ERROR]")));
}

#[test]
fn structured_log_sampling_uses_level_before_embedded_http_status() {
    let records: Vec<String> = [
        "2024-01-01T00:00:00+00:00 [TRACE] category=test pattern=one \"GET / HTTP/1.1\" 200",
        "2024-01-01T00:00:01+00:00 [TRACE] category=test pattern=two \"GET / HTTP/1.1\" 200",
        "2024-01-01T00:00:02+00:00 [ERROR] category=test pattern=three \"GET / HTTP/1.1\" 200",
        "2024-01-01T00:00:03+00:00 [ERROR] category=test pattern=four \"GET / HTTP/1.1\" 200",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for seed in 0..20 {
        let sample = generate_samples(
            &records,
            "error_library",
            2,
            SamplingStrategy::Stratified,
            seed,
        )
        .unwrap();
        assert!(
            sample.logs.iter().any(|record| record.contains("[TRACE]")),
            "seed={seed}"
        );
        assert!(
            sample.logs.iter().any(|record| record.contains("[ERROR]")),
            "seed={seed}"
        );
    }
}

#[test]
fn statistics_dictionary_limits_ranked_fields() {
    let mut stats = log_forge::analysis::LogStatistics::default();
    for index in 0..12 {
        stats.ip_addresses.insert(format!("ip{index:02}"), index);
        stats.user_agents.insert(format!("agent{index:02}"), index);
        stats
            .error_patterns
            .insert(format!("error{index:02}"), index);
    }
    let value = stats.to_dict();
    assert_eq!(value["top_ip_addresses"].as_object().unwrap().len(), 10);
    assert_eq!(value["top_user_agents"].as_object().unwrap().len(), 5);
    assert_eq!(value["error_patterns"].as_object().unwrap().len(), 10);
    assert!(value["top_ip_addresses"].get("ip00").is_none());
    assert_eq!(value["top_user_agents"]["agent11"], 11);
    assert_eq!(stats.ip_addresses.len(), 12);
}

#[test]
fn counts_hours_for_redis_and_goloop_records() {
    let mut redis = LogAnalyzer::new("redis").unwrap();
    redis.observe("78000:M 02 Jan 2024 13:14:17.000 * Background saving terminated with success");
    assert_eq!(redis.statistics().time_distribution.get("13"), Some(&1));
    let mut goloop = LogAnalyzer::new("goloop").unwrap();
    goloop.observe(
        "D|20240102-13:14:15.000000|6df7|1|SV|TM|manager.go:355 reportValidation(err=<nil>)",
    );
    assert_eq!(goloop.statistics().time_distribution.get("13"), Some(&1));
}

#[test]
fn redis_analysis_uses_severity_symbol_instead_of_message_text() {
    let mut analyzer = LogAnalyzer::new("redis").unwrap();
    analyzer.observe("123:M 02 Jan 2024 13:14:17.000 . ERROR in sample text");
    analyzer.observe("123:M 02 Jan 2024 13:14:18.000 - clients connected");
    analyzer.observe("123:M 02 Jan 2024 13:14:19.000 * Ready to accept connections");
    analyzer.observe("123:M 02 Jan 2024 13:14:20.000 # Connection with replica lost");
    let stats = analyzer.statistics();
    assert_eq!(stats.valid_logs, 4);
    for level in ["DEBUG", "VERBOSE", "NOTICE", "WARNING"] {
        assert_eq!(stats.log_levels.get(level), Some(&1), "{level}");
    }
    assert!(!stats.log_levels.contains_key("ERROR"));
}

#[test]
fn cli_analyzes_configured_redis_severity_distribution() {
    use std::{collections::BTreeMap, fs, process::Command};
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("redis.yaml");
    let path = root.path().join("redis_20240102.log");
    fs::write(&config, "log_generator:\n  global: {output_format: console, total_logs: 200, start_time: '2024-01-02'}\n  log_types:\n    redis:\n      log_levels: {DEBUG: 1, VERBOSE: 1, NOTICE: 1, WARNING: 1}\n").unwrap();
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
    let mut expected = BTreeMap::new();
    for line in logs.lines() {
        let level = match line.split_whitespace().nth(5).unwrap() {
            "." => "DEBUG",
            "-" => "VERBOSE",
            "*" => "NOTICE",
            "#" => "WARNING",
            symbol => panic!("unexpected severity {symbol}"),
        };
        *expected.entry(level).or_insert(0_u64) += 1;
    }
    assert_eq!(expected.len(), 4);
    fs::write(&path, logs).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["analyze", "--input", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["redis"]["valid_logs"], 200);
    assert_eq!(
        report["redis"]["log_levels"],
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn goloop_analysis_uses_header_level_not_message_text() {
    let mut analyzer = LogAnalyzer::new("goloop").unwrap();
    analyzer
        .observe("D|20240102-13:14:15.000000|6df7|1|SV|manager.go:355 ERROR inside debug message");
    analyzer.observe("E|20240102-13:14:16.000000|6df7|2|SV|manager.go:355 connection failed");
    let stats = analyzer.statistics();
    assert_eq!(stats.valid_logs, 2);
    assert_eq!(stats.log_levels.get("DEBUG"), Some(&1));
    assert_eq!(stats.log_levels.get("ERROR"), Some(&1));
}

#[test]
fn syslog_analysis_uses_priority_instead_of_message_text() {
    let mut analyzer = LogAnalyzer::new("syslog").unwrap();
    analyzer.observe("<30>Jan  1 00:00:00 server daemon: ERROR in text");
    analyzer.observe("[daemon.err]Jan  1 00:00:01 server daemon: connection failed");
    analyzer.observe("Jan  1 00:00:02 server daemon: ERROR without priority");
    let stats = analyzer.statistics();
    assert_eq!(stats.valid_logs, 3);
    assert_eq!(stats.log_levels.get("INFO"), Some(&1));
    assert_eq!(stats.log_levels.get("ERROR"), Some(&1));
    assert_eq!(stats.log_levels.values().sum::<u64>(), 2);
}

#[test]
fn cli_syslog_analysis_matches_generated_priorities() {
    use std::{collections::BTreeMap, fs, process::Command};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("syslog_20240101.log");
    let generated = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "syslog",
            "--total",
            "300",
            "--start-time",
            "2024-01-01",
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
    let logs = String::from_utf8(generated.stdout).unwrap();
    let mut expected = BTreeMap::new();
    for line in logs.lines() {
        let priority: u64 = line
            .strip_prefix('<')
            .unwrap()
            .split_once('>')
            .unwrap()
            .0
            .parse()
            .unwrap();
        let level = [
            "EMERGENCY",
            "ALERT",
            "CRITICAL",
            "ERROR",
            "WARNING",
            "NOTICE",
            "INFO",
            "DEBUG",
        ][(priority % 8) as usize];
        *expected.entry(level.to_owned()).or_insert(0_u64) += 1;
    }
    fs::write(&path, logs).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["analyze", "--input", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["syslog"]["valid_logs"], 300);
    assert_eq!(
        report["syslog"]["log_levels"],
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn cli_analyzes_goloop_json_level_and_hour() {
    use std::{fs, process::Command};
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let path = root.path().join("goloop_20240102.log");
    fs::write(&config, "log_generator:\n  global:\n    output_format: console\n    total_logs: 40\n    generation_interval: 0\n    start_time: '2024-01-02 13:14:15'\n  log_types:\n    goloop:\n      enabled: true\n      format_type: json\n      log_levels: {E: 1}\n").unwrap();
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
    fs::write(&path, generated.stdout).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["analyze", "--input", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["goloop"]["valid_logs"], 40);
    assert_eq!(report["goloop"]["log_levels"]["ERROR"], 40);
    assert_eq!(report["goloop"]["time_distribution"]["13"], 40);
}

#[test]
fn cli_generates_samples_and_analyzes_generated_file() {
    use std::{fs, process::Command};
    let sample = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "sample",
            "--type",
            "nginx",
            "--count",
            "2",
            "--seed",
            "42",
            "--start-time",
            "2024-01-01",
        ])
        .output()
        .unwrap();
    assert!(
        sample.status.success(),
        "{}",
        String::from_utf8_lossy(&sample.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&sample.stdout).lines().count(), 2);
    let path = std::env::temp_dir().join(format!("log-forge-analysis-{}.log", std::process::id()));
    fs::write(&path, &sample.stdout).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "analyze",
            "--input",
            path.to_str().unwrap(),
            "--type",
            "nginx_access",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let data: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(data["nginx_access"]["total_logs"], 2);
    assert_eq!(data["nginx_access"]["valid_logs"], 2);
    fs::remove_file(path).unwrap();
}

#[test]
fn counts_fields_validation_and_error_patterns() {
    let mut analyzer = LogAnalyzer::new("nginx_access").unwrap();
    analyzer.observe("192.168.1.100 - - [25/Dec/2023:10:00:00 +0000] \"GET / HTTP/1.1\" 200 1234 \"-\" \"Mozilla/5.0\"");
    analyzer.observe("192.168.1.100 - - [25/Dec/2023:11:00:00 +0000] \"GET /bad HTTP/1.1\" 500 1234 \"-\" \"Mozilla/5.0\"");
    analyzer.observe("invalid log entry");
    let stats = analyzer.statistics();
    assert_eq!(stats.total_logs, 3);
    assert_eq!(stats.valid_logs, 2);
    assert_eq!(stats.status_codes.get("200"), Some(&1));
    assert_eq!(stats.status_codes.get("500"), Some(&1));
    assert_eq!(stats.ip_addresses.get("192.168.1.100"), Some(&2));
    assert_eq!(stats.user_agents.get("Mozilla/5.0"), Some(&2));
    assert_eq!(stats.time_distribution.get("10"), Some(&1));
    assert!((stats.field_completeness["ip"] - 2.0 / 3.0).abs() < 0.001);
    assert!(stats.average_log_length > 0.0);
    assert_eq!(stats.quality_metrics["validation_rate"], 2.0 / 3.0);
    assert_eq!(stats.quality_metrics["error_rate"], 1.0 / 3.0);
    assert_eq!(
        stats.quality_metrics["average_quality_score"],
        stats.average_quality_score
    );
    assert!((0.0..=1.0).contains(&stats.quality_metrics["completeness_score"]));
}

#[test]
fn cli_reports_quality_metrics_for_mixed_and_empty_files() {
    use std::{fs, process::Command};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nginx_access_test.log");
    fs::write(&path, "192.168.1.100 - - [25/Dec/2023:10:00:00 +0000] \"GET / HTTP/1.1\" 200 1234 \"-\" \"Mozilla/5.0\"\ninvalid\n").unwrap();
    let analyze = || {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["analyze", "--input", path.to_str().unwrap()])
            .output()
            .unwrap()
    };
    let output = analyze();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let metrics = &report["nginx_access"]["quality_metrics"];
    assert_eq!(metrics["validation_rate"], 0.5);
    assert_eq!(metrics["error_rate"], 0.5);
    assert_eq!(
        metrics["average_quality_score"],
        report["nginx_access"]["average_quality_score"]
    );
    assert!(
        metrics["completeness_score"]
            .as_f64()
            .is_some_and(|score| (0.0..=1.0).contains(&score))
    );
    fs::write(&path, "").unwrap();
    let empty = analyze();
    assert!(empty.status.success());
    let report: serde_json::Value = serde_json::from_slice(&empty.stdout).unwrap();
    assert_eq!(
        report["nginx_access"]["quality_metrics"],
        serde_json::json!({})
    );
}

#[test]
fn ssh_auth_analysis_counts_authentication_outcomes_without_inventing_levels() {
    let mut analyzer = LogAnalyzer::new("ssh_auth").unwrap();
    analyzer.observe("Jan  1 00:00:00 server sshd[123]: Failed password for invalid user admin from 192.0.2.1 port 5000 ssh2");
    analyzer.observe("Jan  1 00:00:01 server sshd[123]: Accepted publickey for deploy from 192.0.2.2 port 5001 ssh2: ED25519 SHA256:syntheticFingerprint");
    analyzer
        .observe("Jan  1 00:00:02 server sshd[123]: Invalid user test from 192.0.2.1 port 5002");
    analyzer.observe("Jan  1 00:00:03 server sshd[123]: Connection closed by authenticating user info 192.0.2.3 port 5003 [preauth]");
    let stats = analyzer.statistics();
    assert_eq!(stats.valid_logs, 4);
    assert_eq!(stats.ip_addresses.get("192.0.2.1"), Some(&2));
    assert_eq!(stats.authentication_events.get("failed"), Some(&1));
    assert_eq!(stats.authentication_events.get("accepted"), Some(&1));
    assert_eq!(stats.authentication_events.get("invalid_user"), Some(&1));
    assert_eq!(
        stats.authentication_events.get("connection_closed"),
        Some(&1)
    );
    assert!(stats.log_levels.is_empty());
}

#[test]
fn cli_analyzes_generated_ssh_authentication_outcomes() {
    use std::{fs, process::Command};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ssh_auth_20240101.log");
    let generate = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--output",
            "console",
            "--types",
            "ssh_auth",
            "--total",
            "300",
            "--seed",
            "42",
            "--start-time",
            "2024-01-01",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        generate.status.success(),
        "{}",
        String::from_utf8_lossy(&generate.stderr)
    );
    fs::write(&path, generate.stdout).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["analyze", "--input", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let stats = &report["ssh_auth"];
    assert_eq!(stats["total_logs"], 300);
    assert_eq!(stats["valid_logs"], 300);
    let events = stats["authentication_events"].as_object().unwrap();
    assert_eq!(
        events
            .values()
            .map(|value| value.as_u64().unwrap())
            .sum::<u64>(),
        300
    );
    for category in ["failed", "accepted", "invalid_user", "connection_closed"] {
        assert!(events[category].as_u64().unwrap() > 0, "{category}");
    }
    assert!(stats["log_levels"].as_object().unwrap().is_empty());
}

#[test]
fn sampling_is_seeded_unique_and_covers_strata() {
    let logs = vec![
        "10.0.0.1 - - [25/Dec/2023:10:00:00 +0000] \"GET / HTTP/1.1\" 200 1".to_owned(),
        "10.0.0.2 - - [25/Dec/2023:10:00:00 +0000] \"GET / HTTP/1.1\" 404 1".to_owned(),
        "10.0.0.3 - - [25/Dec/2023:10:00:00 +0000] \"GET / HTTP/1.1\" 500 1".to_owned(),
        "invalid".to_owned(),
    ];
    for strategy in [
        SamplingStrategy::Random,
        SamplingStrategy::Systematic,
        SamplingStrategy::Stratified,
        SamplingStrategy::QualityBased,
    ] {
        let first = generate_samples(&logs, "nginx_access", 3, strategy, 42).unwrap();
        let second = generate_samples(&logs, "nginx_access", 3, strategy, 42).unwrap();
        assert_eq!(first.logs, second.logs);
        assert_eq!(first.logs.len(), 3);
        assert!(first.logs.iter().all(|record| logs.contains(record)));
        let json = serde_json::to_value(&first).unwrap();
        assert_eq!(json["metadata"]["original_count"], 4);
        assert_eq!(json["metadata"]["sampling_ratio"], 0.75);
        assert_eq!(json["metadata"]["strategy"], json["strategy"]);
        let compatible = first.to_dict();
        assert!(compatible.get("strategy").is_none());
        assert_eq!(compatible["metadata"], json["metadata"]);
        assert_eq!(compatible["logs"], json["logs"]);
        assert!(chrono::DateTime::parse_from_rfc3339(json["timestamp"].as_str().unwrap()).is_ok());
    }
    let empty = generate_samples(&[], "nginx_access", 3, SamplingStrategy::Random, 42).unwrap();
    assert_eq!(empty.to_dict()["metadata"], serde_json::json!({}));
    let stratified = generate_samples(
        &logs[..3],
        "nginx_access",
        3,
        SamplingStrategy::Stratified,
        0,
    )
    .unwrap();
    assert_eq!(stratified.logs.len(), 3);
    assert!(
        generate_samples(&logs, "nginx_access", 0, SamplingStrategy::Systematic, 0)
            .unwrap()
            .logs
            .is_empty()
    );
}

#[test]
fn stratified_sampling_uses_redis_and_goloop_severity() {
    let redis = vec![
        "123:M 02 Jan 2024 13:14:17.000 . event 1".to_owned(),
        "123:M 02 Jan 2024 13:14:17.000 . event 2".to_owned(),
        "123:M 02 Jan 2024 13:14:17.000 # event 1".to_owned(),
        "123:M 02 Jan 2024 13:14:17.000 # event 2".to_owned(),
    ];
    let goloop = vec![
        "D|20240102-13:14:15.000000|6df7|1|SV|manager.go:355 event one".to_owned(),
        "D|20240102-13:14:15.000000|6df7|2|SV|manager.go:355 event two".to_owned(),
        "E|20240102-13:14:15.000000|6df7|3|SV|manager.go:355 event one".to_owned(),
        "E|20240102-13:14:15.000000|6df7|4|SV|manager.go:355 event two".to_owned(),
    ];
    for seed in 0..20 {
        let sample =
            generate_samples(&redis, "redis", 2, SamplingStrategy::Stratified, seed).unwrap();
        assert!(
            sample.logs.iter().any(|line| line.contains(" . ")),
            "seed {seed}"
        );
        assert!(
            sample.logs.iter().any(|line| line.contains(" # ")),
            "seed {seed}"
        );
        let sample =
            generate_samples(&goloop, "goloop", 2, SamplingStrategy::Stratified, seed).unwrap();
        assert!(
            sample.logs.iter().any(|line| line.starts_with("D|")),
            "seed {seed}"
        );
        assert!(
            sample.logs.iter().any(|line| line.starts_with("E|")),
            "seed {seed}"
        );
    }
}

#[test]
fn stratified_sampling_covers_syslog_priorities_and_ssh_outcomes() {
    let syslog = vec![
        "<30>Jan  1 00:00:00 server daemon: event one".to_owned(),
        "<30>Jan  1 00:00:01 server daemon: event two".to_owned(),
        "<27>Jan  1 00:00:02 server daemon: event three".to_owned(),
        "<27>Jan  1 00:00:03 server daemon: event four".to_owned(),
    ];
    let ssh = vec![
        "Jan  1 00:00:00 server sshd[1]: Accepted password for admin from 192.0.2.1 port 5000 ssh2"
            .to_owned(),
        "Jan  1 00:00:01 server sshd[2]: Accepted password for admin from 192.0.2.1 port 5000 ssh2"
            .to_owned(),
        "Jan  1 00:00:02 server sshd[3]: Failed password for admin from 192.0.2.1 port 5000 ssh2"
            .to_owned(),
        "Jan  1 00:00:03 server sshd[4]: Failed password for admin from 192.0.2.1 port 5000 ssh2"
            .to_owned(),
    ];
    for seed in 0..20 {
        let sample =
            generate_samples(&syslog, "syslog", 2, SamplingStrategy::Stratified, seed).unwrap();
        assert!(sample.logs.iter().any(|line| line.starts_with("<30>")));
        assert!(sample.logs.iter().any(|line| line.starts_with("<27>")));
        let sample =
            generate_samples(&ssh, "ssh_auth", 2, SamplingStrategy::Stratified, seed).unwrap();
        assert!(
            sample
                .logs
                .iter()
                .any(|line| line.contains("Accepted password")),
            "seed {seed}"
        );
        assert!(
            sample
                .logs
                .iter()
                .any(|line| line.contains("Failed password")),
            "seed {seed}"
        );
    }
}

#[test]
fn compares_distributions_and_reports_quality() {
    let valid = "10.0.0.1 - - [25/Dec/2023:10:00:00 +0000] \"GET / HTTP/1.1\" 200 1".to_owned();
    let failure = "10.0.0.2 - - [25/Dec/2023:10:00:00 +0000] \"GET / HTTP/1.1\" 500 1".to_owned();
    let comparison =
        compare_log_sets(std::slice::from_ref(&valid), &[failure], "nginx_access").unwrap();
    assert_eq!(comparison.status_differences["200"].difference, 1.0);
    assert_eq!(comparison.status_differences["500"].difference, 1.0);
    let compatible = comparison.to_dict();
    assert_eq!(compatible["differences"]["status_codes"]["total_keys"], 2);
    assert_eq!(compatible["differences"]["status_codes"]["common_keys"], 0);
    assert_eq!(
        compatible["differences"]["status_codes"]["significant_differences"]["200"]["set1_proportion"],
        1.0
    );
    assert_eq!(compatible["similarities"], serde_json::json!({}));
    let empty = compare_log_sets(&[], &[], "nginx_access")
        .unwrap()
        .to_dict();
    assert_eq!(empty["differences"], serde_json::json!({}));
    let report = quality_report(&[valid, "invalid".to_owned()], "nginx_access").unwrap();
    assert_eq!(report.total_logs_analyzed, 2);
    assert_eq!(report.statistics.valid_logs, 1);
    let validator = log_forge::validation::LogValidator::new("nginx_access").unwrap();
    for result in &report.validation_results {
        let separate = validator.validate(&result.log_entry);
        assert_eq!(result.is_valid, separate.is_valid);
        assert_eq!(result.quality_score, separate.quality_score);
        assert_eq!(result.errors, separate.errors);
        assert_eq!(result.field_completeness, separate.field_completeness);
    }
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["validation_summary"]["valid_logs"], 1);
    assert_eq!(json["validation_summary"]["invalid_logs"], 1);
    let compatible = report.to_dict();
    assert!(compatible.get("validation_results").is_none());
    assert_eq!(compatible["validation_summary"], json["validation_summary"]);
    assert_eq!(compatible["statistics"], report.statistics.to_dict());
    assert_eq!(compatible["total_logs_analyzed"], 2);
    assert_eq!(
        json["validation_summary"]["average_quality_score"],
        json["statistics"]["average_quality_score"]
    );
    assert!(chrono::DateTime::parse_from_rfc3339(json["timestamp"].as_str().unwrap()).is_ok());
    assert!(report.overall_score < 80.0);
    assert!(!report.recommendations.is_empty());
    assert!(!report.issues_found.is_empty());
}

#[test]
fn quality_report_detects_concentrated_traffic_and_error_distributions() {
    let line = "192.0.2.1 - - [25/Dec/2023:10:00:00 +0000] \"GET / HTTP/1.1\" 500 1234";
    let records = vec![line.to_owned(); 101];
    let report = quality_report(&records, "nginx_access").unwrap();
    assert!(
        report
            .recommendations
            .iter()
            .any(|entry| entry.contains("IP address diversity"))
    );
    assert!(
        report
            .recommendations
            .iter()
            .any(|entry| entry.contains("success rate"))
    );
    assert!(
        report
            .issues_found
            .iter()
            .any(|entry| entry.contains("IP address dominates"))
    );
    assert!(
        report
            .issues_found
            .iter()
            .any(|entry| entry.contains("single hour"))
    );
}

#[test]
fn quality_report_respects_error_rate_threshold() {
    let log = |level: &str, index: usize| {
        format!("{level}|20240102-13:14:15.000000|6df7|{index}|SV|manager.go:355 event")
    };
    let mut records: Vec<_> = (0..9).map(|index| log("I", index)).collect();
    records.push(log("E", 9));
    let boundary = quality_report(&records, "goloop").unwrap();
    assert!(
        !boundary
            .recommendations
            .iter()
            .any(|entry| entry.contains("High error rate"))
    );
    records[0] = log("E", 0);
    let exceeded = quality_report(&records, "goloop").unwrap();
    assert!(
        exceeded
            .recommendations
            .iter()
            .any(|entry| entry.contains("High error rate"))
    );
}

#[test]
fn normalizes_error_patterns_and_handles_empty_input() {
    let mut analyzer = LogAnalyzer::new("syslog").unwrap();
    assert_eq!(analyzer.statistics().total_logs, 0);
    analyzer.observe("<27>Dec 25 10:00:00 server kernel: ERROR: connection 123 failed");
    analyzer.observe("<27>Dec 25 10:00:01 server kernel: ERROR: connection 456 failed");
    let stats = analyzer.statistics();
    assert_eq!(
        stats.error_patterns.get("connection <num> failed"),
        Some(&2)
    );
    assert_eq!(stats.log_levels.get("ERROR"), Some(&2));
}

#[test]
fn kubernetes_event_analysis_uses_event_type_not_message_text() {
    let mut analyzer = LogAnalyzer::new("kubernetes_event").unwrap();
    analyzer.observe("2024-01-01T00:00:00.000Z Normal Scheduled Pod/example ERROR in message");
    analyzer.observe("2024-01-01T00:00:01.000Z Warning Failed Pod/example INFO in message");
    let stats = analyzer.statistics();
    assert_eq!(stats.log_levels.get("INFO"), Some(&1));
    assert_eq!(stats.log_levels.get("WARNING"), Some(&1));
    assert!(!stats.log_levels.contains_key("ERROR"));
}

#[test]
fn kubernetes_event_stratification_uses_event_type_before_embedded_http_status() {
    let records: Vec<String> = [
        "2024-01-01T00:00:00.000Z Normal Scheduled Pod/a \"GET / HTTP/1.1\" 200 message",
        "2024-01-01T00:00:01.000Z Normal Scheduled Pod/b \"GET / HTTP/1.1\" 200 message",
        "2024-01-01T00:00:02.000Z Warning Failed Pod/c \"GET / HTTP/1.1\" 200 message",
        "2024-01-01T00:00:03.000Z Warning Failed Pod/d \"GET / HTTP/1.1\" 200 message",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for kind in ["kubernetes_event", "kubernetes"] {
        for seed in 0..20 {
            let sample =
                generate_samples(&records, kind, 2, SamplingStrategy::Stratified, seed).unwrap();
            assert!(
                sample.logs.iter().any(|line| line.contains(" Normal ")),
                "{kind} seed={seed}"
            );
            assert!(
                sample.logs.iter().any(|line| line.contains(" Warning ")),
                "{kind} seed={seed}"
            );
        }
    }
}

#[test]
fn mysql_error_analysis_uses_mysql_header_level() {
    let mut analyzer = LogAnalyzer::new("mysql_error").unwrap();
    analyzer.observe("2024-01-01T00:00:00.000Z [Note] ERROR in message");
    analyzer.observe("2024-01-01T00:00:01.000Z [Warning] INFO in message");
    analyzer.observe("2024-01-01T00:00:02.000Z [Error] failure");
    let stats = analyzer.statistics();
    assert_eq!(stats.log_levels.get("NOTE"), Some(&1));
    assert_eq!(stats.log_levels.get("WARNING"), Some(&1));
    assert_eq!(stats.log_levels.get("ERROR"), Some(&1));
    assert!(!stats.log_levels.contains_key("INFO"));
}

#[test]
fn mysql_error_stratification_uses_header_before_message() {
    let records: Vec<String> = [
        "2024-01-01T00:00:00.000Z [Note] \"GET / HTTP/1.1\" 200 ERROR in message",
        "2024-01-01T00:00:01.000Z [Note] \"GET / HTTP/1.1\" 200 ERROR in message",
        "2024-01-01T00:00:02.000Z [Error] \"GET / HTTP/1.1\" 200 INFO in message",
        "2024-01-01T00:00:03.000Z [Error] \"GET / HTTP/1.1\" 200 INFO in message",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for seed in 0..20 {
        let sample = generate_samples(
            &records,
            "mysql_error",
            2,
            SamplingStrategy::Stratified,
            seed,
        )
        .unwrap();
        assert!(
            sample.logs.iter().any(|line| line.contains("[Note]")),
            "seed={seed}"
        );
        assert!(
            sample.logs.iter().any(|line| line.contains("[Error]")),
            "seed={seed}"
        );
    }
}

#[test]
fn postgresql_log_analysis_uses_header_before_message() {
    let mut analyzer = LogAnalyzer::new("postgresql").unwrap();
    analyzer.observe("2024-01-01 00:00:00.000 UTC [1234]: [LOG] ERROR appears in message");
    analyzer.observe("2024-01-01 00:00:01.000 UTC [1234]: [ERROR] INFO appears in message");
    let stats = analyzer.statistics();
    assert_eq!(stats.log_levels.get("LOG"), Some(&1));
    assert_eq!(stats.log_levels.get("ERROR"), Some(&1));
    assert!(!stats.log_levels.contains_key("INFO"));
}

#[test]
fn postgresql_stratification_uses_header_before_embedded_http_status() {
    let records: Vec<String> = [
        "2024-01-01 00:00:00.000 UTC [1234]: [LOG] \"GET / HTTP/1.1\" 200 ERROR in message",
        "2024-01-01 00:00:01.000 UTC [1234]: [LOG] \"GET / HTTP/1.1\" 200 ERROR in message",
        "2024-01-01 00:00:02.000 UTC [1234]: [PANIC] \"GET / HTTP/1.1\" 200 INFO in message",
        "2024-01-01 00:00:03.000 UTC [1234]: [PANIC] \"GET / HTTP/1.1\" 200 INFO in message",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for seed in 0..20 {
        let sample = generate_samples(
            &records,
            "postgresql",
            2,
            SamplingStrategy::Stratified,
            seed,
        )
        .unwrap();
        assert!(
            sample.logs.iter().any(|line| line.contains("[LOG]")),
            "seed={seed}"
        );
        assert!(
            sample.logs.iter().any(|line| line.contains("[PANIC]")),
            "seed={seed}"
        );
    }
}

#[test]
fn postgresql_panic_contributes_to_quality_error_warning() {
    let records: Vec<String> = (0..10)
        .map(|index| {
            let level = if index < 2 { "PANIC" } else { "LOG" };
            format!("2024-01-01 00:00:{index:02}.000 UTC [1234]: [{level}] test message")
        })
        .collect();
    let report = quality_report(&records, "postgresql").unwrap();
    assert_eq!(report.statistics.log_levels["PANIC"], 2);
    assert!(
        report
            .recommendations
            .iter()
            .any(|text| text.contains("High error rate"))
    );
}

#[test]
fn syslog_stratification_uses_priority_before_embedded_http_status() {
    let records: Vec<String> = [
        "<14>Jan 01 00:00:00 host systemd: \"GET / HTTP/1.1\" 200 ERROR in message",
        "<14>Jan 01 00:00:01 host systemd: \"GET / HTTP/1.1\" 200 ERROR in message",
        "<11>Jan 01 00:00:02 host systemd: \"GET / HTTP/1.1\" 200 INFO in message",
        "<11>Jan 01 00:00:03 host systemd: \"GET / HTTP/1.1\" 200 INFO in message",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for seed in 0..20 {
        let sample =
            generate_samples(&records, "syslog", 2, SamplingStrategy::Stratified, seed).unwrap();
        assert!(
            sample.logs.iter().any(|line| line.starts_with("<14>")),
            "seed={seed}"
        );
        assert!(
            sample.logs.iter().any(|line| line.starts_with("<11>")),
            "seed={seed}"
        );
    }
}

#[test]
fn redis_and_goloop_sampling_prioritizes_record_level() {
    let cases: [(&str, [&str; 4], &str, &str); 2] = [
        (
            "redis",
            [
                "123:M 02 Jan 2024 13:14:17.000 * \"GET / HTTP/1.1\" 200 ok",
                "123:M 02 Jan 2024 13:14:18.000 * \"GET / HTTP/1.1\" 200 ok",
                "123:M 02 Jan 2024 13:14:19.000 # \"GET / HTTP/1.1\" 200 failure",
                "123:M 02 Jan 2024 13:14:20.000 # \"GET / HTTP/1.1\" 200 failure",
            ],
            " * ",
            " # ",
        ),
        (
            "goloop",
            [
                "I|20240102-13:14:15.000000|6df7|1|SV|manager.go:355 \"GET / HTTP/1.1\" 200 ok",
                "I|20240102-13:14:16.000000|6df7|1|SV|manager.go:355 \"GET / HTTP/1.1\" 200 ok",
                "E|20240102-13:14:17.000000|6df7|1|SV|manager.go:355 \"GET / HTTP/1.1\" 200 failure",
                "E|20240102-13:14:18.000000|6df7|1|SV|manager.go:355 \"GET / HTTP/1.1\" 200 failure",
            ],
            "I|",
            "E|",
        ),
    ];
    for (kind, records, low, high) in cases {
        let records: Vec<String> = records.into_iter().map(str::to_owned).collect();
        for seed in 0..20 {
            let sample =
                generate_samples(&records, kind, 2, SamplingStrategy::Stratified, seed).unwrap();
            assert!(
                sample.logs.iter().any(|line| line.contains(low)),
                "{kind} seed={seed}"
            );
            assert!(
                sample.logs.iter().any(|line| line.contains(high)),
                "{kind} seed={seed}"
            );
        }
    }
}

#[test]
fn pod_and_kafka_sampling_prioritize_header_level() {
    let cases: [(&str, [&str; 4], &str, &str); 2] = [
        (
            "kubernetes_pod",
            [
                "2024-01-01T00:00:00.000Z INFO \"GET / HTTP/1.1\" 200 normal",
                "2024-01-01T00:00:01.000Z INFO \"GET / HTTP/1.1\" 200 normal",
                "2024-01-01T00:00:02.000Z ERROR \"GET / HTTP/1.1\" 200 failed",
                "2024-01-01T00:00:03.000Z ERROR \"GET / HTTP/1.1\" 200 failed",
            ],
            " INFO ",
            " ERROR ",
        ),
        (
            "kafka_broker",
            [
                "[2024-01-01 00:00:00,000] INFO \"GET / HTTP/1.1\" 200 normal (kafka.Server)",
                "[2024-01-01 00:00:01,000] INFO \"GET / HTTP/1.1\" 200 normal (kafka.Server)",
                "[2024-01-01 00:00:02,000] ERROR \"GET / HTTP/1.1\" 200 failed (kafka.Server)",
                "[2024-01-01 00:00:03,000] ERROR \"GET / HTTP/1.1\" 200 failed (kafka.Server)",
            ],
            " INFO ",
            " ERROR ",
        ),
    ];
    for (kind, records, info, error) in cases {
        let records: Vec<String> = records.into_iter().map(str::to_owned).collect();
        for seed in 0..20 {
            let sample =
                generate_samples(&records, kind, 2, SamplingStrategy::Stratified, seed).unwrap();
            assert!(
                sample.logs.iter().any(|line| line.contains(info)),
                "{kind} seed={seed}"
            );
            assert!(
                sample.logs.iter().any(|line| line.contains(error)),
                "{kind} seed={seed}"
            );
        }
    }
}

#[test]
fn kafka_trace_analysis_uses_header_not_message() {
    let mut analyzer = LogAnalyzer::new("kafka_broker").unwrap();
    analyzer.observe("[2024-01-01 00:00:00,000] TRACE ERROR in message (kafka.Server)");
    analyzer.observe("[2024-01-01 00:00:01,000] ERROR INFO in message (kafka.Server)");
    let stats = analyzer.statistics();
    assert_eq!(stats.valid_logs, 2);
    assert_eq!(stats.log_levels.get("TRACE"), Some(&1));
    assert_eq!(stats.log_levels.get("ERROR"), Some(&1));
    assert!(!stats.log_levels.contains_key("INFO"));
}

#[test]
fn syslog_emergency_and_alert_contribute_to_quality_error_warning() {
    let records: Vec<String> = (0..10)
        .map(|index| {
            let priority = match index {
                0 => 8,
                1 => 9,
                _ => 14,
            };
            format!("<{priority}>Jan 01 00:00:{index:02} host systemd: test message")
        })
        .collect();
    let report = quality_report(&records, "syslog").unwrap();
    assert_eq!(report.statistics.log_levels["EMERGENCY"], 1);
    assert_eq!(report.statistics.log_levels["ALERT"], 1);
    assert!(
        report
            .recommendations
            .iter()
            .any(|text| text.contains("High error rate"))
    );
}

#[test]
fn web_error_analysis_uses_critical_header_levels() {
    let mut nginx = LogAnalyzer::new("nginx_error").unwrap();
    nginx.observe("2024/01/01 00:00:00 [crit] 1234#5: *6 INFO in message, client: 192.0.2.1, server: example.test, request: \"GET / HTTP/1.1\", host: \"example.test\"");
    nginx.observe("2024/01/01 00:00:01 [emerg] 1234#5: *6 INFO in message, client: 192.0.2.1, server: example.test, request: \"GET / HTTP/1.1\", host: \"example.test\"");
    let stats = nginx.statistics();
    assert_eq!(stats.valid_logs, 2);
    assert_eq!(stats.log_levels.get("CRITICAL"), Some(&1));
    assert_eq!(stats.log_levels.get("EMERGENCY"), Some(&1));
    assert!(!stats.log_levels.contains_key("INFO"));
    let mut apache = LogAnalyzer::new("apache_error").unwrap();
    apache.observe("[Mon Jan 01 00:00:00.000000 2024] [alert] [core:error] [pid 1234] [client 192.0.2.1] INFO in message");
    let stats = apache.statistics();
    assert_eq!(stats.valid_logs, 1);
    assert_eq!(stats.log_levels.get("ALERT"), Some(&1));
    assert!(!stats.log_levels.contains_key("ERROR"));
}

#[test]
fn web_error_stratification_uses_header_before_embedded_status() {
    let records: Vec<String> = [
        "2024/01/01 00:00:00 [info] 1234#1: *1 ok, client: 192.0.2.1, server: example.test, request: \"GET / HTTP/1.1\", host: \"example.test\" \"GET / HTTP/1.1\" 200",
        "2024/01/01 00:00:01 [info] 1234#1: *2 ok, client: 192.0.2.1, server: example.test, request: \"GET / HTTP/1.1\", host: \"example.test\" \"GET / HTTP/1.1\" 200",
        "2024/01/01 00:00:02 [crit] 1234#1: *3 fail, client: 192.0.2.1, server: example.test, request: \"GET / HTTP/1.1\", host: \"example.test\" \"GET / HTTP/1.1\" 200",
        "2024/01/01 00:00:03 [crit] 1234#1: *4 fail, client: 192.0.2.1, server: example.test, request: \"GET / HTTP/1.1\", host: \"example.test\" \"GET / HTTP/1.1\" 200",
    ].into_iter().map(str::to_owned).collect();
    for seed in 0..20 {
        let sample = generate_samples(
            &records,
            "nginx_error",
            2,
            SamplingStrategy::Stratified,
            seed,
        )
        .unwrap();
        assert!(
            sample.logs.iter().any(|line| line.contains("[info]")),
            "seed={seed}"
        );
        assert!(
            sample.logs.iter().any(|line| line.contains("[crit]")),
            "seed={seed}"
        );
    }
}

#[test]
fn docker_analysis_uses_level_metadata_without_guessing_from_body() {
    let mut analyzer = LogAnalyzer::new("docker").unwrap();
    analyzer.observe(r#"{"log":"ERROR in plain container output\n","stream":"stderr","time":"2024-01-01T00:00:00.000Z"}"#);
    analyzer.observe("2024-01-01T00:00:01.000Z [INFO] app: ERROR in message");
    analyzer.observe(r#"{"MESSAGE":"INFO in message","PRIORITY":3,"CONTAINER_NAME":"app","CONTAINER_ID":"abc","CONTAINER_ID_FULL":"abc","SYSLOG_IDENTIFIER":"app","__REALTIME_TIMESTAMP":"1704067200000000"}"#);
    let stats = analyzer.statistics();
    assert_eq!(stats.valid_logs, 3);
    assert_eq!(stats.log_levels.get("INFO"), Some(&1));
    assert_eq!(stats.log_levels.get("ERROR"), Some(&1));
}

#[test]
fn docker_stratified_sampling_uses_level_not_embedded_http_status() {
    let records: Vec<String> = [
        "2024-01-01T00:00:00.000Z [INFO] app: \"GET / HTTP/1.1\" 200 ERROR in message",
        "2024-01-01T00:00:01.000Z [INFO] app: \"GET / HTTP/1.1\" 200 ERROR in message",
        "2024-01-01T00:00:02.000Z [ERROR] app: \"GET / HTTP/1.1\" 200 INFO in message",
        "2024-01-01T00:00:03.000Z [ERROR] app: \"GET / HTTP/1.1\" 200 INFO in message",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for seed in 0..20 {
        let sample =
            generate_samples(&records, "docker", 2, SamplingStrategy::Stratified, seed).unwrap();
        assert!(
            sample.logs.iter().any(|line| line.contains("[INFO]")),
            "seed={seed}"
        );
        assert!(
            sample.logs.iter().any(|line| line.contains("[ERROR]")),
            "seed={seed}"
        );
    }
}
