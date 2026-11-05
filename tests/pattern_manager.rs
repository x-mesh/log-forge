use chrono::{TimeZone, Utc};
use log_forge::pattern_file::{LogPattern, PatternManager};
use std::fs;

#[test]
fn creates_original_sample_patterns_without_overwriting_existing_files() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = PatternManager::open(root.path()).unwrap();
    let first = manager.create_sample_patterns().unwrap();
    assert_eq!(
        first.created,
        ["nginx_custom", "api_request", "database_query"]
    );
    assert!(first.existing.is_empty());
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    for name in &first.created {
        let generated = manager.generate(name, time, 42, 0).unwrap();
        assert!(!generated.contains('{'), "{name}: {generated}");
        let pattern = manager.get(name).unwrap();
        assert!(!pattern.sample_output.as_ref().unwrap().contains('{'));
        assert!(chrono::DateTime::parse_from_rfc3339(pattern.created_at.as_ref().unwrap()).is_ok());
        assert_eq!(pattern.created_at, pattern.updated_at);
    }
    let path = root.path().join("nginx_custom.json");
    let before = fs::read(&path).unwrap();
    let second = manager.create_sample_patterns().unwrap();
    assert!(second.created.is_empty());
    assert_eq!(second.existing, first.created);
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(
        PatternManager::open(root.path())
            .unwrap()
            .list(None)
            .count(),
        3
    );
}

#[test]
fn add_and_update_record_samples_and_preserve_creation_time() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = PatternManager::open(root.path()).unwrap();
    let original: LogPattern = serde_json::from_value(serde_json::json!({
        "name":"example", "template":"{timestamp} {message}",
        "created_at":"2020-01-01T00:00:00Z",
        "fields":{"message":{"type":"choice","options":["alpha"]}}
    }))
    .unwrap();
    manager.add(original.clone()).unwrap();
    let created = manager.get("example").unwrap();
    assert_eq!(created.created_at.as_deref(), Some("2020-01-01T00:00:00Z"));
    assert!(created.sample_output.as_ref().unwrap().ends_with("alpha"));
    assert!(chrono::DateTime::parse_from_rfc3339(created.updated_at.as_ref().unwrap()).is_ok());
    let mut changed: LogPattern = serde_json::from_value(serde_json::json!({
        "name":"example", "template":"{timestamp} {message}",
        "fields":{"message":{"type":"choice","options":["beta"]}}
    }))
    .unwrap();
    changed.created_at = None;
    manager.update(changed).unwrap();
    let updated = PatternManager::open(root.path()).unwrap();
    let stored = updated.get("example").unwrap();
    assert_eq!(stored.created_at.as_deref(), Some("2020-01-01T00:00:00Z"));
    assert!(stored.sample_output.as_ref().unwrap().ends_with("beta"));
    assert!(chrono::DateTime::parse_from_rfc3339(stored.updated_at.as_ref().unwrap()).is_ok());
}

#[test]
fn manages_original_patterns_and_generates_typed_fields() {
    let root = tempfile::tempdir().unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("demo_patterns/custom_web_access.json");
    fs::copy(source, root.path().join("custom_web_access.json")).unwrap();
    let mut manager = PatternManager::open(root.path()).unwrap();
    assert_eq!(manager.list(Some("web_server")).count(), 1);
    assert_eq!(manager.categories(), ["web_server"]);
    let sample: LogPattern = serde_json::from_value(serde_json::json!({
        "name": "typed", "description": "typed fields", "category": "test",
        "template": "{pid} {ratio} {count} {id}",
        "fields": {
            "pid": {"type":"random_int", "options":{"min_val":7,"max_val":7}},
            "ratio": {"type":"random_float", "options":{"min_val":0.5,"max_val":0.5}},
            "count": {"type":"counter", "options":{"start":10,"step":2}},
            "id": {"type":"uuid"}
        }
    }))
    .unwrap();
    manager.add(sample.clone()).unwrap();
    assert!(manager.add(sample.clone()).is_err());
    let now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let generated = manager.generate("typed", now, 4, 2).unwrap();
    assert!(generated.starts_with("7 0.500 14 "), "{generated}");
    assert_eq!(generated.split_whitespace().last().unwrap().len(), 36);
    let overrides = std::collections::BTreeMap::from([("pid".to_owned(), "42".to_owned())]);
    assert!(
        manager
            .generate_with_values("typed", now, 4, 2, &overrides)
            .unwrap()
            .starts_with("42 0.500 14 ")
    );
    let json = root.path().join("export.json");
    let yaml = root.path().join("export.yaml");
    manager.export(&json).unwrap();
    manager.export(&yaml).unwrap();
    assert!(manager.export(&json).is_err());
    let other = tempfile::tempdir().unwrap();
    let mut imported = PatternManager::open(other.path()).unwrap();
    assert_eq!(imported.import(&yaml).unwrap(), 2);
    assert!(imported.import(&json).is_err());
    assert_eq!(imported.generate("typed", now, 4, 2).unwrap(), generated);
    let mut changed = sample;
    changed.description = "updated".into();
    imported.update(changed).unwrap();
    let loaded = PatternManager::open(other.path()).unwrap();
    assert_eq!(loaded.get("typed").unwrap().description, "updated");
    imported.remove("typed").unwrap();
    assert!(imported.get("typed").is_none());
    assert!(!other.path().join("typed.json").exists());
}

#[test]
fn rejects_path_escape_and_unsafe_edits_of_shared_files() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = PatternManager::open(root.path()).unwrap();
    let bad: LogPattern =
        serde_json::from_value(serde_json::json!({"name":"../escape", "template":"{timestamp}"}))
            .unwrap();
    assert!(manager.add(bad).is_err());
    let multi = serde_json::json!({"patterns":[{"name":"a", "template":"{timestamp}"},{"name":"b", "template":"{timestamp}"}]});
    fs::write(
        root.path().join("bundle.json"),
        serde_json::to_vec(&multi).unwrap(),
    )
    .unwrap();
    let mut manager = PatternManager::open(root.path()).unwrap();
    assert!(manager.remove("a").is_err());
    assert!(root.path().join("bundle.json").exists());
    assert_eq!(manager.list(None).count(), 2);
}

#[test]
fn original_faker_generators_are_available_in_typed_patterns() {
    let root = tempfile::tempdir().unwrap();
    let mut manager = PatternManager::open(root.path()).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
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
        "response_time",
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
    for field in fields {
        let pattern: LogPattern = serde_json::from_value(serde_json::json!({
            "name": format!("test_{field}"), "template": "{value}",
            "fields": {"value": {"type": "faker", "generator": field}}
        }))
        .unwrap();
        manager
            .add(pattern)
            .unwrap_or_else(|error| panic!("{field}: {error}"));
        let value = manager
            .generate(&format!("test_{field}"), time, 42, 0)
            .unwrap();
        assert!(!value.is_empty(), "{field}");
        assert!(!value.contains(['\n', '\r']), "{field}: {value:?}");
    }
}
