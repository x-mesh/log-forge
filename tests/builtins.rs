use chrono::{Duration, TimeZone, Utc};
use log_forge::{
    BUILTIN_GENERATOR_TYPES, GeneratorRegistry, create_builtin,
    engine::GenerationEngine,
    output::{OutputHandler, RotationOptions},
};

#[test]
fn builtin_generators_produce_reproducible_valid_records() {
    let kinds = [
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
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    for kind in kinds {
        let mut first = create_builtin(kind, &serde_yaml::Value::Null, 42).unwrap();
        let mut second = create_builtin(kind, &serde_yaml::Value::Null, 42).unwrap();
        for index in 0..5 {
            let time = start + Duration::seconds(index);
            let record = first.generate(time).unwrap();
            assert_eq!(record, second.generate(time).unwrap(), "{kind}");
            assert!(first.validate(&record), "{kind}: {record}");
            assert!(!first.pattern().is_empty());
        }
    }
    assert!(create_builtin("missing", &serde_yaml::Value::Null, 0).is_err());
    assert!(create_builtin("scenario", &serde_yaml::Value::Null, 0).is_err());
}

#[test]
fn registry_registers_all_builtin_factories() {
    let registry = GeneratorRegistry::with_builtins(42).unwrap();
    assert_eq!(registry.types().count(), BUILTIN_GENERATOR_TYPES.len());
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    for &kind in BUILTIN_GENERATOR_TYPES {
        assert!(registry.contains(kind));
        let mut generator = registry.create(kind, &serde_yaml::Value::Null).unwrap();
        let record = generator.generate(time).unwrap();
        assert!(generator.validate(&record), "{kind}: {record}");
    }
    assert!(
        registry
            .create("missing", &serde_yaml::Value::Null)
            .is_err()
    );
}

#[test]
fn builtin_generator_applies_type_configuration_in_engine() {
    let settings: serde_yaml::Value = serde_yaml::from_str("custom_fields:\n  ip_ranges: ['10.1.2.0/24']\n  status_codes: {503: 1}\n  url_patterns: ['/ready']\n").unwrap();
    let generator = create_builtin("nginx_access", &settings, 7).unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("out.log");
    let mut engine = GenerationEngine::new(3);
    engine
        .add_generator("nginx_access", 1.0, generator)
        .unwrap();
    engine.add_output(OutputHandler::file(&path, RotationOptions::default()).unwrap());
    engine
        .run([Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()])
        .unwrap();
    engine.close().unwrap();
    let output = std::fs::read_to_string(path).unwrap();
    assert!(output.starts_with("10.1.2."));
    assert!(output.contains(" /ready "));
    assert!(output.contains(" 503 "));
    let invalid: serde_yaml::Value = serde_yaml::from_str("unsupported: true").unwrap();
    assert!(create_builtin("nginx_access", &invalid, 0).is_err());
}

#[test]
fn builtin_docker_metadata_reaches_structured_engine_output() {
    use std::collections::BTreeMap;
    let settings: serde_yaml::Value =
        serde_yaml::from_str("log_levels: {error: 1}\nlog_drivers: [json-file]\n").unwrap();
    let mut generator = create_builtin("docker", &settings, 7).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let generated = generator.generate_record(time).unwrap();
    assert_eq!(generated.level.as_deref(), Some("ERROR"));
    let raw: serde_json::Value = serde_json::from_str(&generated.message).unwrap();
    assert_eq!(raw.as_object().unwrap().len(), 3);
    assert!(raw.get("level").is_none());
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("out.json");
    let mut engine = GenerationEngine::new(0);
    engine.add_generator("docker", 1.0, generator).unwrap();
    engine.add_output(
        OutputHandler::file(&path, RotationOptions::default())
            .unwrap()
            .json(BTreeMap::new(), false, false),
    );
    engine.run([time]).unwrap();
    engine.close().unwrap();
    let structured: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(structured["level"], "ERROR");
    let raw: serde_json::Value =
        serde_json::from_str(structured["message"].as_str().unwrap()).unwrap();
    assert_eq!(raw.as_object().unwrap().len(), 3);
}

#[test]
fn builtin_docker_accepts_explicit_container_identity() {
    let config: serde_yaml::Value = serde_yaml::from_str("container_id: '12345678901234567890'\ncontainer_name: checkout\nlog_drivers: [journald]\ncustom_fields:\n  log_levels: {error: 1}\n").unwrap();
    let mut generator = create_builtin("docker", &config, 7).unwrap();
    let record = generator
        .generate_record(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap())
        .unwrap();
    assert_eq!(record.level.as_deref(), Some("ERROR"));
    let json: serde_json::Value = serde_json::from_str(&record.message).unwrap();
    assert_eq!(json["CONTAINER_ID"], "123456789012");
    assert_eq!(json["CONTAINER_ID_FULL"], "12345678901234567890");
    assert_eq!(json["CONTAINER_NAME"], "checkout");
    let conflict: serde_yaml::Value =
        serde_yaml::from_str("container_id: first\ncustom_fields:\n  container_id: second\n")
            .unwrap();
    assert!(create_builtin("docker", &conflict, 7).is_err());
    assert!(create_builtin("redis", &config, 7).is_err());
}

#[test]
fn builtin_redis_honors_weighted_level_and_metadata() {
    let config: serde_yaml::Value =
        serde_yaml::from_str("log_levels: {DEBUG: 1, WARNING: 0}\n").unwrap();
    let mut generator = create_builtin("redis", &config, 42).unwrap();
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    for index in 0..20 {
        let record = generator
            .generate_record(start + Duration::seconds(index))
            .unwrap();
        assert_eq!(record.level.as_deref(), Some("DEBUG"));
        assert_eq!(record.message.split_whitespace().nth(5), Some("."));
        assert!(generator.validate(&record.message));
    }
}

#[test]
fn docker_metadata_keeps_configured_identity_and_image() {
    let config: serde_yaml::Value = serde_yaml::from_str("container_id: abc123\ncontainer_name: checkout\ncustom_fields:\n  images: ['registry.example/checkout:v1']\n").unwrap();
    let mut generator = create_builtin("docker", &config, 12).unwrap();
    let metadata = generator.metadata();
    assert_eq!(metadata["container_id"], "abc123");
    assert_eq!(metadata["container_name"], "checkout");
    assert_eq!(metadata["image_name"], "registry.example/checkout:v1");
    let generated = generator
        .generate(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap())
        .unwrap();
    let raw: serde_json::Value = serde_json::from_str(&generated).unwrap();
    assert_eq!(raw.as_object().unwrap().len(), 3);
    assert_eq!(generator.metadata(), metadata);
    let same = create_builtin("docker", &config, 12).unwrap();
    assert_eq!(same.metadata(), metadata);
    let explicit: serde_yaml::Value =
        serde_yaml::from_str("image_name: explicit:v2\nimages: [other:v1]\n").unwrap();
    assert_eq!(
        create_builtin("docker", &explicit, 1).unwrap().metadata()["image_name"],
        "explicit:v2"
    );
    let empty: serde_yaml::Value = serde_yaml::from_str("images: []").unwrap();
    assert!(create_builtin("docker", &empty, 1).is_err());
    assert!(create_builtin("redis", &config, 1).is_err());
    assert!(
        create_builtin("redis", &serde_yaml::Value::Null, 1)
            .unwrap()
            .metadata()
            .is_empty()
    );
}

#[test]
fn kubernetes_pod_metadata_matches_generated_identity() {
    let settings: serde_yaml::Value = serde_yaml::from_str("namespace: payments\npod_name: checkout-123\ncontainer_name: checkout\nlabels: {app: checkout, team: orders}\nlog_levels: {ERROR: 1}\n").unwrap();
    let mut generator = create_builtin("kubernetes_pod", &settings, 5).unwrap();
    let metadata = generator.structured_metadata();
    assert_eq!(metadata["namespace"], "payments");
    assert_eq!(metadata["pod_name"], "checkout-123");
    assert_eq!(metadata["container_name"], "checkout");
    assert_eq!(metadata["labels"]["app"], "checkout");
    assert_eq!(metadata["labels"]["team"], "orders");
    assert!(metadata["labels"]["version"].as_str().is_some());
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut observed = false;
    for _ in 0..200 {
        let record = generator.generate(time).unwrap();
        if record.contains("failed readiness probe") {
            assert!(record.contains("Pod checkout-123 in namespace payments"));
            observed = true;
        }
    }
    assert!(observed);
    assert_eq!(generator.structured_metadata(), metadata);
    assert_eq!(
        create_builtin("kubernetes_pod", &settings, 5)
            .unwrap()
            .structured_metadata(),
        metadata
    );
    let invalid: serde_yaml::Value =
        serde_yaml::from_str("namespace: one\ncustom_fields: {namespace: two}\n").unwrap();
    assert!(create_builtin("kubernetes_pod", &invalid, 1).is_err());
    let choices: serde_yaml::Value =
        serde_yaml::from_str("namespaces: [one, two]\nlog_levels: {ERROR: 1}\n").unwrap();
    let mut selected = create_builtin("kubernetes_pod", &choices, 9).unwrap();
    let namespace = selected.metadata()["namespace"].clone();
    assert!(["one", "two"].contains(&namespace.as_str()));
    let mut observed = false;
    for _ in 0..200 {
        let record = selected.generate(time).unwrap();
        if record.contains(" in namespace ") {
            assert!(
                record.contains(&format!(" in namespace {namespace}")),
                "{record}"
            );
            observed = true;
        }
    }
    assert!(observed);
}

#[test]
fn fastapi_custom_endpoint_does_not_generate_unused_default_dates() {
    let config: serde_yaml::Value =
        serde_yaml::from_str("log_levels: {INFO: 1}\nendpoints: ['/only']\n").unwrap();
    let mut generator = create_builtin("fastapi", &config, 42).unwrap();
    for _ in 0..300 {
        generator
            .generate(chrono::DateTime::<Utc>::MIN_UTC)
            .unwrap();
    }
}

#[test]
fn builtin_docker_default_container_names_follow_original_choices() {
    let expected = [
        "web-server",
        "api-backend",
        "database",
        "cache-redis",
        "worker-queue",
        "nginx-proxy",
        "app-frontend",
        "monitoring",
        "logging-agent",
        "load-balancer",
    ];
    let mut names = std::collections::BTreeSet::new();
    for seed in 0..40 {
        let generator = create_builtin("docker", &serde_yaml::Value::Null, seed).unwrap();
        let metadata = generator.metadata();
        let name = metadata.get("container_name").unwrap();
        assert!(expected.contains(&name.as_str()), "{name}");
        names.insert(name.clone());
    }
    assert!(names.len() > 1);
}

#[test]
fn docker_default_identity_matches_metadata_across_formats() {
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut expected = None;
    for config in [
        "format_type: text\n",
        "log_drivers: [syslog]\n",
        "log_drivers: [journald]\n",
    ] {
        let settings: serde_yaml::Value = serde_yaml::from_str(config).unwrap();
        let mut generator = create_builtin("docker", &settings, 42).unwrap();
        let name = generator.metadata()["container_name"].clone();
        if let Some(expected) = &expected {
            assert_eq!(&name, expected);
        } else {
            expected = Some(name.clone());
        }
        for _ in 0..10 {
            let record = generator.generate(time).unwrap();
            assert!(record.contains(&name), "{config}: {record}");
            assert!(generator.validate(&record), "{config}: {record}");
        }
    }
}

#[test]
fn kubernetes_default_identity_uses_original_choices() {
    let mut namespaces = std::collections::BTreeSet::new();
    let mut prefixes = std::collections::BTreeSet::new();
    let mut containers = std::collections::BTreeSet::new();
    let pod_name = regex::Regex::new(r"^([a-z-]+)-[0-9a-f]{10}$").unwrap();
    for seed in 0..40 {
        let generator = create_builtin("kubernetes_pod", &serde_yaml::Value::Null, seed).unwrap();
        let metadata = generator.metadata();
        namespaces.insert(metadata["namespace"].clone());
        prefixes.insert(pod_name.captures(&metadata["pod_name"]).unwrap()[1].to_owned());
        containers.insert(metadata["container_name"].clone());
    }
    assert!(namespaces.len() > 1, "{namespaces:?}");
    assert!(prefixes.len() > 1, "{prefixes:?}");
    assert!(containers.len() > 1, "{containers:?}");
}

#[test]
fn syslog_priority_covers_original_facilities_and_severities() {
    let facilities = [
        ("kern", 0),
        ("user", 1),
        ("mail", 2),
        ("daemon", 3),
        ("auth", 4),
        ("syslog", 5),
        ("lpr", 6),
        ("news", 7),
        ("uucp", 8),
        ("cron", 9),
        ("authpriv", 10),
        ("ftp", 11),
        ("local0", 16),
        ("local1", 17),
        ("local2", 18),
        ("local3", 19),
        ("local4", 20),
        ("local5", 21),
        ("local6", 22),
        ("local7", 23),
    ];
    let severities = [
        "emerg", "alert", "crit", "err", "warning", "notice", "info", "debug",
    ];
    let levels = [
        "EMERGENCY",
        "ALERT",
        "CRITICAL",
        "ERROR",
        "WARNING",
        "NOTICE",
        "INFO",
        "DEBUG",
    ];
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("syslog.jsonl");
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .json(Default::default(), false, false);
    let mut analyzer = log_forge::analysis::LogAnalyzer::new("syslog").unwrap();
    let mut expected = Vec::new();
    for (facility, code) in facilities {
        for (severity_code, severity) in severities.iter().enumerate() {
            let config: serde_yaml::Value = serde_yaml::from_str(&format!(
                "facilities: [{facility}]\nseverities: [{severity}]\n"
            ))
            .unwrap();
            let mut generator = create_builtin("syslog", &config, 42).unwrap();
            let record = generator.generate(time).unwrap();
            assert!(
                record.starts_with(&format!("<{}>", code * 8 + severity_code)),
                "{facility} {severity}: {record}"
            );
            assert!(
                generator.validate(&record),
                "{facility} {severity}: {record}"
            );
            output.write("syslog", time, &record).unwrap();
            analyzer.observe(&record);
            expected.push(levels[severity_code]);
        }
    }
    output.close().unwrap();
    let contents = std::fs::read_to_string(path).unwrap();
    assert_eq!(contents.lines().count(), expected.len());
    for (line, level) in contents.lines().zip(expected) {
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(record["level"], level);
    }
    let stats = analyzer.statistics();
    for level in levels {
        assert_eq!(stats.log_levels.get(level), Some(&20));
    }
}
