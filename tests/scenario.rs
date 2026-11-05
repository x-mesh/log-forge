use chrono::{TimeZone, Utc};
use log_forge::{
    LogGenerator,
    scenario::{ScenarioGenerator, ScenarioManager},
};

#[test]
fn scenario_can_be_created_from_an_in_memory_definition() {
    let definition = serde_json::json!({"name":"inline", "description":"test scenario", "patterns":["nginx_timeout"], "sequence_rules":{}, "duration_minutes":0.5});
    let generator = ScenarioGenerator::from_definition(definition.clone(), None, 5).unwrap();
    assert_eq!(generator.definition().unwrap(), definition);
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("inline.json");
    generator.save_definition(&path).unwrap();
    assert!(generator.save_definition(&path).is_err());
    assert_eq!(
        ScenarioGenerator::from_sources(&path, None, 5)
            .unwrap()
            .definition()
            .unwrap(),
        definition
    );
    let mut manager = ScenarioManager::new();
    manager.register(generator).unwrap();
    manager.start("inline", 5).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    assert!(
        manager
            .generate("inline", time)
            .unwrap()
            .contains("pattern=nginx_timeout")
    );
    let invalid =
        serde_json::json!({"name":"bad", "patterns":["missing_pattern"], "sequence_rules":{}});
    assert!(ScenarioGenerator::from_definition(invalid, None, 5).is_err());
    let empty_name =
        serde_json::json!({"name":"", "patterns":["nginx_timeout"], "sequence_rules":{}});
    assert!(ScenarioGenerator::from_definition(empty_name, None, 5).is_err());
}

#[test]
fn generated_scenario_records_are_accepted_by_shared_validator() {
    let definition = serde_json::json!({"name":"service outage", "patterns":["nginx_timeout"], "sequence_rules":{}});
    let mut generator = ScenarioGenerator::from_definition(definition, None, 5).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let record = generator.generate(time).unwrap();
    let validator = log_forge::validation::LogValidator::new("scenario").unwrap();
    assert!(validator.validate(&record).is_valid, "{record}");
    assert!(!validator.validate("not a scenario record").is_valid);
}

#[test]
fn scenario_definition_preserves_original_metadata_and_sequence_rules() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("error_scenarios/database_overload.json");
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let generator = ScenarioGenerator::from_sources(&path, None, 0).unwrap();
    assert_eq!(generator.definition().unwrap(), expected);
}

#[test]
fn scenario_rejects_invalid_pattern_options_before_generation() {
    let root = tempfile::tempdir().unwrap();
    let scenario = root.path().join("scenario.json");
    std::fs::write(
        &scenario,
        r#"{"name":"test","patterns":["sample"],"sequence_rules":{}}"#,
    )
    .unwrap();
    let mut options = vec![serde_json::json!("valid"); 100];
    options.clear();
    let pattern = serde_json::json!({"id":"sample", "category":"test", "severity":"error", "message_template":"{value}", "context_fields":{"value":options}});
    std::fs::write(
        root.path().join("sample.json"),
        serde_json::to_vec(&pattern).unwrap(),
    )
    .unwrap();
    assert!(ScenarioGenerator::from_files(&scenario, root.path(), 0).is_err());
}

#[test]
fn scenario_rejects_missing_declared_pattern_even_with_valid_chain() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("scenario.json");
    std::fs::write(&path, r#"{"name":"test","patterns":["missing_pattern"],"sequence_rules":{"escalation_chain":["nginx_timeout"],"escalation_delay_minutes":[]}}"#).unwrap();
    assert!(ScenarioGenerator::from_sources(&path, None, 0).is_err());
}

