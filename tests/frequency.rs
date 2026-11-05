use chrono::{Duration, TimeZone, Utc};
use log_forge::frequency::{ErrorFrequencyConfig, ErrorFrequencyController};

#[test]
fn peak_and_burst_adjust_probability_deterministically() {
    let config = ErrorFrequencyConfig {
        base_frequency: 30.0,
        peak_hours: vec![10],
        peak_multiplier: 2.0,
        burst_probability: 0.0,
        burst_multiplier: 1.0,
        burst_duration_minutes: 1,
    };
    let peak = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
    let mut a = ErrorFrequencyController::new(config.clone(), 42).unwrap();
    let mut b = ErrorFrequencyController::new(config, 42).unwrap();
    for index in 0..100 {
        let time = peak + Duration::seconds(index);
        assert!(a.should_generate(time).unwrap());
        assert!(b.should_generate(time).unwrap());
    }
    let outside_peak = Utc.with_ymd_and_hms(2024, 1, 1, 2, 0, 0).unwrap();
    let repeatable = ErrorFrequencyConfig {
        base_frequency: 30.0,
        peak_hours: vec![],
        peak_multiplier: 1.0,
        burst_probability: 0.0,
        burst_multiplier: 1.0,
        burst_duration_minutes: 1,
    };
    let mut first = ErrorFrequencyController::new(repeatable.clone(), 7).unwrap();
    let mut second = ErrorFrequencyController::new(repeatable, 7).unwrap();
    let choices: Vec<_> = (0..100)
        .map(|i| {
            first
                .should_generate(outside_peak + Duration::seconds(i))
                .unwrap()
        })
        .collect();
    let repeated: Vec<_> = (0..100)
        .map(|i| {
            second
                .should_generate(outside_peak + Duration::seconds(i))
                .unwrap()
        })
        .collect();
    assert_eq!(choices, repeated);
    assert!(choices.contains(&true) && choices.contains(&false));
    let burst_config = ErrorFrequencyConfig {
        base_frequency: 6.0,
        peak_hours: vec![],
        peak_multiplier: 1.0,
        burst_probability: 1.0,
        burst_multiplier: 10.0,
        burst_duration_minutes: 2,
    };
    let mut burst = ErrorFrequencyController::new(burst_config, 1).unwrap();
    assert!(burst.should_generate(peak).unwrap());
    assert!(!burst.in_burst(peak - Duration::seconds(1)));
    assert!(burst.in_burst(peak + Duration::seconds(119)));
    assert!(!burst.in_burst(peak + Duration::seconds(120)));
    assert!(burst.should_generate(peak - Duration::seconds(1)).is_err());
}

#[test]
fn rejects_invalid_frequency_configuration() {
    let invalid = ErrorFrequencyConfig {
        burst_probability: 1.1,
        ..Default::default()
    };
    assert!(ErrorFrequencyController::new(invalid, 0).is_err());
}

#[test]
fn original_web_and_security_presets_produce_configured_fields() {
    use log_forge::frequency::{FieldFrequencyConfig, FieldFrequencyController};
    use std::collections::HashMap;
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
    for (config, field, expected) in [
        (FieldFrequencyConfig::web(), "http_method", "GET"),
        (
            FieldFrequencyConfig::security(),
            "security_event",
            "login_success",
        ),
    ] {
        config.validate().unwrap();
        let mut controller = FieldFrequencyController::new(config, 42).unwrap();
        let values: Vec<_> = (0..100)
            .map(|_| {
                controller
                    .controlled_value(field, &HashMap::new(), time, || "fallback".into())
                    .unwrap()
            })
            .collect();
        assert!(values.iter().any(|value| value == expected));
        assert!(!values.iter().any(|value| value == "fallback"));
    }
    let web = FieldFrequencyConfig::web();
    assert_eq!(web.correlation_rules[0].condition_value, "500");
    assert_eq!(
        web.burst_patterns["http_status_code"].burst_duration_seconds,
        300
    );
    assert_eq!(
        FieldFrequencyConfig::security().burst_patterns["security_event"].burst_duration_seconds,
        600
    );
}

#[test]
fn field_frequency_statistics_report_distinct_configured_fields() {
    use log_forge::frequency::{FieldFrequencyConfig, FieldFrequencyController};
    let controller = FieldFrequencyController::new(FieldFrequencyConfig::web(), 42).unwrap();
    let statistics = controller.statistics();
    assert_eq!(statistics.frequency_maps_count, 3);
    assert_eq!(statistics.time_patterns_count, 1);
    assert_eq!(statistics.correlation_rules_count, 1);
    assert_eq!(statistics.burst_patterns_count, 1);
    assert_eq!(
        statistics.configured_fields,
        ["http_method", "http_status_code", "log_level"]
    );
    assert!(serde_json::to_string(&statistics).is_ok());
}

#[test]
fn database_preset_correlates_integer_rows_above_threshold() {
    use log_forge::frequency::{FieldFrequencyConfig, FieldFrequencyController};
    use std::collections::HashMap;
    let mut config = FieldFrequencyConfig::database();
    config.correlation_rules[0].probability = 1.0;
    let mut controller = FieldFrequencyController::new(config.clone(), 42).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
    let make_context = |rows: &str| HashMap::from([("rows_affected".to_owned(), rows.to_owned())]);
    assert_eq!(
        controller
            .controlled_value("query_duration", &make_context("1000"), time, || "fallback"
                .into())
            .unwrap(),
        "fallback"
    );
    let value = controller
        .controlled_value("query_duration", &make_context("1001"), time, || {
            "fallback".into()
        })
        .unwrap();
    assert!(["100", "500", "1000", "2000"].contains(&value.as_str()));
    assert!(
        controller
            .controlled_value("query_duration", &make_context("invalid"), time, || {
                "fallback".into()
            })
            .is_err()
    );
    config.correlation_rules[0].condition_value = "invalid".into();
    assert!(FieldFrequencyController::new(config, 0).is_err());
}

