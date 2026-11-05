#![cfg(unix)]
use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};

#[test]
fn process_plugin_runs_without_rebuild_and_validation_does_not_start_it() {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("plugin.sh");
    let marker = root.path().join("started");
    fs::write(&script, r#"#!/bin/sh
: > "$1"
count=0
while IFS= read -r line; do
    case "$line" in
        *'"action":"init"'*'"prefix":"example"'*'"seed":17'*) printf '%s\n' '{"version":1,"pattern":"^example-[0-9]+$"}' ;;
        *'"action":"generate"'*'"timestamp":'*) count=$((count + 1)); printf '{"version":1,"record":"example-%s"}\n' "$count" ;;
        *) printf '%s\n' '{"version":1,"error":"unexpected request"}' ;;
    esac
done
"#).unwrap();
    let config = root.path().join("config.yaml");
    let data = serde_yaml::to_string(&serde_json::json!({"log_generator": {"global": {"output_format":"console", "total_logs":3, "start_time":"2024-01-01"}, "log_types": {"example": {"plugin_command":"/bin/sh", "plugin_args":[script,marker], "plugin_config":{"prefix":"example"}, "plugin_timeout":2}}}})).unwrap();
    fs::write(&config, data).unwrap();
    let valid = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["config", "validate", "--file", config.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    assert!(!marker.exists());
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--seed",
            "17",
            "--threads",
            "2",
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "example-1\nexample-2\nexample-3\n"
    );
    assert!(marker.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("stateful generator runs"));
}

#[test]
fn process_plugin_timeout_and_protocol_errors_stop_generation() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let output = root.path().join("output.log");
    let write_config = |script: &str| {
        let data = serde_json::json!({"log_generator": {"global": {"output_format":"file", "output_path":output, "total_logs":1}, "log_types": {"example": {"plugin_command":"/bin/sh", "plugin_args":["-c", script], "plugin_timeout":0.2}}}});
        fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    };
    write_config("while IFS= read -r line; do :; done");
    let started = Instant::now();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(String::from_utf8_lossy(&result.stderr).contains("plugin response timed out"));
    assert!(!output.exists());
    write_config("IFS= read -r line; printf '%s\\n' '{\"version\":2,\"pattern\":\"^x$\"}'");
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("unsupported plugin protocol version")
    );
    assert!(!output.exists());
}

#[test]
fn process_plugin_write_timeout_does_not_hang() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.yaml");
    let payload = "x".repeat(200_000);
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"console", "total_logs":1}, "log_types": {"example": {"plugin_command":"/bin/sleep", "plugin_args":["10"], "plugin_config":{"payload":payload}, "plugin_timeout":0.2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let started = Instant::now();
    let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(String::from_utf8_lossy(&result.stderr).contains("plugin request timed out"));
}

#[test]
fn process_plugin_error_can_report_completed_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("plugin.sh");
    let marker = root.path().join("unexpected-shutdown");
    fs::write(
        &script,
        r#"
IFS= read -r line
printf '%s\n' '{"version":1,"pattern":"^record$","supports_shutdown":true}'
IFS= read -r line
printf '%s\n' '{"version":1,"error":"failure marker","shutdown_complete":true}'
if IFS= read -r line; then printf '%s' unexpected > "$1"; fi
"#,
    )
    .unwrap();
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"console", "total_logs":1}, "log_types": {"legacy": {"plugin_command":"/bin/sh", "plugin_args":[script, marker], "plugin_timeout":2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failure marker"));
    assert!(!stderr.contains("generator cleanup failed"), "{stderr}");
    assert!(!marker.exists());
}

#[test]
fn python_adapter_loads_original_style_generator_and_calls_cleanup() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 is unavailable; adapter test skipped");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let module = root.path().join("legacy_generator.py");
    fs::write(
        &module,
        r#"
class Generator:
    def __init__(self, config):
        self.prefix = config['prefix']
        self.cleanup_path = config['cleanup_path']
        self.suffix = ''
        print('plugin diagnostic')
    def set_custom_fields(self, fields):
        self.suffix = fields['suffix']
        if 201 not in fields['status_codes']:
            raise ValueError('status code key is not an integer')
    def get_log_pattern(self):
        return '^legacy .* done$'
    def validate_log(self, line):
        return line.startswith('legacy ')
    def generate_log(self):
        return 'unused'
    def generate_log_with_time(self, timestamp):
        return f'{self.prefix} {timestamp.isoformat()} {self.suffix}'
    def cleanup(self):
        with open(self.cleanup_path, 'w', encoding='utf-8') as output:
            output.write('closed')
"#,
    )
    .unwrap();
    let cleanup = root.path().join("cleanup.txt");
    let adapter =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/python_generator.py");
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"console", "total_logs":2, "generation_interval":1, "start_time":"2024-01-01"}, "log_types": {"legacy": {"plugin_command":"python3", "plugin_args":[adapter, "legacy_generator:Generator"], "plugin_config":{"prefix":"legacy", "cleanup_path":cleanup, "custom_fields":{"suffix":"done", "status_codes":{"201":1}}}, "plugin_timeout":2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--seed",
            "7",
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "legacy 2024-01-01T00:00:00+00:00 done\nlegacy 2024-01-01T00:00:01+00:00 done\n"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("plugin diagnostic"));
    assert_eq!(fs::read_to_string(cleanup).unwrap(), "closed");
}