#[test]
fn scenario_without_escalation_delays_uses_regular_patterns() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("scenario.json");
    std::fs::write(&path, r#"{"name":"test","patterns":["mysql_connection_lost"],"sequence_rules":{"escalation_chain":["nginx_timeout"]}}"#).unwrap();
    let mut generator = ScenarioGenerator::from_sources(&path, None, 0).unwrap();
    let output = generator
        .generate(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap())
        .unwrap();
    assert!(output.contains("pattern=mysql_connection_lost"), "{output}");
    std::fs::write(&path, r#"{"name":"test","patterns":["mysql_connection_lost"],"sequence_rules":{"escalation_chain":["nginx_timeout"],"escalation_delay_minutes":[]}}"#).unwrap();
    let mut generator = ScenarioGenerator::from_sources(&path, None, 0).unwrap();
    let output = generator
        .generate(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap())
        .unwrap();
    assert!(output.contains("pattern=nginx_timeout"), "{output}");
}

#[test]
fn scenario_definition_preserves_absent_sequence_fields() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("scenario.json");
    std::fs::write(&path, r#"{"name":"test","patterns":["mysql_connection_lost"],"sequence_rules":{},"duration_minutes":30}"#).unwrap();
    let generator = ScenarioGenerator::from_sources(&path, None, 0).unwrap();
    let definition = generator.definition().unwrap();
    assert_eq!(definition["sequence_rules"], serde_json::json!({}));
    std::fs::write(&path, r#"{"name":"test","patterns":["mysql_connection_lost"],"sequence_rules":{"escalation_chain":[]},"duration_minutes":30}"#).unwrap();
    let generator = ScenarioGenerator::from_sources(&path, None, 0).unwrap();
    assert_eq!(
        generator.definition().unwrap()["sequence_rules"],
        serde_json::json!({"escalation_chain":[]})
    );
}

#[test]
fn export_includes_definitions_and_usage_without_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("error_scenarios/web_server_cascade.json");
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut manager = ScenarioManager::new();
    manager
        .register(ScenarioGenerator::from_sources(&path, None, 1).unwrap())
        .unwrap();
    manager.start("web_server_cascade", 42).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    manager.generate("web_server_cascade", time).unwrap();
    let destination = root.path().join("export.json");
    manager.export_configuration(&destination).unwrap();
    let data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&destination).unwrap()).unwrap();
    assert_eq!(data["scenarios"]["web_server_cascade"], expected);
    assert_eq!(data["pattern_stats"]["nginx_timeout"]["usage_count"], 1);
    assert_eq!(
        data["pattern_stats"]["nginx_timeout"]["escalation_count"],
        1
    );
    assert_eq!(
        data["pattern_stats"]["nginx_timeout"]["last_used"],
        time.to_rfc3339()
    );
    assert!(manager.export_configuration(&destination).is_err());
}

#[test]
fn configured_frequency_controls_generation_and_export() {
    use log_forge::frequency::ErrorFrequencyConfig;
    let root = tempfile::tempdir().unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("error_scenarios/web_server_cascade.json");
    let mut manager = ScenarioManager::new();
    manager
        .register(ScenarioGenerator::from_sources(&path, None, 1).unwrap())
        .unwrap();
    manager.start("web_server_cascade", 42).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    assert!(manager.generate_if_due("web_server_cascade", time).is_err());
    let mut config = ErrorFrequencyConfig {
        base_frequency: 0.0,
        burst_probability: 0.0,
        ..Default::default()
    };
    manager.set_frequency(config.clone(), 2).unwrap();
    assert!(
        manager
            .generate_if_due("web_server_cascade", time)
            .unwrap()
            .is_none()
    );
    config.base_frequency = 60.0;
    manager.set_frequency(config, 2).unwrap();
    assert!(
        manager
            .generate_if_due("web_server_cascade", time)
            .unwrap()
            .is_some()
    );
    let export = root.path().join("export.json");
    manager.export_configuration(&export).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(export).unwrap()).unwrap();
    assert_eq!(value["frequency_config"]["base_frequency"], 60.0);
    assert_eq!(value["pattern_stats"]["nginx_timeout"]["usage_count"], 1);
}

