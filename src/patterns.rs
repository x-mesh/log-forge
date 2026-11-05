use chrono::{DateTime, Utc};
use rand::{Rng, rngs::StdRng};
use regex::Regex;
use std::{collections::BTreeMap, sync::LazyLock};

static PATTERNS: LazyLock<BTreeMap<String, Vec<String>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/generator_patterns.json"))
        .expect("valid built-in generator patterns")
});
static SYSLOG: LazyLock<BTreeMap<String, Vec<String>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/syslog_patterns.json"))
        .expect("valid built-in syslog patterns")
});
static FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{([a-zA-Z_][a-zA-Z_0-9]*)(?::[^}]*)?\}").expect("valid syslog field pattern")
});

pub(crate) fn load_builtins() {
    LazyLock::force(&PATTERNS);
    LazyLock::force(&SYSLOG);
    crate::goloop::load_patterns();
}

pub(crate) fn choose(name: &str, rng: &mut StdRng) -> &'static str {
    let patterns = PATTERNS.get(name).expect("built-in pattern group exists");
    &patterns[rng.gen_range(0..patterns.len())]
}

pub(crate) fn render(template: &str, values: &[(&str, String)]) -> Result<String, String> {
    let mut message = template.to_owned();
    for field in FIELD.captures_iter(template) {
        let name = field.get(1).expect("pattern field capture").as_str();
        let value = values
            .iter()
            .find_map(|(key, value)| (*key == name).then_some(value))
            .ok_or_else(|| format!("unknown built-in pattern field: {name}"))?;
        message = message.replace(&field[0], value);
    }
    Ok(message)
}

pub(crate) fn validate_docker_template(template: &str) -> Result<(), String> {
    if template.is_empty() || template.contains(['\n', '\r']) {
        return Err("invalid docker message template".into());
    }
    let names = [
        "port",
        "host",
        "ip",
        "request_id",
        "job_id",
        "user",
        "key",
        "duration",
        "service",
        "config_file",
        "endpoint",
        "pool_size",
        "size",
        "file",
        "error",
        "available",
        "domain",
        "method",
        "path",
        "query",
        "client_ip",
        "client_port",
    ];
    let values: Vec<_> = names
        .iter()
        .map(|name| (*name, "value".to_owned()))
        .collect();
    render(template, &values).map(|_| ())
}

pub(crate) fn syslog_message(
    facility: &str,
    time: DateTime<Utc>,
    ip: &str,
    custom: Option<&[String]>,
    rng: &mut StdRng,
) -> Result<String, String> {
    let Some(patterns) = custom.or_else(|| SYSLOG.get(facility).map(Vec::as_slice)) else {
        // Some valid syslog facilities have no patterns in the original library.
        return Ok(format!("Generic {facility} message"));
    };
    let template = &patterns[rng.gen_range(0..patterns.len())];
    render_syslog(template, time, ip, rng)
}