#[test]
fn python_adapter_reports_cleanup_failure_and_missing_virtual_time() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("broken_cleanup.py"),
        r#"
class Generator:
    def generate_log(self): return 'legacy record'
    def get_log_pattern(self): return '^legacy record$'
    def validate_log(self, line): return line == 'legacy record'
    def cleanup(self): raise RuntimeError('cleanup failed')
"#,
    )
    .unwrap();
    let adapter =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/python_generator.py");
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"console", "total_logs":1}, "log_types": {"legacy": {"plugin_command":"python3", "plugin_args":[adapter, "broken_cleanup:Generator"], "plugin_timeout":2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "legacy record\n");
    let errors = String::from_utf8(output.stderr).unwrap();
    assert!(errors.contains("generator does not apply the requested timestamp"));
    assert!(errors.contains("plugin error: RuntimeError: cleanup failed"));
}

#[test]
fn python_adapter_passes_custom_config_to_legacy_constructor() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("custom_generator.py"),
        r#"
class Generator:
    def __init__(self, custom_config=None):
        self.prefix = custom_config['prefix']
        if custom_config['status_codes'] != {503: 1}:
            raise ValueError('status code key is not an integer')
    def generate_log(self): return self.prefix + ' record'
    def get_log_pattern(self): return '^configured record$'
    def validate_log(self, line): return line == 'configured record'
"#,
    )
    .unwrap();
    let adapter =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/python_generator.py");
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"console", "total_logs":1}, "log_types": {"legacy": {"plugin_command":"python3", "plugin_args":[adapter, "custom_generator:Generator"], "plugin_config":{"prefix":"configured", "status_codes":{"503":1}}, "plugin_timeout":2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "configured record\n"
    );
}

#[test]
fn python_adapter_passes_original_format_arguments() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("format_generator.py"),
        r#"
class Generator:
    def __init__(self, format_type='text', rfc_format='3164', custom_config=None):
        self.value = format_type + ' ' + rfc_format + ' ' + custom_config['prefix']
    def generate_log(self): return self.value
    def get_log_pattern(self): return '^json 5424 configured$'
    def validate_log(self, line): return line == 'json 5424 configured'
"#,
    )
    .unwrap();
    let adapter =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/python_generator.py");
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"console", "total_logs":1}, "log_types": {"legacy": {"plugin_command":"python3", "plugin_args":[adapter, "format_generator:Generator"], "plugin_config":{"prefix":"configured", "format_type":"json", "rfc_format":"5424"}, "plugin_timeout":2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "json 5424 configured\n"
    );
}

#[test]
fn python_adapter_cleans_up_when_pattern_is_invalid() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("invalid_pattern.py"),
        r#"
class Generator:
    def __init__(self, config): self.path = config['cleanup_path']
    def generate_log(self): return 'record'
    def get_log_pattern(self): return '['
    def validate_log(self, line): return True
    def cleanup(self):
        with open(self.path, 'w', encoding='utf-8') as output: output.write('closed')
"#,
    )
    .unwrap();
    let adapter =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/python_generator.py");
    let marker = root.path().join("closed");
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"console", "total_logs":1}, "log_types": {"legacy": {"plugin_command":"python3", "plugin_args":[adapter, "invalid_pattern:Generator"], "plugin_config":{"cleanup_path":marker}, "plugin_timeout":2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("regex parse error"));
    assert_eq!(fs::read_to_string(marker).unwrap(), "closed");
}