#[test]
fn frequency_control_rejects_time_before_scenario_start() {
    use log_forge::frequency::ErrorFrequencyConfig;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("error_scenarios/web_server_cascade.json");
    let mut manager = ScenarioManager::new();
    manager
        .register(ScenarioGenerator::from_sources(&path, None, 1).unwrap())
        .unwrap();
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    manager.start_at("web_server_cascade", 42, start).unwrap();
    manager
        .set_frequency(
            ErrorFrequencyConfig {
                base_frequency: 0.0,
                burst_probability: 0.0,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    assert!(
        manager
            .generate_if_due("web_server_cascade", start - chrono::Duration::seconds(1))
            .is_err()
    );
    assert!(
        manager
            .generate_if_due("web_server_cascade", start)
            .unwrap()
            .is_none()
    );
}

#[test]
fn prioritized_generation_checks_start_time_even_when_not_due() {
    use log_forge::{error_library::ErrorLibrary, frequency::ErrorFrequencyConfig};
    let mut manager = ScenarioManager::new();
    let definition =
        serde_json::json!({"name":"test","patterns":["nginx_timeout"],"sequence_rules":{}});
    manager
        .register(ScenarioGenerator::from_definition(definition, None, 0).unwrap())
        .unwrap();
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    manager.start_at("test", 0, start).unwrap();
    manager
        .set_frequency(
            ErrorFrequencyConfig {
                base_frequency: 0.0,
                burst_probability: 0.0,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    let earlier = start - chrono::Duration::seconds(1);
    assert!(manager.generate_first_active_if_due(earlier).is_err());
    let mut library = ErrorLibrary::from_sources(None, None, None, 0).unwrap();
    assert!(
        manager
            .generate_realistic_if_due(&mut library, earlier, None, None, true)
            .is_err()
    );
    assert!(
        manager
            .generate_first_active_if_due(start)
            .unwrap()
            .is_none()
    );
}

#[test]
fn scenario_health_counts_recent_synthetic_errors() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("error_scenarios/web_server_cascade.json");
    let mut manager = ScenarioManager::new();
    manager
        .register(ScenarioGenerator::from_sources(&path, None, 1).unwrap())
        .unwrap();
    manager.start("web_server_cascade", 42).unwrap();
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    for _ in 0..11 {
        manager.generate("web_server_cascade", start).unwrap();
    }
    let health = manager.health_at(start);
    assert_eq!(health.health_status, "degraded");
    assert_eq!(health.total_errors_last_hour, 11);
    assert_eq!(health.category_breakdown["web_server"], 11);
    assert_eq!(health.severity_breakdown["error"], 11);
    assert_eq!(health.active_scenarios, ["web_server_cascade"]);
    assert_eq!(
        health.recommendations,
        ["Continue monitoring the synthetic scenario"]
    );
    assert_eq!(
        manager
            .health_at(start - chrono::Duration::seconds(1))
            .total_errors_last_hour,
        0
    );
    assert_eq!(
        manager
            .health_at(start + chrono::Duration::hours(1))
            .total_errors_last_hour,
        0
    );
    manager.stop("web_server_cascade");
    assert!(manager.health_at(start).active_scenarios.is_empty());
    assert_eq!(manager.health_at(start).total_errors_last_hour, 11);
}

#[test]
fn scenario_health_recommends_review_for_repeated_synthetic_errors() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("error_scenarios/web_server_cascade.json");
    let mut manager = ScenarioManager::new();
    manager
        .register(ScenarioGenerator::from_sources(&path, None, 1).unwrap())
        .unwrap();
    manager.start("web_server_cascade", 42).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    for _ in 0..51 {
        manager.generate("web_server_cascade", time).unwrap();
    }
    let health = manager.health_at(time);
    assert_eq!(health.health_status, "critical");
    assert_eq!(health.recommendations.len(), 2);
    assert!(
        health
            .recommendations
            .iter()
            .any(|value| value.contains("upstream"))
    );
    assert!(serde_json::to_string(&health).is_ok());
}

#[test]
fn pattern_analytics_persist_across_scenario_restarts() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("error_scenarios/web_server_cascade.json");
    let mut manager = ScenarioManager::new();
    manager
        .register(ScenarioGenerator::from_sources(&path, None, 1).unwrap())
        .unwrap();
    let id = "nginx_timeout";
    assert_eq!(manager.pattern_analytics(id).unwrap().total_usage, 0);
    assert!(manager.pattern_analytics("missing").is_none());
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    for _ in 0..2 {
        manager.start("web_server_cascade", 42).unwrap();
        manager.generate("web_server_cascade", time).unwrap();
        manager.stop("web_server_cascade");
    }
    let analytics = manager.pattern_analytics(id).unwrap();
    assert_eq!(analytics.pattern.id, id);
    assert_eq!(analytics.total_usage, 2);
    assert_eq!(analytics.escalation_count, 2);
    assert_eq!(analytics.recent_usage, 2);
    assert_eq!(analytics.last_used, Some(time.to_rfc3339()));
    let value = analytics.to_dict();
    assert_eq!(value["pattern_info"]["id"], id);
    assert_eq!(value["usage_stats"]["total_usage"], 2);
    assert_eq!(value["usage_stats"]["escalation_count"], 2);
    assert!(serde_json::to_string(&analytics).is_ok());
}

#[test]
fn scenario_analytics_include_library_weight_history() {
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut library =
        log_forge::error_library::ErrorLibrary::from_sources(None, None, None, 1).unwrap();
    let old_weight = library
        .get_pattern("nginx_timeout")
        .unwrap()
        .frequency_weight;
    library
        .update_weight_with_reason("nginx_timeout", 2.0, "test", time)
        .unwrap();
    let manager = ScenarioManager::new();
    let analytics = manager
        .pattern_analytics_with_library(&library, "nginx_timeout")
        .unwrap()
        .unwrap();
    let value = analytics.to_dict();
    assert_eq!(value["pattern_info"]["frequency_weight"], 2.0);
    assert_eq!(value["effectiveness"]["avg_response_time"], 0.0);
    assert_eq!(
        value["effectiveness"]["weight_changes"][0]["old_weight"],
        old_weight
    );
    assert_eq!(
        value["effectiveness"]["weight_changes"][0]["reason"],
        "test"
    );
    assert!(
        manager
            .pattern_analytics_with_library(&library, "missing")
            .unwrap()
            .is_none()
    );
}

#[test]
fn scenario_manager_rejects_conflicting_definitions_for_one_pattern_id() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = ScenarioManager::new();
    for (name, category) in [("one", "web"), ("two", "database"), ("three", "web")] {
        let directory = root.path().join(name);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("shared.json"), format!(r#"{{"id":"shared","category":"{category}","severity":"error","message_template":"failure"}}"#)).unwrap();
        let scenario = directory.join("scenario.json");
        std::fs::write(
            &scenario,
            format!(r#"{{"name":"{name}","patterns":["shared"],"sequence_rules":{{}}}}"#),
        )
        .unwrap();
        let generator = ScenarioGenerator::from_files(&scenario, &directory, 0).unwrap();
        if name != "two" {
            manager.register(generator).unwrap();
        } else {
            assert!(manager.register(generator).is_err());
        }
    }
}

#[test]
fn manages_independent_scenarios_and_restarts_from_seed() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("error_scenarios");
    let mut manager = ScenarioManager::new();
    for name in ["database_overload", "web_server_cascade"] {
        manager
            .register(
                ScenarioGenerator::from_sources(&directory.join(format!("{name}.json")), None, 0)
                    .unwrap(),
            )
            .unwrap();
    }
    assert_eq!(
        manager.scenarios().collect::<Vec<_>>(),
        ["database_overload", "web_server_cascade"]
    );
    assert!(manager.start("missing", 42).is_err());
    manager.start("database_overload", 42).unwrap();
    manager.start("web_server_cascade", 7).unwrap();
    assert!(manager.start("database_overload", 42).is_err());
    assert_eq!(manager.active_scenarios().count(), 2);
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let first = manager.generate("database_overload", start).unwrap();
    assert!(
        manager
            .generate("web_server_cascade", start)
            .unwrap()
            .contains("nginx_timeout")
    );
    assert_eq!(
        manager.active("database_overload").unwrap().pattern_usage()["database_slow_query"].count,
        1
    );
    assert!(manager.stop("database_overload").is_some());
    assert!(manager.generate("database_overload", start).is_err());
    assert!(manager.stop("database_overload").is_none());
    manager.start("database_overload", 42).unwrap();
    assert_eq!(manager.generate("database_overload", start).unwrap(), first);
    assert_eq!(manager.active_scenarios().count(), 2);
}

#[test]
fn active_scenarios_follow_start_order_and_restart_moves_to_end() {
    let mut manager = ScenarioManager::new();
    for name in ["alpha", "zeta"] {
        let definition =
            serde_json::json!({"name":name,"patterns":["nginx_timeout"],"sequence_rules":{}});
        manager
            .register(ScenarioGenerator::from_definition(definition, None, 0).unwrap())
            .unwrap();
    }
    manager.start("zeta", 0).unwrap();
    manager.start("alpha", 0).unwrap();
    assert_eq!(
        manager.active_scenarios().collect::<Vec<_>>(),
        ["zeta", "alpha"]
    );
    manager.stop("zeta");
    manager.start("zeta", 0).unwrap();
    assert_eq!(
        manager.active_scenarios().collect::<Vec<_>>(),
        ["alpha", "zeta"]
    );
}

#[test]
fn first_active_generation_skips_expired_scenarios_in_start_order() {
    use log_forge::frequency::ErrorFrequencyConfig;
    let mut manager = ScenarioManager::new();
    for (name, id, duration) in [
        ("first", "nginx_timeout", 0.5),
        ("second", "nginx_connection_refused", 2.0),
    ] {
        let definition = serde_json::json!({"name":name,"patterns":[id],"sequence_rules":{},"duration_minutes":duration});
        manager
            .register(ScenarioGenerator::from_definition(definition, None, 0).unwrap())
            .unwrap();
    }
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    manager.start_at("first", 1, time).unwrap();
    manager.start_at("second", 2, time).unwrap();
    manager
        .set_frequency(
            ErrorFrequencyConfig {
                base_frequency: 60.0,
                burst_probability: 0.0,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    let output = manager
        .generate_first_active_if_due(time + chrono::Duration::minutes(1))
        .unwrap()
        .unwrap();
    assert!(output.contains("scenario=second pattern=nginx_connection_refused"));
    assert_eq!(manager.active_scenarios().collect::<Vec<_>>(), ["second"]);
    assert_eq!(
        manager
            .pattern_analytics("nginx_connection_refused")
            .unwrap()
            .total_usage,
        1
    );
}

#[test]
fn realistic_generation_uses_scenario_then_filtered_error_pattern() {
    use log_forge::{
        error_library::ErrorLibrary, frequency::ErrorFrequencyConfig, scenario::ManagedError,
    };
    let mut manager = ScenarioManager::new();
    let definition = serde_json::json!({"name":"first","patterns":["nginx_timeout"],"sequence_rules":{},"duration_minutes":0.5});
    manager
        .register(ScenarioGenerator::from_definition(definition, None, 0).unwrap())
        .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    manager.start_at("first", 1, time).unwrap();
    manager
        .set_frequency(
            ErrorFrequencyConfig {
                base_frequency: 60.0,
                burst_probability: 0.0,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    let mut library = ErrorLibrary::from_sources(None, None, None, 0).unwrap();
    assert!(matches!(
        manager
            .generate_realistic_if_due(&mut library, time, Some("missing"), None, true)
            .unwrap(),
        Some(ManagedError::Scenario(_))
    ));
    assert_eq!(
        library.pattern_usage("nginx_timeout").unwrap().total_usage,
        1
    );
    let weights = library
        .adjusted_weights(
            time,
            Some("web_server"),
            None,
            &ErrorFrequencyConfig::default(),
        )
        .unwrap();
    assert_eq!(weights["nginx_timeout"], 0.0);
    assert!(matches!(
        manager
            .generate_realistic_if_due(
                &mut library,
                time + chrono::Duration::minutes(1),
                Some("web_server"),
                None,
                true
            )
            .unwrap(),
        Some(ManagedError::Pattern(_))
    ));
    assert!(manager.active("first").is_none());
    let health = manager.health_at(time + chrono::Duration::minutes(1));
    assert_eq!(health.total_errors_last_hour, 2);
    assert_eq!(health.category_breakdown["web_server"], 2);
    let total: u64 = library
        .pattern_ids()
        .filter_map(|id| manager.pattern_analytics(id))
        .map(|value| value.total_usage)
        .sum();
    let escalations: u64 = library
        .pattern_ids()
        .filter_map(|id| manager.pattern_analytics(id))
        .map(|value| value.escalation_count)
        .sum();
    assert_eq!(total, 2);
    assert_eq!(escalations, 1);
    assert!(
        manager
            .generate_realistic_if_due(
                &mut library,
                time + chrono::Duration::minutes(2),
                Some("missing"),
                None,
                true
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn realistic_generation_rejects_library_mismatch_before_scenario_changes() {
    use log_forge::{error_library::ErrorLibrary, frequency::ErrorFrequencyConfig};
    let mut manager = ScenarioManager::new();
    let definition =
        serde_json::json!({"name":"test","patterns":["nginx_timeout"],"sequence_rules":{}});
    manager
        .register(ScenarioGenerator::from_definition(definition, None, 0).unwrap())
        .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    manager.start_at("test", 0, time).unwrap();
    manager
        .set_frequency(
            ErrorFrequencyConfig {
                base_frequency: 60.0,
                burst_probability: 0.0,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    let mut library = ErrorLibrary::from_sources(None, Some("database"), None, 0).unwrap();
    assert!(
        manager
            .generate_realistic_if_due(&mut library, time, None, None, true)
            .is_err()
    );
    assert_eq!(
        manager
            .pattern_analytics("nginx_timeout")
            .unwrap()
            .total_usage,
        0
    );
    assert!(manager.active("test").is_some());
}

#[test]
fn realistic_generation_skips_library_check_when_not_due() {
    use log_forge::{error_library::ErrorLibrary, frequency::ErrorFrequencyConfig};
    let mut manager = ScenarioManager::new();
    let definition =
        serde_json::json!({"name":"test","patterns":["nginx_timeout"],"sequence_rules":{}});
    manager
        .register(ScenarioGenerator::from_definition(definition, None, 0).unwrap())
        .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    manager.start_at("test", 0, time).unwrap();
    manager
        .set_frequency(
            ErrorFrequencyConfig {
                base_frequency: 0.0,
                burst_probability: 0.0,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    let mut library = ErrorLibrary::from_sources(None, Some("database"), None, 0).unwrap();
    assert!(
        manager
            .generate_realistic_if_due(&mut library, time, None, None, true)
            .unwrap()
            .is_none()
    );
}

#[test]
fn expired_scenario_does_not_block_regular_error_generation() {
    use log_forge::{
        error_library::ErrorLibrary, frequency::ErrorFrequencyConfig, scenario::ManagedError,
    };
    let mut manager = ScenarioManager::new();
    let definition = serde_json::json!({"name":"expired","patterns":["nginx_timeout"],"sequence_rules":{},"duration_minutes":0.5});
    manager
        .register(ScenarioGenerator::from_definition(definition, None, 0).unwrap())
        .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    manager.start_at("expired", 0, time).unwrap();
    manager
        .set_frequency(
            ErrorFrequencyConfig {
                base_frequency: 60.0,
                burst_probability: 0.0,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    let mut library = ErrorLibrary::from_sources(None, Some("database"), None, 0).unwrap();
    assert!(matches!(
        manager
            .generate_realistic_if_due(
                &mut library,
                time + chrono::Duration::minutes(1),
                Some("database"),
                None,
                true
            )
            .unwrap(),
        Some(ManagedError::Pattern(_))
    ));
    assert!(manager.active("expired").is_none());
}

#[test]
fn realistic_generation_rejects_replaced_pattern_with_existing_history() {
    use log_forge::{error_library::ErrorLibrary, frequency::ErrorFrequencyConfig};
    let mut manager = ScenarioManager::new();
    manager
        .set_frequency(
            ErrorFrequencyConfig {
                base_frequency: 60.0,
                burst_probability: 0.0,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    let mut library = ErrorLibrary::from_sources(None, None, None, 0).unwrap();
    let pattern = serde_json::from_value(serde_json::json!({"id":"unique_test", "category":"custom", "severity":"error", "message_template":"first"})).unwrap();
    library.add_pattern(pattern).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    assert!(
        manager
            .generate_realistic_if_due(&mut library, time, Some("custom"), None, false)
            .unwrap()
            .is_some()
    );
    let replacement = serde_json::from_value(serde_json::json!({"id":"unique_test", "category":"custom", "severity":"warning", "message_template":"replacement"})).unwrap();
    library.add_pattern(replacement).unwrap();
    assert!(
        manager
            .generate_realistic_if_due(
                &mut library,
                time + chrono::Duration::minutes(1),
                Some("custom"),
                None,
                false
            )
            .is_err()
    );
    let analytics = manager.pattern_analytics("unique_test").unwrap();
    assert_eq!(analytics.total_usage, 1);
    assert_eq!(analytics.pattern.severity, "error");
}

#[test]
fn realistic_generation_ignores_conflicts_outside_requested_filter() {
    use log_forge::{error_library::ErrorLibrary, frequency::ErrorFrequencyConfig};
    let mut manager = ScenarioManager::new();
    manager
        .set_frequency(
            ErrorFrequencyConfig {
                base_frequency: 60.0,
                burst_probability: 0.0,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    let mut library = ErrorLibrary::from_sources(None, None, None, 0).unwrap();
    let first = serde_json::json!({"id":"custom_a","category":"alpha_test","severity":"error","message_template":"old"});
    let second = serde_json::json!({"id":"custom_b","category":"beta_test","severity":"error","message_template":"other"});
    library
        .add_pattern(serde_json::from_value(first).unwrap())
        .unwrap();
    library
        .add_pattern(serde_json::from_value(second).unwrap())
        .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    manager
        .generate_realistic_if_due(&mut library, time, Some("alpha_test"), None, false)
        .unwrap();
    let replacement = serde_json::json!({"id":"custom_a","category":"alpha_test","severity":"error","message_template":"new"});
    library
        .add_pattern(serde_json::from_value(replacement).unwrap())
        .unwrap();
    assert!(
        manager
            .generate_realistic_if_due(
                &mut library,
                time + chrono::Duration::minutes(1),
                Some("beta_test"),
                None,
                false
            )
            .unwrap()
            .is_some()
    );
}

#[test]
fn failed_pattern_conflict_does_not_advance_frequency_time() {
    use log_forge::{error_library::ErrorLibrary, frequency::ErrorFrequencyConfig};
    let mut manager = ScenarioManager::new();
    manager
        .set_frequency(
            ErrorFrequencyConfig {
                base_frequency: 60.0,
                burst_probability: 0.0,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    let mut library = ErrorLibrary::from_sources(None, None, None, 0).unwrap();
    for (id, category) in [("custom_a", "alpha_test"), ("custom_b", "beta_test")] {
        let pattern = serde_json::json!({"id":id,"category":category,"severity":"error","message_template":"old"});
        library
            .add_pattern(serde_json::from_value(pattern).unwrap())
            .unwrap();
    }
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    manager
        .generate_realistic_if_due(&mut library, time, Some("alpha_test"), None, false)
        .unwrap();
    let replacement = serde_json::json!({"id":"custom_a","category":"alpha_test","severity":"error","message_template":"new"});
    library
        .add_pattern(serde_json::from_value(replacement).unwrap())
        .unwrap();
    assert!(
        manager
            .generate_realistic_if_due(
                &mut library,
                time + chrono::Duration::minutes(2),
                Some("alpha_test"),
                None,
                false
            )
            .is_err()
    );
    assert!(
        manager
            .generate_realistic_if_due(
                &mut library,
                time + chrono::Duration::minutes(1),
                Some("beta_test"),
                None,
                false
            )
            .unwrap()
            .is_some()
    );
}

#[test]
fn scenario_registration_rejects_conflict_with_regular_error_history() {
    use log_forge::{error_library::ErrorLibrary, frequency::ErrorFrequencyConfig};
    let root = tempfile::tempdir().unwrap();
    let mut manager = ScenarioManager::new();
    manager
        .set_frequency(
            ErrorFrequencyConfig {
                base_frequency: 60.0,
                burst_probability: 0.0,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    let mut library = ErrorLibrary::from_sources(None, None, None, 0).unwrap();
    let pattern = serde_json::json!({"id":"shared_custom", "category":"custom", "severity":"error", "message_template":"old"});
    library
        .add_pattern(serde_json::from_value(pattern).unwrap())
        .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    manager
        .generate_realistic_if_due(&mut library, time, Some("custom"), None, false)
        .unwrap();
    let replacement = serde_json::json!({"id":"shared_custom", "category":"custom", "severity":"warning", "message_template":"new"});
    std::fs::write(
        root.path().join("shared_custom.json"),
        serde_json::to_vec(&replacement).unwrap(),
    )
    .unwrap();
    let definition =
        serde_json::json!({"name":"new", "patterns":["shared_custom"], "sequence_rules":{}});
    let generator = ScenarioGenerator::from_definition(definition, Some(root.path()), 0).unwrap();
    assert!(manager.register(generator).is_err());
    assert_eq!(
        manager
            .pattern_analytics("shared_custom")
            .unwrap()
            .pattern
            .severity,
        "error"
    );
}

#[test]
fn realistic_generation_allows_weight_updates_without_losing_history() {
    use log_forge::{error_library::ErrorLibrary, frequency::ErrorFrequencyConfig};
    let mut manager = ScenarioManager::new();
    let definition =
        serde_json::json!({"name":"test","patterns":["nginx_timeout"],"sequence_rules":{}});
    manager
        .register(ScenarioGenerator::from_definition(definition, None, 0).unwrap())
        .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    manager.start_at("test", 0, time).unwrap();
    manager
        .set_frequency(
            ErrorFrequencyConfig {
                base_frequency: 60.0,
                burst_probability: 0.0,
                ..Default::default()
            },
            0,
        )
        .unwrap();
    let mut library = ErrorLibrary::from_sources(None, None, None, 0).unwrap();
    manager
        .generate_realistic_if_due(&mut library, time, None, None, true)
        .unwrap();
    library.update_weight("nginx_timeout", 3.0).unwrap();
    assert!(
        manager
            .generate_realistic_if_due(
                &mut library,
                time + chrono::Duration::seconds(1),
                None,
                None,
                true
            )
            .unwrap()
            .is_some()
    );
    assert_eq!(
        manager
            .pattern_analytics("nginx_timeout")
            .unwrap()
            .total_usage,
        2
    );
    assert_eq!(
        manager
            .pattern_analytics("nginx_timeout")
            .unwrap()
            .pattern
            .frequency_weight,
        3.0
    );
}

#[test]
fn manager_starts_at_explicit_time_and_deactivates_expired_scenario() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("error_scenarios/web_server_cascade.json");
    let mut manager = ScenarioManager::new();
    manager
        .register(ScenarioGenerator::from_sources(&path, None, 0).unwrap())
        .unwrap();
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    manager.start_at("web_server_cascade", 42, start).unwrap();
    let output = manager
        .generate("web_server_cascade", start + chrono::Duration::minutes(2))
        .unwrap();
    assert!(output.contains("pattern=nginx_connection_refused"));
    assert!(
        manager
            .generate(
                "web_server_cascade",
                start + chrono::Duration::minutes(30) + chrono::Duration::milliseconds(1)
            )
            .is_err()
    );
    assert!(manager.active("web_server_cascade").is_none());
    assert!(
        manager
            .pattern_analytics("nginx_connection_refused")
            .unwrap()
            .total_usage
            > 0
    );
}

#[test]
fn scenario_duration_excludes_subsecond_after_deadline() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("scenario.json");
    std::fs::write(&path, r#"{"name":"test","patterns":["nginx_connection_refused"],"sequence_rules":{},"duration_minutes":1}"#).unwrap();
    let mut generator = ScenarioGenerator::from_sources(&path, None, 7).unwrap();
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    generator.generate(start).unwrap();
    generator
        .generate(start + chrono::Duration::minutes(1))
        .unwrap();
    assert!(
        generator
            .generate(start + chrono::Duration::minutes(1) + chrono::Duration::milliseconds(1))
            .is_err()
    );
}

#[test]
fn scenario_duration_accepts_fractional_minutes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("scenario.json");
    std::fs::write(&path, r#"{"name":"test","patterns":["nginx_connection_refused"],"sequence_rules":{},"duration_minutes":0.5}"#).unwrap();
    let mut generator = ScenarioGenerator::from_sources(&path, None, 7).unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(generator.definition().unwrap(), original);
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    generator.generate(start).unwrap();
    generator
        .generate(start + chrono::Duration::seconds(30))
        .unwrap();
    assert!(
        generator
            .generate(start + chrono::Duration::milliseconds(30_001))
            .is_err()
    );
    let mut manager = ScenarioManager::new();
    manager
        .register(ScenarioGenerator::from_sources(&path, None, 7).unwrap())
        .unwrap();
    manager.start_at("test", 7, start).unwrap();
    manager
        .generate("test", start + chrono::Duration::seconds(30))
        .unwrap();
    assert!(
        manager
            .generate("test", start + chrono::Duration::milliseconds(30_001))
            .is_err()
    );
    assert!(manager.active("test").is_none());
    std::fs::write(&path, r#"{"name":"test","patterns":["nginx_connection_refused"],"sequence_rules":{},"duration_minutes":-0.5}"#).unwrap();
    assert!(ScenarioGenerator::from_sources(&path, None, 7).is_err());
}

#[test]
fn scenario_supports_fractional_minute_escalation_delays() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("scenario.json");
    std::fs::write(&path, r#"{"name":"test","patterns":["nginx_timeout","nginx_connection_refused"],"sequence_rules":{"escalation_chain":["nginx_timeout","nginx_connection_refused"],"escalation_delay_minutes":[0.5]}}"#).unwrap();
    let mut generator = ScenarioGenerator::from_sources(&path, None, 0).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    assert!(
        generator
            .generate(time)
            .unwrap()
            .contains("pattern=nginx_timeout")
    );
    assert!(
        generator
            .generate(time + chrono::Duration::milliseconds(29_999))
            .unwrap()
            .contains("pattern=nginx_timeout")
    );
    assert!(
        generator
            .generate(time + chrono::Duration::seconds(30))
            .unwrap()
            .contains("pattern=nginx_connection_refused")
    );
}

#[test]
fn original_scenarios_generate_each_escalation_stage() {
    let cases = [
        (
            "database_overload",
            [
                "database_slow_query",
                "mysql_connection_lost",
                "postgresql_deadlock",
            ],
            [0, 2, 5],
        ),
        (
            "web_server_cascade",
            [
                "nginx_timeout",
                "nginx_connection_refused",
                "apache_file_not_found",
            ],
            [0, 1, 3],
        ),
        (
            "system_resource_exhaustion",
            [
                "memory_allocation_failed",
                "disk_space_full",
                "service_failed_to_start",
            ],
            [0, 5, 10],
        ),
        (
            "application_validation_storm",
            [
                "fastapi_validation_error",
                "api_rate_limit_exceeded",
                "django_database_error",
            ],
            [0, 1, 2],
        ),
    ];
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    for (name, patterns, times) in cases {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("error_scenarios")
            .join(format!("{name}.json"));
        let mut generator = ScenarioGenerator::from_sources(&path, None, 42).unwrap();
        for (id, minutes) in patterns.into_iter().zip(times) {
            let output = generator
                .generate(start + chrono::Duration::minutes(minutes))
                .unwrap();
            assert!(
                output.contains(&format!("scenario={name} pattern={id} ")),
                "{output}"
            );
        }
    }
}

#[test]
fn scenario_rejects_time_regression_after_later_event() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("scenario.json");
    std::fs::write(&path, r#"{"name":"test","patterns":["nginx_connection_refused"],"sequence_rules":{},"duration_minutes":30}"#).unwrap();
    let mut generator = ScenarioGenerator::from_sources(&path, None, 7).unwrap();
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    generator.generate(start).unwrap();
    generator
        .generate(start + chrono::Duration::minutes(2))
        .unwrap();
    assert!(
        generator
            .generate(start + chrono::Duration::minutes(1))
            .is_err()
    );
    assert_eq!(generator.started_at(), Some(start));
    assert_eq!(
        generator.last_generated_at(),
        Some(start + chrono::Duration::minutes(2))
    );
    let usage = &generator.pattern_usage()["nginx_connection_refused"];
    assert_eq!(usage.count, 2);
    assert_eq!(usage.last_used, generator.last_generated_at());
    assert!(
        generator
            .generate(start + chrono::Duration::minutes(31))
            .is_err()
    );
    assert_eq!(
        generator.pattern_usage()["nginx_connection_refused"].count,
        2
    );
}

#[test]
fn every_original_builtin_pattern_renders() {
    let patterns: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../data/builtin_error_patterns.json")).unwrap();
    assert_eq!(patterns.len(), 15);
    let path = std::env::temp_dir().join(format!(
        "log-forge-builtin-patterns-{}.json",
        std::process::id()
    ));
    for pattern in patterns {
        let id = pattern["id"].as_str().unwrap();
        let scenario = serde_json::json!({"name": "test", "patterns": [id], "sequence_rules": {}});
        std::fs::write(&path, serde_json::to_vec(&scenario).unwrap()).unwrap();
        let mut generator = ScenarioGenerator::from_sources(&path, None, 7).unwrap();
        let record = generator
            .generate(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap())
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(record.contains(&format!("pattern={id}")));
        assert!(!record.contains('{'));
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn library_analytics_rejects_conflicting_pattern_definitions() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("error_scenarios/web_server_cascade.json");
    let mut manager = ScenarioManager::new();
    manager
        .register(ScenarioGenerator::from_sources(&path, None, 1).unwrap())
        .unwrap();
    let mut library =
        log_forge::error_library::ErrorLibrary::from_sources(None, None, None, 1).unwrap();
    let mut pattern = library.get_pattern("nginx_timeout").unwrap().clone();
    pattern.description = "different pattern".into();
    library.add_pattern(pattern).unwrap();
    assert!(
        manager
            .pattern_analytics_with_library(&library, "nginx_timeout")
            .is_err()
    );
}
