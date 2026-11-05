#![cfg(windows)]

use std::{
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct RunningChild(Child);

impl Drop for RunningChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn control_commands_work_across_processes() {
    let dir = tempfile::tempdir().unwrap();
    let output_path = dir.path().join("output.log");
    let name = format!("log-forge-integration-{}", std::process::id());
    let mut child = RunningChild(
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["start", "--output", "file", "--path"])
            .arg(&output_path)
            .args([
                "--types",
                "redis",
                "--total",
                "0",
                "--real-time",
                "--interval",
                "0.1",
                "--no-monitor",
                "--control-socket",
                &name,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let request = |action: &str| {
        Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([action, "--control-socket", &name])
            .output()
            .unwrap()
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if request("status").status.success() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "control pipe did not become ready"
        );
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "generator exited before control was ready"
        );
        thread::sleep(Duration::from_millis(50));
    }
    let call = |action: &str| {
        let output = request(action);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    assert_eq!(call("pause")["state"], "paused");
    thread::sleep(Duration::from_millis(200));
    let paused = call("status")["generated"].as_u64().unwrap();
    thread::sleep(Duration::from_millis(250));
    assert_eq!(call("status")["generated"].as_u64().unwrap(), paused);
    assert_eq!(call("resume")["state"], "running");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if call("status")["generated"].as_u64().unwrap() > paused {
            break;
        }
        assert!(Instant::now() < deadline, "generation did not resume");
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(call("stop")["state"], "stopping");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "generator did not stop");
        thread::sleep(Duration::from_millis(20));
    }
    assert!(!request("status").status.success());
    assert!(
        std::fs::read_to_string(output_path)
            .unwrap()
            .contains(":M ")
    );
}

#[test]
fn control_commands_reject_non_simple_pipe_names() {
    for name in ["nested/name", "nested\\name", "drive:name"] {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args(["status", "--control-socket", name])
            .output()
            .unwrap();
        assert!(!result.status.success(), "accepted {name}");
        assert!(
            String::from_utf8_lossy(&result.stderr)
                .contains("control pipe name must be a simple name"),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
