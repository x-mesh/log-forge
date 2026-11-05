use chrono::{TimeZone, Utc};
use log_forge::{LogGenerator, error_library::ErrorLibrary};
use std::{fs, process::Command};

#[test]
fn rejects_invalid_context_option_before_generation() {
    let mut library = ErrorLibrary::from_sources(None, None, None, 17).unwrap();
    let mut choices = vec![serde_json::json!("valid"); 100];
    choices.clear();
    let pattern = serde_json::from_value(serde_json::json!({"id":"invalid_choice", "category":"custom", "severity":"error", "message_template":"{value}", "context_fields":{"value":choices}})).unwrap();
    assert!(library.add_pattern(pattern).is_err());
}

#[test]
fn error_pattern_renders_escaped_braces_and_repeated_fields() {
    let mut library = ErrorLibrary::from_sources(None, None, None, 17).unwrap();
    let pattern = serde_json::from_value(serde_json::json!({"id":"json_event", "category":"custom", "severity":"error", "message_template":r#"{{"service":"{service}","copy":"{service}"}}"#, "context_fields":{"service":["api"]}})).unwrap();
    library.add_pattern(pattern).unwrap();
    let output = library
        .generate_message("json_event", &std::collections::BTreeMap::new())
        .unwrap();
    assert_eq!(output, r#"{"service":"api","copy":"api"}"#);
}

#[test]
fn error_pattern_uses_original_unknown_context_default() {
    let mut library = ErrorLibrary::from_sources(None, None, None, 17).unwrap();
    let pattern = serde_json::from_value(serde_json::json!({"id":"context_default", "category":"custom", "severity":"error", "message_template":"service={service}", "context_fields":{"service":{}}})).unwrap();
    library.add_pattern(pattern).unwrap();
    assert_eq!(
        library
            .generate_message("context_default", &std::collections::BTreeMap::new())
            .unwrap(),
        "service=unknown"
    );
}

#[test]
fn error_pattern_formats_boolean_context_like_original() {
    let mut library = ErrorLibrary::from_sources(None, None, None, 17).unwrap();
    let pattern = serde_json::from_value(serde_json::json!({"id":"boolean_context", "category":"custom", "severity":"error", "message_template":"enabled={enabled} failed={failed}", "context_fields":{"enabled":true,"failed":[false]}})).unwrap();
    library.add_pattern(pattern).unwrap();
    assert_eq!(
        library
            .generate_message("boolean_context", &std::collections::BTreeMap::new())
            .unwrap(),
        "enabled=True failed=False"
    );
}

#[test]
fn error_pattern_formats_null_context_like_original() {
    let mut library = ErrorLibrary::from_sources(None, None, None, 17).unwrap();
    let pattern = serde_json::from_value(serde_json::json!({"id":"null_context", "category":"custom", "severity":"error", "message_template":"a={a} b={b} c={c}", "context_fields":{"a":null,"b":[null],"c":{"default":null}}})).unwrap();
    library.add_pattern(pattern).unwrap();
    assert_eq!(
        library
            .generate_message("null_context", &std::collections::BTreeMap::new())
            .unwrap(),
        "a=None b=None c=None"
    );
}

#[test]
fn error_pattern_formats_nested_context_like_original() {
    let mut library = ErrorLibrary::from_sources(None, None, None, 17).unwrap();
    let pattern = serde_json::from_value(serde_json::json!({"id":"nested_context", "category":"custom", "severity":"error", "message_template":"{items} {details}", "context_fields":{"items":[["api",null,true]],"details":[{"id":1}]}})).unwrap();
    library.add_pattern(pattern).unwrap();
    assert_eq!(
        library
            .generate_message("nested_context", &std::collections::BTreeMap::new())
            .unwrap(),
        "['api', None, True] {'id': 1}"
    );
}

#[test]
fn nested_context_escapes_unicode_line_separators() {
    let mut library = ErrorLibrary::from_sources(None, None, None, 17).unwrap();
    let pattern = serde_json::from_value(serde_json::json!({"id":"separator", "category":"custom", "severity":"error", "message_template":"{items}", "context_fields":{"items":[["a\u{2028}b", "c\u{2029}d"]]}})).unwrap();
    library.add_pattern(pattern).unwrap();
    assert_eq!(
        library
            .generate_message("separator", &std::collections::BTreeMap::new())
            .unwrap(),
        r"['a\u2028b', 'c\u2029d']"
    );
}

#[test]
fn error_pattern_accepts_fields_supplied_at_generation_time() {
    let mut library = ErrorLibrary::from_sources(None, None, None, 17).unwrap();
    let pattern = serde_json::from_value(serde_json::json!({"id":"request_failure", "category":"custom", "severity":"error", "message_template":"request {request_id} failed"})).unwrap();
    library.add_pattern_with_external_context(pattern).unwrap();
    assert_eq!(
        library.required_context_fields("request_failure").unwrap(),
        ["request_id"]
    );
    assert!(library.required_context_fields("missing").is_err());
    let overrides = std::collections::BTreeMap::from([("request_id".into(), "abc{123}".into())]);
    assert_eq!(
        library
            .generate_message("request_failure", &overrides)
            .unwrap(),
        "request abc{123} failed"
    );
    assert!(
        library
            .generate_message("request_failure", &std::collections::BTreeMap::new())
            .is_err()
    );
    let root = tempfile::tempdir().unwrap();
    library.export(&root.path().join("patterns.json")).unwrap();
    assert!(ErrorLibrary::from_sources(Some(root.path()), Some("custom"), None, 17).is_err());
    let mut loaded = ErrorLibrary::from_sources_with_external_context(
        Some(root.path()),
        Some("custom"),
        None,
        17,
    )
    .unwrap();
    assert_eq!(
        loaded.required_context_fields("request_failure").unwrap(),
        ["request_id"]
    );
    assert_eq!(
        loaded
            .generate_message("request_failure", &overrides)
            .unwrap(),
        "request abc{123} failed"
    );
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    assert!(
        loaded
            .generate_filtered(time, Some("custom"), None)
            .is_err()
    );
    assert!(loaded.pattern_usage("request_failure").is_none());
    let output = loaded
        .generate_filtered_with_context(time, Some("custom"), None, &overrides)
        .unwrap();
    assert!(output.contains("request abc{123} failed"));
    assert_eq!(
        loaded.pattern_usage("request_failure").unwrap().total_usage,
        1
    );
    let later = time + chrono::Duration::minutes(5);
    let frequency = log_forge::frequency::ErrorFrequencyConfig::default();
    assert!(
        loaded
            .generate_adjusted(later, Some("custom"), None, &frequency)
            .is_err()
    );
    assert_eq!(
        loaded.pattern_usage("request_failure").unwrap().total_usage,
        1
    );
    let output = loaded
        .generate_adjusted_with_context(later, Some("custom"), None, &frequency, &overrides)
        .unwrap();
    assert!(output.contains("request abc{123} failed"));
    assert_eq!(
        loaded.pattern_usage("request_failure").unwrap().total_usage,
        2
    );
    let messages = loaded
        .generate_scenario_with_context("custom", 3, &overrides)
        .unwrap();
    assert_eq!(messages.len(), 3);
    assert!(
        messages
            .iter()
            .all(|(id, message)| id == "request_failure" && message == "request abc{123} failed")
    );
    assert_eq!(
        loaded.pattern_usage("request_failure").unwrap().total_usage,
        2
    );
}

#[test]
fn weight_change_history_records_only_successful_updates() {
    let mut library = ErrorLibrary::from_sources(None, None, None, 7).unwrap();
    let id = library.pattern_ids().next().unwrap().to_owned();
    let old = library.get_pattern(&id).unwrap().frequency_weight;
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    library
        .update_weight_with_reason(&id, old + 1.0, "load test", time)
        .unwrap();
    assert!(
        library
            .update_weight_with_reason(&id, f64::NAN, "invalid", time)
            .is_err()
    );
    let changes = library.weight_changes(&id);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].old_weight, old);
    assert_eq!(changes[0].new_weight, old + 1.0);
    assert_eq!(changes[0].reason, "load test");
    assert_eq!(changes[0].timestamp, time.to_rfc3339());
    assert!(library.weight_changes("missing").is_empty());
    let category = library.get_pattern(&id).unwrap().category.clone();
    let severity = library.get_pattern(&id).unwrap().severity.clone();
    library
        .generate_filtered(time, Some(&category), Some(&severity))
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("analytics.json");
    library.export_analytics(&path).unwrap();
    let data: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        data["pattern_stats"][&id]["weight_changes"][0]["old_weight"],
        old
    );
    assert_eq!(
        data["pattern_stats"][&id]["weight_changes"][0]["reason"],
        "load test"
    );
    let total: u64 = data["pattern_stats"]
        .as_object()
        .unwrap()
        .values()
        .map(|value| value["total_usage"].as_u64().unwrap())
        .sum();
    assert_eq!(total, 1);
    assert!(library.export_analytics(&path).is_err());
}

#[test]
fn replacing_pattern_resets_old_usage_and_weight_changes() {
    let mut library = ErrorLibrary::from_sources(None, None, None, 7).unwrap();
    let id = library.pattern_ids().next().unwrap().to_owned();
    let original = library.get_pattern(&id).unwrap().clone();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    library
        .generate_filtered(time, Some(&original.category), Some(&original.severity))
        .unwrap();
    let used = library
        .pattern_ids()
        .find(|id| library.pattern_usage(id).is_some())
        .unwrap()
        .to_owned();
    let mut replacement = library.get_pattern(&used).unwrap().clone();
    library
        .update_weight_with_reason(&used, replacement.frequency_weight + 1.0, "test", time)
        .unwrap();
    let mut invalid = replacement.clone();
    invalid.frequency_weight = f64::NAN;
    assert!(library.add_pattern(invalid).is_err());
    assert!(library.pattern_usage(&used).is_some());
    assert_eq!(library.weight_changes(&used).len(), 1);
    replacement.message_template = "replacement".into();
    replacement.context_fields.clear();
    library.add_pattern(replacement).unwrap();
    assert!(library.pattern_usage(&used).is_none());
    assert!(library.weight_changes(&used).is_empty());
}

#[test]
fn adjusted_weights_use_recent_activity_and_peak_hours() {
    use log_forge::frequency::ErrorFrequencyConfig;
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("patterns.json"), serde_json::to_vec(&serde_json::json!([
        {"id":"critical_one", "category":"custom", "severity":"critical", "frequency_weight":2.0, "message_template":"critical"},
        {"id":"error_one", "category":"custom", "severity":"error", "frequency_weight":1.0, "message_template":"error"}
    ])).unwrap()).unwrap();
    let mut library =
        ErrorLibrary::from_sources(Some(root.path()), Some("custom"), None, 7).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
    let frequency = ErrorFrequencyConfig::default();
    let before = library
        .adjusted_weights(time, Some("custom"), None, &frequency)
        .unwrap();
    assert_eq!(before["critical_one"], 1.0);
    library
        .generate_filtered(time, Some("custom"), Some("error"))
        .unwrap();
    let submillisecond = library
        .adjusted_weights(
            time + chrono::Duration::microseconds(500),
            Some("custom"),
            Some("error"),
            &frequency,
        )
        .unwrap();
    assert!(submillisecond["error_one"] > 0.0);
    let after = library
        .adjusted_weights(
            time + chrono::Duration::minutes(5),
            Some("custom"),
            None,
            &frequency,
        )
        .unwrap();
    assert_eq!(after["error_one"], 0.5);
    assert_eq!(after["critical_one"], 1.0);
    assert!(
        library
            .adjusted_weights(
                time - chrono::Duration::seconds(1),
                Some("custom"),
                None,
                &frequency
            )
            .is_err()
    );
    let record = library
        .generate_adjusted(
            time + chrono::Duration::minutes(5),
            Some("custom"),
            Some("critical"),
            &frequency,
        )
        .unwrap();
    assert!(record.contains("pattern=critical_one"));
    assert_eq!(
        library.pattern_usage("critical_one").unwrap().total_usage,
        1
    );
}