#[test]
fn field_frequency_supports_time_rules_correlations_and_timed_bursts() {
    use log_forge::frequency::{
        CorrelationRule, FieldBurstPattern, FieldFrequencyConfig, FieldFrequencyController,
        FieldTimePattern,
    };
    use std::collections::{BTreeMap, HashMap};
    let monday = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
    let saturday = Utc.with_ymd_and_hms(2024, 1, 6, 10, 0, 0).unwrap();
    let config = FieldFrequencyConfig {
        distributions: BTreeMap::from([(
            "status".into(),
            BTreeMap::from([("ok".into(), 1.0), ("failed".into(), 0.0)]),
        )]),
        time_patterns: BTreeMap::from([(
            "status".into(),
            FieldTimePattern {
                peak_hours: vec![10],
                peak_multiplier: 4.0,
                weekend_factor: 0.0,
                business_hours_only: false,
            },
        )]),
        correlation_rules: vec![CorrelationRule {
            condition_field: "status".into(),
            condition_value: "failed".into(),
            condition_operator: log_forge::frequency::ConditionOperator::Equal,
            target_field: "latency".into(),
            target_distribution: BTreeMap::from([("slow".into(), 1.0)]),
            probability: 1.0,
        }],
        burst_patterns: BTreeMap::from([(
            "status".into(),
            FieldBurstPattern {
                burst_probability: 1.0,
                burst_duration_seconds: 60,
                burst_multiplier: 4.0,
                burst_values: vec!["failed".into()],
            },
        )]),
    };
    let mut controller = FieldFrequencyController::new(config, 7).unwrap();
    assert!(controller.should_generate_field("status", monday).unwrap());
    assert_eq!(
        controller
            .controlled_value("status", &HashMap::new(), monday, || "default".into())
            .unwrap(),
        "failed"
    );
    assert!(!controller.in_burst("status", monday - Duration::seconds(1)));
    assert!(controller.in_burst("status", monday + Duration::seconds(59)));
    assert!(!controller.in_burst("status", monday + Duration::seconds(60)));
    let context = HashMap::from([("status".to_owned(), "failed".to_owned())]);
    assert_eq!(
        controller
            .controlled_value("latency", &context, monday, || "normal".into())
            .unwrap(),
        "slow"
    );
    assert!(
        !controller
            .should_generate_field("status", saturday)
            .unwrap()
    );
    assert!(controller.should_generate_field("status", monday).is_err());
}

#[test]
fn field_frequency_checks_configuration_and_is_seeded() {
    use log_forge::frequency::{
        FieldBurstPattern, FieldFrequencyConfig, FieldFrequencyController, FieldTimePattern,
    };
    use std::collections::{BTreeMap, HashMap};
    let config = FieldFrequencyConfig {
        distributions: BTreeMap::from([(
            "status".into(),
            BTreeMap::from([("200".into(), 0.8), ("500".into(), 0.2)]),
        )]),
        time_patterns: BTreeMap::from([(
            "status".into(),
            FieldTimePattern {
                peak_hours: vec![10],
                peak_multiplier: 2.0,
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let now = Utc.with_ymd_and_hms(2024, 1, 1, 2, 0, 0).unwrap();
    let mut first = FieldFrequencyController::new(config.clone(), 31).unwrap();
    let mut second = FieldFrequencyController::new(config, 31).unwrap();
    let mut emitted = 0;
    for i in 0..100 {
        let time = now + Duration::seconds(i);
        let a = first.should_generate_field("status", time).unwrap();
        let b = second.should_generate_field("status", time).unwrap();
        assert_eq!(a, b);
        if a {
            emitted += 1;
            assert_eq!(
                first
                    .controlled_value("status", &HashMap::new(), time, || "x".into())
                    .unwrap(),
                second
                    .controlled_value("status", &HashMap::new(), time, || "x".into())
                    .unwrap()
            );
        }
    }
    assert!((30..=70).contains(&emitted));
    let invalid = FieldFrequencyConfig {
        burst_patterns: BTreeMap::from([(
            "x".into(),
            FieldBurstPattern {
                burst_probability: 1.1,
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    assert!(FieldFrequencyController::new(invalid, 0).is_err());
}

#[test]
fn failed_field_correlation_does_not_advance_time() {
    use log_forge::frequency::{FieldFrequencyConfig, FieldFrequencyController};
    use std::collections::HashMap;
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
    let mut controller =
        FieldFrequencyController::new(FieldFrequencyConfig::database(), 42).unwrap();
    let invalid = HashMap::from([("rows_affected".to_owned(), "invalid".to_owned())]);
    assert!(
        controller
            .controlled_value("query_duration", &invalid, time, || "default".into())
            .is_err()
    );
    let valid = HashMap::from([("rows_affected".to_owned(), "1001".to_owned())]);
    assert!(
        controller
            .controlled_value(
                "query_duration",
                &valid,
                time - Duration::seconds(1),
                || "default".into()
            )
            .is_ok()
    );
}

#[test]
fn error_frequency_overflow_does_not_advance_time() {
    let config = ErrorFrequencyConfig {
        burst_probability: 1.0,
        burst_duration_minutes: 1,
        ..ErrorFrequencyConfig::default()
    };
    let mut controller = ErrorFrequencyController::new(config, 42).unwrap();
    assert!(
        controller
            .should_generate(chrono::DateTime::<Utc>::MAX_UTC)
            .is_err()
    );
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    assert!(controller.should_generate(time).is_ok());
}
