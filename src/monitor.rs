use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io::{self, BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct MonitorConfig {
    pub enable_statistics: bool,
    pub statistics_interval: f64,
    pub metrics: MetricsConfig,
    pub health_check: HealthConfig,
    pub service_metrics: Vec<ServiceMetricConfig>,
}
impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            enable_statistics: true,
            statistics_interval: 1.0,
            metrics: MetricsConfig::default(),
            health_check: HealthConfig::default(),
            service_metrics: Vec::new(),
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceMetricConfig {
    pub service: String,
    pub metrics: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct MetricsConfig {
    pub enabled: bool,
    format: String,
    endpoint: String,
    port: u16,
    labels: BTreeMap<String, String>,
}
impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            format: "prometheus".into(),
            endpoint: "/metrics".into(),
            port: 9090,
            labels: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct HealthConfig {
    pub enabled: bool,
    endpoint: String,
    port: u16,
}
impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "/health".into(),
            port: 8080,
        }
    }
}
impl MonitorConfig {
    pub fn validate(&self) -> io::Result<()> {
        if !self.statistics_interval.is_finite()
            || self.statistics_interval <= 0.0
            || Duration::try_from_secs_f64(self.statistics_interval).is_err()
            || Duration::from_secs_f64(self.statistics_interval).is_zero()
        {
            return Err(io::Error::other(
                "statistics_interval must be a positive finite duration",
            ));
        }
        if self.metrics.enabled {
            if self.metrics.format != "prometheus" {
                return Err(io::Error::other("invalid metrics format"));
            }
            crate::http_output::validate_target("localhost", &self.metrics.endpoint)?;
            for (name, value) in &self.metrics.labels {
                let mut chars = name.chars();
                if !chars
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
                    || name == "log_type"
                    || name == "service"
                    || name.starts_with("__")
                    || value.contains(['\n', '\r'])
                {
                    return Err(io::Error::other("invalid metrics label"));
                }
            }
        }
        if !self.service_metrics.is_empty() && !self.metrics.enabled {
            return Err(io::Error::other("service_metrics requires metrics.enabled"));
        }
        let mut services = std::collections::HashSet::new();
        for service in &self.service_metrics {
            if service.service.is_empty()
                || service.service.chars().any(char::is_whitespace)
                || !services.insert(&service.service)
                || service.metrics.is_empty()
            {
                return Err(io::Error::other(
                    "invalid or duplicate service_metrics service",
                ));
            }
            let mut names = std::collections::HashSet::new();
            for metric in &service.metrics {
                if ![
                    "request_count",
                    "response_time",
                    "error_rate",
                    "active_users",
                    "registration_rate",
                    "auth_failures",
                    "order_count",
                    "order_value",
                    "cancellation_rate",
                ]
                .contains(&metric.as_str())
                {
                    return Err(io::Error::other(format!(
                        "unsupported service metric: {metric}"
                    )));
                }
                if !names.insert(metric) {
                    return Err(io::Error::other("duplicate service metric"));
                }
            }
        }
        if self.health_check.enabled {
            crate::http_output::validate_target("localhost", &self.health_check.endpoint)?;
        }
        if self.metrics.enabled
            && self.health_check.enabled
            && self.metrics.port != 0
            && self.metrics.port == self.health_check.port
        {
            return Err(io::Error::other("metrics and health ports must differ"));
        }
        Ok(())
    }
}
#[derive(Default)]
struct ServiceValues {
    requests: u64,
    errors: u64,
    duration_ms: f64,
    users: HashMap<u32, Instant>,
    registrations: u64,
    auth_failures: u64,
    orders: u64,
    cancellations: u64,
    order_value: f64,
}
static USER_EVENT: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?i)\buser (\d+)\b").expect("valid user event pattern"));
static ORDER_CREATED: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\bOrder [^ ]+ created for user ").expect("valid order event pattern")
});
static ORDER_CANCELLED: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\bOrder [^ ]+ cancelled by user ")
        .expect("valid cancellation event pattern")
});
static ORDER_PAID: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\bPayment processed for order [^: ]+: amount ([0-9]+(?:\.[0-9]+)?)")
        .expect("valid payment event pattern")
});
impl ServiceValues {
    fn observe_events(&mut self, text: &str) -> io::Result<()> {
        if let Some(user) = USER_EVENT
            .captures(text)
            .and_then(|capture| capture[1].parse::<u32>().ok())
        {
            self.users
                .retain(|_, last| last.elapsed() < Duration::from_secs(300));
            if !self.users.contains_key(&user) && self.users.len() >= 100_000 {
                return Err(io::Error::other("active user limit exceeded"));
            }
            self.users.insert(user, Instant::now());
        }
        if USER_EVENT
            .find(text)
            .is_some_and(|matched| text[matched.end()..].starts_with(" registered successfully"))
        {
            self.registrations += 1;
        }
        if text.contains("Authentication failed for user ") {
            self.auth_failures += 1;
        }
        if ORDER_CREATED.is_match(text) {
            self.orders += 1;
        }
        if ORDER_CANCELLED.is_match(text) {
            self.cancellations += 1;
        }
        if let Some(amount) = ORDER_PAID
            .captures(text)
            .and_then(|capture| capture[1].parse::<f64>().ok())
            .filter(|amount| amount.is_finite())
        {
            self.order_value += amount;
        }
        Ok(())
    }
}