#[test]
fn filtered_generation_tracks_usage_per_selected_pattern() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("patterns.json"), serde_json::to_vec(&serde_json::json!([
        {"id":"warning_one", "category":"custom", "severity":"warning", "message_template":"warning"},
        {"id":"error_one", "category":"custom", "severity":"error", "message_template":"error"}
    ])).unwrap()).unwrap();
    let mut library =
        ErrorLibrary::from_sources(Some(root.path()), Some("custom"), None, 7).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let record = library
        .generate_filtered(time, Some("custom"), Some("error"))
        .unwrap();
    assert!(record.contains("pattern=error_one"));
    assert_eq!(library.pattern_usage("error_one").unwrap().total_usage, 1);
    assert_eq!(
        library.pattern_usage("error_one").unwrap().last_used,
        Some(time)
    );
    assert!(library.pattern_usage("warning_one").is_none());
    assert!(
        library
            .generate_filtered(time, Some("missing"), None)
            .is_err()
    );
}

#[test]
fn selects_weighted_custom_patterns_and_applies_filters() {
    let root = tempfile::tempdir().unwrap();
    let patterns = serde_json::json!([
        {"id":"active", "category":"custom", "severity":"warning", "frequency_weight": 1.0, "message_template":"Service {service} used {count}", "context_fields":{"service":["api"], "count":{"min_val": 3,"max_val":3}}},
        {"id":"disabled", "category":"custom", "severity":"warning", "frequency_weight": 0.0, "message_template":"disabled"},
        {"id":"error", "category":"custom", "severity":"error", "frequency_weight": 1.0, "message_template":"different severity"}
    ]);
    fs::write(
        root.path().join("patterns.json"),
        serde_json::to_vec(&patterns).unwrap(),
    )
    .unwrap();
    let mut library =
        ErrorLibrary::from_sources(Some(root.path()), Some("custom"), Some("warning"), 7).unwrap();
    assert_eq!(
        library.pattern_ids().collect::<Vec<_>>(),
        vec!["active", "disabled"]
    );
    let now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    for _ in 0..20 {
        let record = library.generate(now).unwrap();
        assert!(record.contains("[warning] category=custom pattern=active Service api used 3"));
        assert!(library.validate(&record));
    }
    let usage = library.pattern_usage("active").unwrap();
    assert_eq!(usage.total_usage, 20);
    assert_eq!(usage.last_used, Some(now));
    assert!(library.pattern_usage("disabled").is_none());
    let stats = library.statistics();
    assert_eq!(stats.total_patterns, 2);
    assert_eq!(stats.categories["custom"], 2);
    assert_eq!(stats.severity_distribution["warning"], 2);
    assert_eq!(stats.average_weight, 0.5);
    assert!(ErrorLibrary::from_sources(Some(root.path()), Some("missing"), None, 7).is_err());
    let config = root.path().join("config.yaml");
    fs::write(&config, format!("log_generator:\n  global:\n    output_format: console\n    total_logs: 10\n    start_time: '2024-01-01'\n  log_types:\n    error_library:\n      enabled: true\n      patterns_dir: '{}'\n      error_category: custom\n      error_severity: warning\n", root.path().display())).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--config",
                config.to_str().unwrap(),
                "--seed",
                "7",
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
    assert_eq!(logs.lines().count(), 10);
    let validator = log_forge::validation::LogValidator::new("error_library").unwrap();
    assert!(
        logs.lines()
            .all(|line| validator.validate(line).is_valid && line.contains("pattern=active"))
    );
}

