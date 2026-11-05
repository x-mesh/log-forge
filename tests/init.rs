use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

fn log_forge(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn init_with_yes_creates_a_configuration_that_start_runs() {
    let root = tempfile::tempdir().unwrap();
    let created = log_forge(
        root.path(),
        &[
            "init",
            "--types",
            "nginx_access=0.7,syslog=0.3",
            "--total",
            "100",
            "--start-time",
            "2024-01-01 00:00:00",
            "--yes",
        ],
    );
    let stderr = String::from_utf8_lossy(&created.stderr);
    assert!(created.status.success(), "{stderr}");
    assert_eq!(
        String::from_utf8_lossy(&created.stdout),
        "Configuration created: config.yaml\n"
    );
    assert!(
        stderr.contains("next: log-forge start --config config.yaml"),
        "{stderr}"
    );

    let validated = log_forge(
        root.path(),
        &["config", "validate", "--file", "config.yaml"],
    );
    assert!(validated.status.success());

    let started = log_forge(
        root.path(),
        &[
            "start",
            "--config",
            "config.yaml",
            "--seed",
            "1",
            "--no-monitor",
        ],
    );
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let logs = root.path().join("logs");
    let records: usize = fs::read_dir(&logs)
        .unwrap()
        .map(|entry| {
            fs::read_to_string(entry.unwrap().path())
                .unwrap()
                .lines()
                .count()
        })
        .sum();
    assert_eq!(records, 100);
    assert!(logs.join("nginx_access_20240101.log").is_file());
    assert!(logs.join("syslog_20240101.log").is_file());
}

#[test]
fn init_writes_json_for_a_json_file() {
    let root = tempfile::tempdir().unwrap();
    let created = log_forge(
        root.path(),
        &[
            "init",
            "--file",
            "conf/gen.json",
            "--output",
            "console",
            "--yes",
        ],
    );
    assert!(created.status.success());
    let text = fs::read_to_string(root.path().join("conf/gen.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["log_generator"]["global"]["output_format"], "console");
    assert!(value["log_generator"]["global"]["output_path"].is_null());
}

#[test]
fn init_without_a_terminal_asks_for_yes() {
    let root = tempfile::tempdir().unwrap();
    let output = log_forge(root.path(), &["init"]);
    assert_eq!(output.status.code(), Some(2));
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("init --yes"), "{message}");
    assert!(!root.path().join("config.yaml").exists());
}

#[test]
fn init_keeps_an_existing_file() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("config.yaml"), "keep\n").unwrap();
    let output = log_forge(root.path(), &["init", "--yes"]);
    assert_eq!(output.status.code(), Some(1));
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("config.yaml already exists"), "{message}");
    assert_eq!(
        fs::read_to_string(root.path().join("config.yaml")).unwrap(),
        "keep\n"
    );
}

#[test]
fn init_rejects_an_unknown_type_without_a_file() {
    let root = tempfile::tempdir().unwrap();
    let output = log_forge(root.path(), &["init", "--types", "rediss", "--yes"]);
    assert_eq!(output.status.code(), Some(1));
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("did you mean 'redis'"), "{message}");
    assert!(!root.path().join("config.yaml").exists());
}