pub(crate) fn render_syslog(
    template: &str,
    time: DateTime<Utc>,
    ip: &str,
    rng: &mut StdRng,
) -> Result<String, String> {
    let mut message = template.to_owned();
    let mut values: BTreeMap<&str, String> = BTreeMap::new();
    for capture in FIELD.captures_iter(template) {
        let key = capture.get(1).expect("syslog field capture").as_str();
        let value = if let Some(value) = values.get(key) {
            value.clone()
        } else {
            let value = match key {
                "ip" => ip.to_owned(),
                "ip2" => format!("198.51.100.{}", rng.gen_range(1..=254)),
                "epoch" => time.timestamp().to_string(),
                "time" | "timestamp" => time.format("%Y-%m-%d %H:%M:%S").to_string(),
                "uptime" => format!("{:.6}", rng.gen_range(0.0..86400.0)),
                "http_status" => {
                    [200, 201, 204, 301, 400, 401, 403, 404, 500][rng.gen_range(0..9)].to_string()
                }
                "port" | "sport" => rng.gen_range(1024..=65535).to_string(),
                "dport" | "ip_id" => rng.gen_range(1..=65535).to_string(),
                "pid" => rng.gen_range(1000..=99999).to_string(),
                "score" => rng.gen_range(0..=1000).to_string(),
                "cpu" => rng.gen_range(0..=7).to_string(),
                "code" => [0, 1, 2, 130, 143][rng.gen_range(0..5)].to_string(),
                "errno" => rng.gen_range(1..=255).to_string(),
                "tty" => rng.gen_range(0..=9).to_string(),
                "nrcpt" => rng.gen_range(1..=5).to_string(),
                "len" => rng.gen_range(40..=1500).to_string(),
                "ttl" => rng.gen_range(32..=255).to_string(),
                "window" => rng.gen_range(512..=65535).to_string(),
                "sector" => rng.gen_range(8..=10000000).to_string(),
                "msec" => rng.gen_range(100000..=999999).to_string(),
                "uid" | "gid" => rng.gen_range(1000..=60000).to_string(),
                "ses" => rng.gen_range(1..=999).to_string(),
                "latency" => rng.gen_range(1..=500).to_string(),
                "latency_thresh" => [100, 200, 250][rng.gen_range(0..3)].to_string(),
                "sample_rate" => [50000, 25000, 10000][rng.gen_range(0..3)].to_string(),
                "bytes_in" | "bytes_out" => rng.gen_range(100..=1000000).to_string(),
                "wid" => rng.gen_range(1..=16).to_string(),
                "rtt" => rng.gen_range(1..=250).to_string(),
                "duration" => rng.gen_range(1..=600).to_string(),
                "removed" => rng.gen_range(1..=50).to_string(),
                "kept" => rng.gen_range(10..=100).to_string(),
                "bufsize" => [65536, 131072, 262144][rng.gen_range(0..3)].to_string(),
                "process" | "process_name" => {
                    ["apache2", "nginx", "mysql", "postgres", "python3"][rng.gen_range(0..5)].into()
                }
                "service" => {
                    ["nginx", "apache2", "mysql", "postgresql", "redis"][rng.gen_range(0..5)].into()
                }
                "user" | "username" => format!("user{}", rng.gen_range(1..=10000)),
                "hostname" | "fqdn" | "target" => {
                    format!("host{}.example.test", rng.gen_range(1..=10000))
                }
                "address" | "sp" => format!("0x{:x}", rng.gen_range(0x1000..=0xffffffff_u32)),
                "size" => format!("0x{:x}", rng.gen_range(0x1000..=0x10000_u32)),
                "status" => ["charging", "discharging", "full"][rng.gen_range(0..3)].into(),
                "error" => ["Connection refused", "Permission denied", "No such file"]
                    [rng.gen_range(0..3)]
                .into(),
                "pwd" => ["/home/user", "/var/log", "/etc", "/tmp"][rng.gen_range(0..4)].into(),
                "command" => {
                    ["ls -la", "cat /etc/passwd", "systemctl status"][rng.gen_range(0..3)].into()
                }
                "queue_id" | "queue_id2" => format!("Q{:08X}", rng.r#gen::<u32>()),
                "message_id" => format!(
                    "{:08x}.{}@example.test",
                    rng.r#gen::<u32>(),
                    rng.gen_range(1000000000_u64..=9999999999)
                ),
                "from" => format!("sender{}@example.test", rng.gen_range(1..=10000)),
                "to" => format!("recipient{}@example.test", rng.gen_range(1..=10000)),
                "relay" => format!(
                    "mail{}.example.test[192.0.2.{}]:{}",
                    rng.gen_range(1..=1000),
                    rng.gen_range(1..=254),
                    rng.gen_range(25..=587)
                ),
                "file" | "crontab" => {
                    ["/etc/crontab", "/var/spool/cron/crontabs/root"][rng.gen_range(0..2)].into()
                }
                "version" => format!(
                    "{}.{}.{}",
                    rng.gen_range(8..=9),
                    rng.gen_range(1..=50),
                    rng.gen_range(0..=10)
                ),
                "iface" => ["eth0", "ens33", "enp0s3", "wlan0"][rng.gen_range(0..4)].into(),
                "link_state" => ["up", "down"][rng.gen_range(0..2)].into(),
                "mac" => (0..6)
                    .map(|_| format!("{:02x}", rng.gen_range(0..=255)))
                    .collect::<Vec<_>>()
                    .join(":"),
                "proto" => ["TCP", "UDP", "ICMP"][rng.gen_range(0..3)].into(),
                "tcp_flags" => ["SYN", "SYN,ACK", "ACK", "FIN", "RST"][rng.gen_range(0..5)].into(),
                "disk" => ["sda", "sdb", "nvme0n1"][rng.gen_range(0..3)].into(),
                "mount_opts" => {
                    ["errors=remount-ro", "noatime", "barrier=1"][rng.gen_range(0..3)].into()
                }
                "nm_old_state" => {
                    ["disconnected", "connecting", "connected"][rng.gen_range(0..3)].into()
                }
                "nm_new_state" => ["connected", "disconnecting"][rng.gen_range(0..2)].into(),
                "container_id" => format!("{:012x}", rng.gen_range(0..(1_u64 << 48))),
                "image" => ["nginx:1.25", "redis:7", "alpine:3.20"][rng.gen_range(0..3)].into(),
                "delay" => format!("{:.2}", rng.gen_range(0.1..10.0)),
                "delays" => (0..4)
                    .map(|_| format!("{:.1}", rng.gen_range(0.0..3.0)))
                    .collect::<Vec<_>>()
                    .join(","),
                "home" => format!("/home/user{}", rng.gen_range(1..=10000)),
                "req_id" => format!("{:08x}", rng.r#gen::<u32>()),
                "url_path" => ["/api/v1/health", "/login", "/orders"][rng.gen_range(0..3)].into(),
                "cache_op" => ["GET", "SET", "DEL"][rng.gen_range(0..3)].into(),
                "cache_key" => format!("key-{:08x}", rng.r#gen::<u32>()),
                "cache_result" => ["hit", "miss", "set"][rng.gen_range(0..3)].into(),
                "load_avg" => format!("{:.2}", rng.gen_range(0.0..8.0)),
                "reason" => ["OOM", "SIGSEGV", "healthcheck-fail"][rng.gen_range(0..3)].into(),
                "up" => ["True", "False"][rng.gen_range(0..2)].into(),
                "alert_id" => format!("alert-{:04x}", rng.r#gen::<u16>()),
                "sev" => ["low", "medium", "high", "critical"][rng.gen_range(0..4)].into(),
                "rule" => ["disk_usage > 90%", "latency_p95 > 200ms"][rng.gen_range(0..2)].into(),
                "snapshot_id" => format!("snap-{:08x}", rng.r#gen::<u32>()),
                "fingerprint" => format!("{:032x}{:08x}", rng.r#gen::<u128>(), rng.r#gen::<u32>()),
                "path" => ["/home", "/var/log", "/etc/ssh"][rng.gen_range(0..3)].into(),
                _ => return Err(format!("unknown built-in syslog field: {key}")),
            };
            values.insert(key, value.clone());
            value
        };
        message = message.replace(&capture[0], &value);
    }
    Ok(message)
}