#[test]
fn preserves_metadata_updates_weights_and_exports_patterns() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fs::create_dir(&source).unwrap();
    let patterns = serde_json::json!([
        {"id":"first", "category":"custom", "severity":"warning", "description":"First error", "real_world_examples":["example"], "created_at":"2025-01-01T00:00:00", "frequency_weight":1.0, "message_template":"first"},
        {"id":"second", "category":"custom", "severity":"warning", "frequency_weight":0.0, "message_template":"second"}
    ]);
    fs::write(
        source.join("patterns.json"),
        serde_json::to_vec(&patterns).unwrap(),
    )
    .unwrap();
    let mut library = ErrorLibrary::from_sources(Some(&source), Some("custom"), None, 9).unwrap();
    let pattern = library.get_pattern("first").unwrap();
    assert_eq!(pattern.description, "First error");
    assert_eq!(pattern.real_world_examples, ["example"]);
    assert_eq!(pattern.created_at.as_deref(), Some("2025-01-01T00:00:00"));
    assert_eq!(library.categories(), ["custom"]);
    assert_eq!(library.severities(), ["warning"]);
    assert_eq!(library.patterns_by_category("custom").count(), 2);
    assert_eq!(library.patterns_by_severity("error").count(), 0);
    assert!(library.update_weight("first", 0.0).is_err());
    assert_eq!(library.get_pattern("first").unwrap().frequency_weight, 1.0);
    library.update_weight("second", 1.0).unwrap();
    library.update_weight("first", 0.0).unwrap();
    assert!(library.update_weight("second", f64::NAN).is_err());
    library.update_weight("first", 1.0e308).unwrap();
    assert!(library.update_weight("second", 1.0e308).is_err());
    assert_eq!(library.get_pattern("second").unwrap().frequency_weight, 1.0);
    library.update_weight("first", 0.0).unwrap();
    let now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    assert!(library.generate(now).unwrap().contains("pattern=second"));
    let export_dir = root.path().join("export");
    fs::create_dir(&export_dir).unwrap();
    let export = export_dir.join("patterns.json");
    library.export(&export).unwrap();
    assert!(library.export(&export).is_err());
    let reloaded = ErrorLibrary::from_sources(Some(&export_dir), Some("custom"), None, 9).unwrap();
    let pattern = reloaded.get_pattern("first").unwrap();
    assert_eq!(pattern.description, "First error");
    assert_eq!(pattern.real_world_examples, ["example"]);
    assert_eq!(pattern.frequency_weight, 0.0);
}