#[test]
fn python_adapter_cleans_up_when_pattern_lookup_fails() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("pattern_failure.py"),
        r#"
class Generator:
    def __init__(self, config): self.path = config['cleanup_path']
    def generate_log(self): return 'record'
    def get_log_pattern(self): raise RuntimeError('pattern lookup failed')
    def validate_log(self, line): return True
    def cleanup(self):
        with open(self.path, 'w', encoding='utf-8') as output: output.write('closed')
"#,
    )
    .unwrap();
    let adapter =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/python_generator.py");
    let marker = root.path().join("closed");
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"console", "total_logs":1}, "log_types": {"legacy": {"plugin_command":"python3", "plugin_args":[adapter, "pattern_failure:Generator"], "plugin_config":{"cleanup_path":marker}, "plugin_timeout":2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("pattern lookup failed"));
    assert_eq!(fs::read_to_string(marker).unwrap(), "closed");
}

#[test]
fn python_adapter_cleans_up_after_post_constructor_initialization_error() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("bad_init.py"),
        r#"
class Generator:
    def __init__(self, config): self.path = config['cleanup_path']
    def set_custom_fields(self, fields): raise RuntimeError('field setup failed')
    def generate_log(self): return 'record'
    def get_log_pattern(self): return '^record$'
    def validate_log(self, line): return True
    def cleanup(self):
        with open(self.path, 'w', encoding='utf-8') as output: output.write('closed')
"#,
    )
    .unwrap();
    let adapter =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/python_generator.py");
    let marker = root.path().join("closed");
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"console", "total_logs":1}, "log_types": {"legacy": {"plugin_command":"python3", "plugin_args":[adapter, "bad_init:Generator"], "plugin_config":{"cleanup_path":marker, "custom_fields":{"enabled":true}}, "plugin_timeout":2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("field setup failed"));
    assert_eq!(fs::read_to_string(marker).unwrap(), "closed");
    fs::write(
        root.path().join("bad_init.py"),
        r#"
class Generator:
    def __init__(self, config): pass
    def set_custom_fields(self, fields): raise RuntimeError('field setup failed')
    def cleanup(self): raise RuntimeError('cleanup failed too')
"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("field setup failed"), "{error}");
    assert!(error.contains("cleanup failed too"), "{error}");
}

#[test]
fn earlier_plugin_is_cleaned_up_when_later_initialization_fails() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("plugins.py"),
        r#"
class Good:
    def __init__(self, config): self.path = config['cleanup_path']
    def generate_log(self): return 'record'
    def get_log_pattern(self): return '^record$'
    def validate_log(self, line): return True
    def cleanup(self):
        with open(self.path, 'w', encoding='utf-8') as output: output.write('closed')
class Bad:
    def __init__(self): raise RuntimeError('initialization failed')
"#,
    )
    .unwrap();
    let adapter =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/python_generator.py");
    let marker = root.path().join("closed");
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"console", "total_logs":1}, "log_types": {
        "a_good": {"plugin_command":"python3", "plugin_args":[adapter, "plugins:Good"], "plugin_config":{"cleanup_path":marker}, "plugin_timeout":2},
        "z_bad": {"plugin_command":"python3", "plugin_args":[adapter, "plugins:Bad"], "plugin_timeout":2}
    }}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("initialization failed"));
    assert_eq!(fs::read_to_string(marker).unwrap(), "closed");
}

#[test]
fn plugin_cleanup_runs_when_output_rotation_fails() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("cleanup_plugin.py"),
        r#"
class Generator:
    def __init__(self, config): self.path = config['cleanup_path']
    def generate_log(self): return 'record'
    def get_log_pattern(self): return '^record$'
    def validate_log(self, line): return True
    def cleanup(self):
        with open(self.path, 'w', encoding='utf-8') as output: output.write('closed')
"#,
    )
    .unwrap();
    let adapter =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/python_generator.py");
    let marker = root.path().join("closed");
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"file", "output_path":root.path().join("output.log"), "total_logs":4, "rotation_size":1, "max_files":1}, "log_types": {"legacy": {"plugin_command":"python3", "plugin_args":[adapter, "cleanup_plugin:Generator"], "plugin_config":{"cleanup_path":marker}, "plugin_timeout":2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("rotation limit reached"));
    assert_eq!(fs::read_to_string(marker).unwrap(), "closed");
}

