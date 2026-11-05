use regex::Regex;
use std::process::Command;

#[test]
fn output_matches_original_generator_patterns() {
    let cases = [
        (
            "nginx_error",
            r#"^\d{4}/\d{2}/\d{2} \d{2}:\d{2}:\d{2} \[\w+\] \d+#\d+: \*\d+ .+, client: [\d.]+, server: [^,]+, request: "[^"]+", host: "[^"]+"$"#,
        ),
        (
            "apache_error",
            r"^\[[^\]]+\] \[\w+\] \[[^\]]+\] \[pid \d+\] \[client [\d.]+\] .+$",
        ),
        (
            "django_request",
            r#"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} \w+ [\w.]+: "[A-Z]+ [^"]+" \d+ \[\d+ms\] \(user: [^,]+, ip: [\d.]+\)$"#,
        ),
        (
            "django_sql",
            r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} \w+ [\w.]+: \([0-9.]+\) .+; args=.+$",
        ),
        (
            "docker",
            r#"^\{"log":".*","stream":"(stdout|stderr)","time":"[^"]+"\}$"#,
        ),
        (
            "kubernetes_pod",
            r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z \w+ .+$",
        ),
        (
            "kubernetes_event",
            r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z (Normal|Warning) \w+ \w+/[^\s]+ .+$",
        ),
        ("mysql_query", r"^\d{6} \d{2}:\d{2}:\d{2}\s+\d+\s+\w+\s+.+$"),
        (
            "mysql_error",
            r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z \[\w+\] .+$",
        ),
        (
            "mysql_slow",
            r"^# Time: \d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z\n# User@Host: [^\n]+\n# Thread_id: \d+\s+Schema: [^\n]*\n# Query_time: [\d.]+\s+Lock_time: [\d.]+\s+Rows_sent: \d+\s+Rows_examined: \d+\nSET timestamp=\d+;\n.+;$",
        ),
        (
            "postgresql",
            r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+ UTC \[\d+\]: \[\w+\] .+$",
        ),
        (
            "postgresql_slow",
            r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+ UTC \[\d+\]: \[LOG\] duration: [\d.]+ ms statement: .+$",
        ),
    ];
    let mut failures = Vec::new();
    for (kind, pattern) in cases {
        let result = Command::new(env!("CARGO_BIN_EXE_log-forge"))
            .args([
                "start",
                "--output",
                "console",
                "--types",
                kind,
                "--total",
                "1",
                "--start-time",
                "2024-01-01",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        if !Regex::new(pattern).unwrap().is_match(output.trim_end()) {
            failures.push(format!("{kind}: {output}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