#[test]
fn adds_error_patterns_and_generates_with_custom_context() {
    use std::collections::BTreeMap;
    let mut library = ErrorLibrary::from_sources(None, None, None, 17).unwrap();
    let pattern = serde_json::from_value(serde_json::json!({
        "id":"custom_timeout", "category":"custom", "severity":"error",
        "message_template":"{service} failed after {duration}ms", "frequency_weight":1.0,
        "context_fields":{"service":["api"], "duration":{"min_val":10,"max_val":10}}
    }))
    .unwrap();
    library.add_pattern(pattern).unwrap();
    let context = BTreeMap::from([
        ("service".into(), "payments".into()),
        ("duration".into(), "50".into()),
    ]);
    assert_eq!(
        library
            .generate_message("custom_timeout", &context)
            .unwrap(),
        "payments failed after 50ms"
    );
    assert_eq!(
        library
            .generate_message("custom_timeout", &BTreeMap::new())
            .unwrap(),
        "api failed after 10ms"
    );
    assert!(library.generate_message("missing", &context).is_err());
    let invalid = serde_json::from_value(serde_json::json!({"id":"custom_timeout", "category":"custom", "severity":"error", "message_template":"{missing}"})).unwrap();
    assert!(library.add_pattern(invalid).is_err());
    assert_eq!(
        library
            .generate_message("custom_timeout", &context)
            .unwrap(),
        "payments failed after 50ms"
    );
}

