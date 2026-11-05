use std::process::{Command, Output};

fn log_forge(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn subcommand_help_names_the_command_and_describes_it() {
    for command in [
        "start",
        "config",
        "list-types",
        "sample",
        "validate",
        "analyze",
        "status",
        "monitor",
    ] {
        let output = log_forge(&[command, "--help"]);
        assert!(output.status.success(), "{command}");
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(
            help.contains(&format!("Usage: log-forge {command}")),
            "{help}"
        );
        assert!(!help.starts_with("Usage:"), "{command} has no description");
        for line in help.lines() {
            let line = line.trim();
            if line.starts_with("--") || line.starts_with('-') {
                assert!(
                    line.split("  ").filter(|part| !part.is_empty()).count() > 1,
                    "{command}: option without description: {line}"
                );
            }
        }
    }
}

#[test]
fn mistyped_command_suggests_the_closest_command() {
    let output = log_forge(&["strat"]);
    assert_eq!(output.status.code(), Some(2));
    let message = String::from_utf8(output.stderr).unwrap();
    assert!(message.contains("'strat'"), "{message}");
    assert!(message.contains("'start'"), "{message}");
}

#[test]
fn unknown_log_type_suggests_a_type_and_the_list_command() {
    let output = log_forge(&[
        "start",
        "--output",
        "console",
        "--types",
        "rediss",
        "--total",
        "1",
        "--no-monitor",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let message = String::from_utf8(output.stderr).unwrap();
    assert!(message.contains("unknown log type: rediss"), "{message}");
    assert!(message.contains("'redis'"), "{message}");
    assert!(message.contains("log-forge list-types"), "{message}");
}

#[test]
fn missing_input_reports_the_path() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing.log");
    for command in ["validate", "analyze"] {
        let output = log_forge(&[command, "--input", missing.to_str().unwrap()]);
        assert_eq!(output.status.code(), Some(1));
        let message = String::from_utf8(output.stderr).unwrap();
        assert!(message.contains("input not found"), "{message}");
        assert!(message.contains("missing.log"), "{message}");
        assert!(!message.contains("cannot infer"), "{message}");
    }
}

#[cfg(unix)]
#[test]
fn closed_stdout_ends_quietly() {
    use std::io::Read;
    use std::process::Stdio;

    let root = tempfile::tempdir().unwrap();
    let log = root.path().join("nginx_access_20240101.log");
    let generated = log_forge(&[
        "--total",
        "20000",
        "--output",
        log.to_str().unwrap(),
        "--seed",
        "1",
    ]);
    assert!(generated.status.success());

    let commands: [&[&str]; 2] = [
        &[
            "start",
            "--output",
            "console",
            "--types",
            "redis",
            "--total",
            "1000000",
            "--no-monitor",
        ],
        &["analyze", "--input", log.to_str().unwrap(), "--pretty"],
    ];
    for args in commands {
        let mut child = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut first = [0_u8; 16];
        child.stdout.take().unwrap().read_exact(&mut first).unwrap();
        let output = child.wait_with_output().unwrap();
        let message = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{args:?}: {message}");
        assert!(message.is_empty(), "{args:?}: {message}");
    }
}

#[cfg(unix)]
#[test]
fn closed_stdout_ends_monitor_quietly() {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let root = tempfile::tempdir().unwrap();
    let socket = std::env::temp_dir().join(format!("lf-{}.sock", std::process::id()));
    let mut generator = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["start", "--output", "file", "--path"])
        .arg(root.path().join("monitor.log"))
        .args([
            "--types",
            "redis",
            "--total",
            "0",
            "--real-time",
            "--interval",
            "0.5",
            "--no-monitor",
            "--control-socket",
        ])
        .arg(&socket)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !socket.exists() {
        assert!(Instant::now() < deadline, "control socket was not created");
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut monitor = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["monitor", "--refresh", "0.05", "--control-socket"])
        .arg(&socket)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut first = [0_u8; 1];
    monitor
        .stdout
        .take()
        .unwrap()
        .read_exact(&mut first)
        .unwrap();
    let output = monitor.wait_with_output().unwrap();

    let stopped = log_forge(&["stop", "--control-socket", socket.to_str().unwrap()]);
    generator.wait().unwrap();
    assert!(stopped.status.success());
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{message}");
    assert!(message.is_empty(), "{message}");
}