struct ServiceCounter {
    config: ServiceMetricConfig,
    values: Mutex<ServiceValues>,
    started: Instant,
}

struct Counter {
    name: String,
    value: AtomicU64,
}
pub(crate) struct Monitor {
    counts: Arc<Vec<Counter>>,
    services: Arc<Vec<ServiceCounter>>,
    stop: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<()>>,
    server_stop: Arc<AtomicBool>,
    servers: Vec<JoinHandle<()>>,
}
impl Monitor {
    pub fn new(
        names: impl IntoIterator<Item = String>,
        total: u64,
        progress: bool,
        show_statistics: bool,
        config: &MonitorConfig,
    ) -> io::Result<Self> {
        config.validate()?;
        let metrics_listener = if config.metrics.enabled {
            Some(TcpListener::bind((
                Ipv4Addr::LOCALHOST,
                config.metrics.port,
            ))?)
        } else {
            None
        };
        let health_listener = if config.health_check.enabled {
            Some(TcpListener::bind((
                Ipv4Addr::LOCALHOST,
                config.health_check.port,
            ))?)
        } else {
            None
        };
        let metrics_port = metrics_listener
            .as_ref()
            .map(|listener| listener.local_addr().map(|address| address.port()))
            .transpose()?;
        let health_port = health_listener
            .as_ref()
            .map(|listener| listener.local_addr().map(|address| address.port()))
            .transpose()?;
        let counts = Arc::new(
            names
                .into_iter()
                .map(|name| Counter {
                    name,
                    value: AtomicU64::new(0),
                })
                .collect::<Vec<_>>(),
        );
        let server_stop = Arc::new(AtomicBool::new(false));
        let mut monitor = Self {
            counts,
            services: Arc::new(
                config
                    .service_metrics
                    .iter()
                    .cloned()
                    .map(|config| ServiceCounter {
                        config,
                        values: Mutex::new(ServiceValues::default()),
                        started: Instant::now(),
                    })
                    .collect(),
            ),
            stop: None,
            worker: None,
            server_stop,
            servers: Vec::new(),
        };
        if let Some(listener) = metrics_listener {
            monitor.serve(
                listener,
                config.metrics.endpoint.clone(),
                Some(config.metrics.labels.clone()),
            )?;
        }
        if let Some(listener) = health_listener {
            monitor.serve(listener, config.health_check.endpoint.clone(), None)?;
        }
        if config.metrics.enabled || config.health_check.enabled {
            let mut message = "monitor HTTP endpoints ready".to_owned();
            if config.metrics.enabled && config.metrics.port == 0 {
                message.push_str(&format!(
                    " metrics={}",
                    metrics_port.ok_or_else(|| io::Error::other("metrics listener unavailable"))?
                ));
            }
            if config.health_check.enabled && config.health_check.port == 0 {
                message.push_str(&format!(
                    " health={}",
                    health_port.ok_or_else(|| io::Error::other("health listener unavailable"))?
                ));
            }
            eprintln!("{message}");
        }
        if show_statistics {
            let shared = Arc::clone(&monitor.counts);
            let interval = Duration::from_secs_f64(config.statistics_interval);
            let (stop, receive) = mpsc::channel();
            let worker = thread::Builder::new()
                .name("log-forge-monitor".into())
                .spawn(move || {
                    let started = Instant::now();
                    loop {
                        let done = !matches!(
                            receive.recv_timeout(interval),
                            Err(mpsc::RecvTimeoutError::Timeout)
                        );
                        let counts: Vec<_> = shared
                            .iter()
                            .map(|c| (c.name.as_str(), c.value.load(Ordering::Relaxed)))
                            .collect();
                        let generated: u64 = counts.iter().map(|(_, count)| count).sum();
                        let rate = generated as f64 / started.elapsed().as_secs_f64().max(0.001);
                        let breakdown = counts
                            .iter()
                            .map(|(name, count)| format!("{name}={count}"))
                            .collect::<Vec<_>>()
                            .join(" ");
                        let progress = if progress && total > 0 {
                            format!(" progress={generated}/{total}")
                        } else {
                            String::new()
                        };
                        eprintln!("generated={generated}{progress} rate={rate:.1}/s {breakdown}");
                        if done {
                            break;
                        }
                    }
                })?;
            monitor.stop = Some(stop);
            monitor.worker = Some(worker);
        }
        Ok(monitor)
    }
    fn serve(
        &mut self,
        listener: TcpListener,
        endpoint: String,
        labels: Option<BTreeMap<String, String>>,
    ) -> io::Result<()> {
        listener.set_nonblocking(true)?;
        let counts = Arc::clone(&self.counts);
        let services = Arc::clone(&self.services);
        let stop = Arc::clone(&self.server_stop);
        let worker = thread::Builder::new().name("log-forge-monitor-http".into()).spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if let Err(error) = stream.set_nonblocking(false) { eprintln!("monitor connection setup failed: {error}"); continue; }
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
                        let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
                        let mut request = String::new();
                        let mut reader = BufReader::new((&stream).take(8192));
                        let mut complete = reader.read_line(&mut request).is_ok();
                        while complete {
                            let mut header = String::new();
                            match reader.read_line(&mut header) {
                                Ok(0) | Err(_) => { complete = false; break; }
                                Ok(_) if header == "\r\n" || header == "\n" => break,
                                Ok(_) => {}
                            }
                        }
                        let valid = complete && request.split_whitespace().take(2).eq(["GET", endpoint.as_str()]);
                        let body = if valid {
                            if let Some(labels) = &labels {
                                let escape = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
                                let extra = labels.iter().map(|(key, value)| format!(",{key}=\"{}\"", escape(value))).collect::<String>();
                                let mut result = "# TYPE log_forge_records_total counter\n".to_owned();
                                for counter in counts.iter() { result.push_str(&format!("log_forge_records_total{{log_type=\"{}\"{extra}}} {}\n", escape(&counter.name), counter.value.load(Ordering::Relaxed))); }
                                for service in services.iter() {
                                    if let Ok(mut values) = service.values.lock() {
                                        values.users.retain(|_, last| last.elapsed() < Duration::from_secs(300));
                                        let attrs = format!("service=\"{}\"{extra}", escape(&service.config.service));
                                        for metric in &service.config.metrics {
                                            let (name, value) = match metric.as_str() {
                                                "request_count" => ("log_forge_service_requests_total", values.requests as f64),
                                                "response_time" => ("log_forge_service_response_time_ms", if values.requests == 0 { 0.0 } else { values.duration_ms / values.requests as f64 }),
                                                "error_rate" => ("log_forge_service_error_ratio", if values.requests == 0 { 0.0 } else { values.errors as f64 / values.requests as f64 }),
                                                "active_users" => ("log_forge_service_active_users", values.users.len() as f64),
                                                "registration_rate" => ("log_forge_service_registrations_per_second", values.registrations as f64 / service.started.elapsed().as_secs_f64().max(0.001)),
                                                "auth_failures" => ("log_forge_service_auth_failures_total", values.auth_failures as f64),
                                                "order_count" => ("log_forge_service_orders_total", values.orders as f64),
                                                "order_value" => ("log_forge_service_order_value_total", values.order_value),
                                                "cancellation_rate" => ("log_forge_service_cancellation_ratio", if values.orders == 0 { 0.0 } else { values.cancellations as f64 / values.orders as f64 }),
                                                _ => continue,
                                            };
                                            result.push_str(&format!("{name}{{{attrs}}} {value}\n"));
                                        }
                                    }
                                }
                                result
                            } else { "ok\n".into() }
                        } else { "not found\n".into() };
                        let status = if valid { "200 OK" } else { "404 Not Found" };
                        let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(20)),
                    Err(error) => { eprintln!("monitor HTTP server failed: {error}"); break; }
                }
            }
        })?;
        self.servers.push(worker);
        Ok(())
    }
    pub fn record(&self, index: usize, service: Option<&str>, record: &[u8]) -> io::Result<()> {
        self.counts[index].value.fetch_add(1, Ordering::Relaxed);
        let Some(service) = service else {
            return Ok(());
        };
        let Some(counter) = self
            .services
            .iter()
            .find(|entry| entry.config.service == service)
        else {
            return Ok(());
        };
        let text = std::str::from_utf8(record).map_err(io::Error::other)?;
        let mut values = counter
            .values
            .lock()
            .map_err(|_| io::Error::other("service metrics state poisoned"))?;
        values.observe_events(text)?;
        let Some((_, data)) = text.split_once(" method=GET endpoint=") else {
            return Ok(());
        };
        let mut status = None;
        let mut duration = None;
        for field in data.split_whitespace() {
            if let Some(value) = field.strip_prefix("status=") {
                status = value.parse::<u16>().ok();
            }
            if let Some(value) = field.strip_prefix("response_time_ms=") {
                duration = value.parse::<u64>().ok();
            }
        }
        let (Some(status), Some(duration)) = (status, duration) else {
            return Err(io::Error::other("invalid gateway metric fields"));
        };
        values.requests = values
            .requests
            .checked_add(1)
            .ok_or_else(|| io::Error::other("service request count overflow"))?;
        if status >= 500 {
            values.errors += 1;
        }
        values.duration_ms += duration as f64;
        Ok(())
    }
    pub fn finish(&mut self) -> io::Result<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| io::Error::other("monitor thread failed"))?;
        }
        self.server_stop.store(true, Ordering::Relaxed);
        for server in self.servers.drain(..) {
            server
                .join()
                .map_err(|_| io::Error::other("monitor HTTP thread failed"))?;
        }
        Ok(())
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_metrics_count_observed_user_and_order_events() {
        let mut values = ServiceValues::default();
        for message in [
            "User 123 registered successfully",
            "Profile updated for user 123",
            "Password changed for user 456",
            "Authentication failed for user alice",
            "Order ORD-123 created for user 123",
            "Order ORD-123 cancelled by user 123",
            "Payment processed for order ORD-123: amount 25.50",
        ] {
            values.observe_events(message).unwrap();
        }
        assert_eq!(values.users.len(), 2);
        assert_eq!(values.registrations, 1);
        assert_eq!(values.auth_failures, 1);
        assert_eq!(values.orders, 1);
        assert_eq!(values.cancellations, 1);
        assert_eq!(values.order_value, 25.5);
        values
            .users
            .insert(999, Instant::now() - Duration::from_secs(301));
        values
            .observe_events("Profile updated for user 456")
            .unwrap();
        assert!(!values.users.contains_key(&999));
    }
}