#[test]
fn dynamically_filters_patterns_and_generates_reproducible_scenarios() {
    let root = tempfile::tempdir().unwrap();
    let patterns = serde_json::json!([
        {"id":"active", "category":"custom", "severity":"warning", "frequency_weight":1.0, "message_template":"warning {service}", "context_fields":{"service":["api"]}},
        {"id":"disabled", "category":"custom", "severity":"warning", "frequency_weight":0.0, "message_template":"disabled"},
        {"id":"fatal", "category":"custom", "severity":"error", "frequency_weight":1.0, "message_template":"error"}
    ]);
    fs::write(
        root.path().join("patterns.json"),
        serde_json::to_vec(&patterns).unwrap(),
    )
    .unwrap();
    let make = || ErrorLibrary::from_sources(Some(root.path()), Some("custom"), None, 7).unwrap();
    let mut library = make();
    for _ in 0..20 {
        assert_eq!(
            library
                .select_pattern(Some("custom"), Some("warning"))
                .unwrap()
                .id,
            "active"
        );
    }
    assert!(library.select_pattern(Some("missing"), None).is_err());
    assert!(library.generate_scenario("missing", 5).unwrap().is_empty());
    assert!(library.generate_scenario("custom", 0).unwrap().is_empty());
    library.update_weight("fatal", 0.0).unwrap();
    assert!(
        library
            .select_pattern(Some("custom"), Some("error"))
            .is_err()
    );
    let first = make().generate_scenario("custom", 30).unwrap();
    let second = make().generate_scenario("custom", 30).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.len(), 30);
    assert!(first.iter().all(|(id, _)| id == "active" || id == "fatal"));
    assert!(first.iter().any(|(id, _)| id == "active"));
    assert!(first.iter().any(|(id, _)| id == "fatal"));
}