#[test]
fn output_and_plugin_cleanup_failures_are_both_reported() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("cleanup_failure.py"),
        r#"
class Generator:
    def generate_log(self): return 'record'
    def get_log_pattern(self): return '^record$'
    def validate_log(self, line): return True
    def cleanup(self): raise RuntimeError('cleanup failure marker')
"#,
    )
    .unwrap();
    let adapter =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/python_generator.py");
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"file", "output_path":root.path().join("output.log"), "total_logs":4, "rotation_size":1, "max_files":1}, "log_types": {"legacy": {"plugin_command":"python3", "plugin_args":[adapter, "cleanup_failure:Generator"], "plugin_timeout":2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("rotation limit reached"), "{stderr}");
    assert!(stderr.contains("cleanup failure marker"), "{stderr}");
}

#[test]
fn python_adapter_cleans_up_after_generation_error() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("generation_failure.py"),
        r#"
class Generator:
    def __init__(self, config): self.path = config['cleanup_path']
    def generate_log(self): raise RuntimeError('generation failure marker')
    def get_log_pattern(self): return '^record$'
    def validate_log(self, line): return True
    def cleanup(self):
        with open(self.path, 'w', encoding='utf-8') as output: output.write('closed')
"#,
    )
    .unwrap();
    let adapter =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/python_generator.py");
    let marker = root.path().join("closed");
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"console", "total_logs":1}, "log_types": {"legacy": {"plugin_command":"python3", "plugin_args":[adapter, "generation_failure:Generator"], "plugin_config":{"cleanup_path":marker}, "plugin_timeout":2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("generation failure marker"));
    assert!(!stderr.contains("generator cleanup failed"), "{stderr}");
    assert_eq!(fs::read_to_string(marker).unwrap(), "closed");
}

#[test]
fn stopping_process_plugin_flushes_output_and_runs_cleanup() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("stoppable.py"),
        r#"
class Generator:
    def __init__(self, config): self.ready, self.closed = config['ready'], config['closed']
    def generate_log(self):
        with open(self.ready, 'w', encoding='utf-8') as output: output.write('ready')
        return 'record'
    def get_log_pattern(self): return '^record$'
    def validate_log(self, line): return True
    def cleanup(self):
        with open(self.closed, 'w', encoding='utf-8') as output: output.write('closed')
"#,
    )
    .unwrap();
    let adapter =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/python_generator.py");
    let ready = root.path().join("ready");
    let closed = root.path().join("closed");
    let socket = root.path().join("control.sock");
    let log = root.path().join("output.log");
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"file", "output_path":log, "total_logs":0, "virtual_time":false, "generation_interval":60, "async_io":true}, "log_types": {"legacy": {"plugin_command":"python3", "plugin_args":[adapter, "stoppable:Generator"], "plugin_config":{"ready":ready, "closed":closed}, "plugin_timeout":2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--control-socket",
            socket.to_str().unwrap(),
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() || !socket.exists() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("plugin did not start");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let stop = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args(["stop", "--control-socket", socket.to_str().unwrap()])
        .output()
        .unwrap();
    if !stop.status.success() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("{}", String::from_utf8_lossy(&stop.stderr));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("plugin did not stop");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fs::read_to_string(closed).unwrap(), "closed");
    assert_eq!(fs::read_to_string(log).unwrap(), "record\n");
}

#[test]
fn python_adapter_reports_generation_and_cleanup_errors() {
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("dual_failure.py"),
        r#"
class Generator:
    def generate_log(self): raise RuntimeError('generation failure marker')
    def get_log_pattern(self): return '^record$'
    def validate_log(self, line): return True
    def cleanup(self): raise RuntimeError('cleanup failure marker')
"#,
    )
    .unwrap();
    let adapter =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("adapters/python_generator.py");
    let config = root.path().join("config.yaml");
    let data = serde_json::json!({"log_generator": {"global": {"output_format":"console", "total_logs":1}, "log_types": {"legacy": {"plugin_command":"python3", "plugin_args":[adapter, "dual_failure:Generator"], "plugin_timeout":2}}}});
    fs::write(&config, serde_yaml::to_string(&data).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_log-forge"))
        .args([
            "start",
            "--config",
            config.to_str().unwrap(),
            "--no-monitor",
        ])
        .env("PYTHONPATH", root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("generation failure marker"), "{stderr}");
    assert!(stderr.contains("cleanup failure marker"), "{stderr}");
}
