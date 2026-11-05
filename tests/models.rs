use chrono::{TimeZone, Utc};
use log_forge::models::{GenerationStatistics, LogEntry};
use serde_json::json;

#[test]
fn formats_entry_and_preserves_custom_fields() {
    let time = Utc
        .with_ymd_and_hms(2023, 12, 25, 10, 0, 0)
        .single()
        .unwrap();
    let mut entry = LogEntry::new(time, "nginx", "INFO", "Test message");
    entry.source_ip = Some("192.168.1.100".into());
    entry.status_code = Some(200);
    entry.response_time = Some(0.123);
    entry.custom_fields.insert("user_id".into(), json!(123));
    assert_eq!(
        entry.to_string(),
        "192.168.1.100 - 2023-12-25T10:00:00+00:00 [INFO] Test message - Status: 200 - Time: 0.123s"
    );
    let value: serde_json::Value = serde_json::from_str(&entry.to_json().unwrap()).unwrap();
    assert_eq!(value["user_id"], 123);
    assert_eq!(value["source_ip"], "192.168.1.100");
    assert_eq!(value["status_code"], 200);
    assert!(value.get("user_agent").is_none());
    assert!(entry.to_dict().unwrap()["user_agent"].is_null());
    entry.response_time = Some(0.0);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&entry.to_json().unwrap()).unwrap()["response_time"],
        0.0
    );
    entry.response_time = Some(f64::NAN);
    assert!(entry.to_json().is_err());
}

#[test]
fn tracks_generation_counts_rates_and_timestamps() {
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).single().unwrap();
    let mut stats = GenerationStatistics::default();
    stats.add_log("redis");
    stats.add_log("redis");
    stats.add_log("ssh_auth");
    stats.add_error();
    assert_eq!(stats.calculate_rate(), 0.0);
    stats.start_time = Some(start);
    stats.end_time = Some(start + chrono::Duration::milliseconds(1500));
    assert_eq!(stats.calculate_rate(), 2.0);
    let fields = stats.to_dict();
    assert_eq!(fields["total_logs_generated"], 3);
    assert_eq!(fields["logs_by_type"]["redis"], 2);
    assert_eq!(fields["errors_count"], 1);
    assert_eq!(fields["start_time"], "2024-01-01T00:00:00+00:00");
    assert_eq!(fields["generation_rate"], 2.0);
    stats.end_time = Some(start);
    assert_eq!(stats.calculate_rate(), 0.0);
    stats.end_time = Some(start - chrono::Duration::seconds(1));
    assert_eq!(stats.calculate_rate(), 0.0);
}
