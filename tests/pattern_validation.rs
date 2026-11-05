use chrono::{TimeZone, Utc};
use log_forge::{pattern_file::LogPattern, pattern_validation};
use std::{fs, process::Command};

fn pattern(name: &str, template: &str) -> LogPattern {
    serde_json::from_value(serde_json::json!({"name":name,"template":template})).unwrap()
}

#[test]
fn suggests_original_improvements_and_handles_escaped_braces() {
    let suggestions =
        pattern_validation::suggest_improvements(&pattern("short", "{ip_address} {ip_address}"))
            .unwrap();
    assert!(
        suggestions
            .iter()
            .any(|s| s.contains("different field names"))
    );
    assert!(suggestions.iter().any(|s| s.contains("timestamp field")));
    assert!(suggestions.iter().any(|s| s.contains("static content")));
    assert!(suggestions.iter().any(|s| s.contains("field options")));
    let suggestions = pattern_validation::suggest_improvements(&pattern(
        "structured",
        "{{request status={status:200,500} timestamp={timestamp}}}",
    ))
    .unwrap();
    assert!(suggestions.is_empty(), "{suggestions:?}");
    assert!(pattern_validation::suggest_improvements(&pattern("bad", "{timestamp")).is_err());
}

#[test]
fn validates_templates_samples_regex_and_compares_fields() {
    let now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let first = pattern(
        "first",
        "{{\\\"status\\\":\\\"{status:200,500}\\\",\\\"n\\\":{counter}}}",
    );
    let report = pattern_validation::validate(&first);
    assert!(report.valid, "{:?}", report.errors);
    assert_eq!(
        report
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["status", "counter"]
    );
    let samples = pattern_validation::test_generation(&first, now, 7, 5).unwrap();
    assert_eq!(samples.total_generated, 5);
    assert_eq!(samples.unique_outputs, 5);
    assert!(samples.samples[0].starts_with('{'));
    let second = pattern("second", "{status:200,500} {timestamp}");
    let comparison = pattern_validation::compare(&first, &second, now, 7, 5).unwrap();
    assert_eq!(comparison.common_fields, ["status"]);
    assert_eq!(comparison.first_only, ["counter"]);
    assert_eq!(comparison.second_only, ["timestamp"]);
    let regex =
        pattern_validation::test_regex(r"^(foo)-(\d+)$", &["foo-42".into(), "bad".into()]).unwrap();
    assert_eq!(regex.matches.len(), 1);
    assert_eq!(
        regex.matches[0].groups,
        [Some("foo".into()), Some("42".into())]
    );
    assert_eq!(regex.non_matches, ["bad"]);
    assert!(pattern_validation::test_regex("(", &[]).is_err());
    let benchmark = pattern_validation::benchmark(&first, now, 7, 10).unwrap();
    assert_eq!(benchmark.iterations, 10);
    assert!(benchmark.max_seconds >= benchmark.min_seconds);
    assert!(pattern_validation::test_generation(&first, now, 7, 0).is_err());
}

#[test]
fn malformed_braces_fail_before_output_and_escaped_braces_generate_json() {
    for template in ["broken } literal", "{timestamp", "{}", "{outer{inner}}"] {
        let report = pattern_validation::validate(&pattern("bad", template));
        assert!(!report.valid, "{template}");
        assert!(!report.errors.is_empty());
    }
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.json");
    let output = root.path().join("out.log");
    let config_value = |template: &str| serde_json::json!({"log_generator": {"global": {"output_format":"file", "output_path":output, "total_logs":2, "start_time":"2024-01-01"}, "log_types": {"custom": {"template":template}}}});
    fs::write(
        &config,
        serde_json::to_vec(&config_value("bad } trailing")).unwrap(),
    )
    .unwrap();
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
    let invalid = run();
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("unmatched closing brace"));
    assert!(!output.exists());
    fs::write(
        &config,
        serde_json::to_vec(&config_value(
            r#"{{"status":"{status:200,500}","n":{counter}}}"#,
        ))
        .unwrap(),
    )
    .unwrap();
    let valid = run();
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    for (index, line) in fs::read_to_string(output).unwrap().lines().enumerate() {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(["200", "500"].contains(&value["status"].as_str().unwrap()));
        assert_eq!(value["n"].as_u64().unwrap(), index as u64 + 1);
    }
}