#[test]
fn failed_adjusted_generation_preserves_random_state() {
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let frequency = log_forge::frequency::ErrorFrequencyConfig::default();
    for seed in 0..20 {
        let mut failed = ErrorLibrary::from_sources(None, None, None, seed).unwrap();
        let mut fresh = ErrorLibrary::from_sources(None, None, None, seed).unwrap();
        let pattern = serde_json::from_value(serde_json::json!({"id":"random_external", "category":"isolated_random", "severity":"error", "message_template":"{value} {external}", "context_fields":{"value":["a","b","c","d","e","f","g","h"]}})).unwrap();
        failed.add_pattern_with_external_context(pattern).unwrap();
        let pattern = failed.get_pattern("random_external").unwrap().clone();
        fresh.add_pattern_with_external_context(pattern).unwrap();
        assert!(
            failed
                .generate_adjusted(time, Some("isolated_random"), None, &frequency)
                .is_err()
        );
        let context = std::collections::BTreeMap::from([("external".into(), "ok".into())]);
        assert_eq!(
            failed
                .generate_adjusted_with_context(
                    time,
                    Some("isolated_random"),
                    None,
                    &frequency,
                    &context
                )
                .unwrap(),
            fresh
                .generate_adjusted_with_context(
                    time,
                    Some("isolated_random"),
                    None,
                    &frequency,
                    &context
                )
                .unwrap()
        );
    }
}

#[test]
fn failed_generation_preserves_random_state_across_library_apis() {
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    for seed in 0..20 {
        for method in 0..3 {
            let mut failed = ErrorLibrary::from_sources(None, None, None, seed).unwrap();
            let mut fresh = ErrorLibrary::from_sources(None, None, None, seed).unwrap();
            let pattern = serde_json::from_value(serde_json::json!({"id":"random_external", "category":"isolated_random", "severity":"error", "message_template":"{value} {external}", "context_fields":{"value":["a","b","c","d","e","f","g","h"]}})).unwrap();
            failed.add_pattern_with_external_context(pattern).unwrap();
            fresh
                .add_pattern_with_external_context(
                    failed.get_pattern("random_external").unwrap().clone(),
                )
                .unwrap();
            let empty = std::collections::BTreeMap::new();
            let context = std::collections::BTreeMap::from([("external".into(), "ok".into())]);
            match method {
                0 => {
                    assert!(failed.generate_message("random_external", &empty).is_err());
                    assert_eq!(
                        failed
                            .generate_message("random_external", &context)
                            .unwrap(),
                        fresh.generate_message("random_external", &context).unwrap()
                    );
                }
                1 => {
                    assert!(
                        failed
                            .generate_filtered(time, Some("isolated_random"), None)
                            .is_err()
                    );
                    assert_eq!(
                        failed
                            .generate_filtered_with_context(
                                time,
                                Some("isolated_random"),
                                None,
                                &context
                            )
                            .unwrap(),
                        fresh
                            .generate_filtered_with_context(
                                time,
                                Some("isolated_random"),
                                None,
                                &context
                            )
                            .unwrap()
                    );
                }
                _ => {
                    assert!(failed.generate_scenario("isolated_random", 3).is_err());
                    assert_eq!(
                        failed
                            .generate_scenario_with_context("isolated_random", 3, &context)
                            .unwrap(),
                        fresh
                            .generate_scenario_with_context("isolated_random", 3, &context)
                            .unwrap()
                    );
                }
            }
        }
    }
}
