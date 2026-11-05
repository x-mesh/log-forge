use super::{Args, Format, LogType, generate_with_level};
use crate::GeneratorRegistry;
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use clap::{Parser, ValueEnum};
use rand::{
    Rng, SeedableRng,
    distributions::{Distribution, WeightedIndex},
    rngs::StdRng,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    error::Error,
    fs::{self, OpenOptions},
    io::{self, BufWriter, IsTerminal, Write},
    net::{Ipv4Addr, ToSocketAddrs, UdpSocket},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

type ActiveSignal = Option<(Arc<AtomicBool>, std::thread::Thread)>;

struct BuiltinGenerator {
    args: Args,
    rng: StdRng,
    index: u64,
    validator: crate::validation::LogValidator,
}

fn initialize_docker(settings: &mut GeneratorFields, seed: u64) {
    let mut chooser = StdRng::seed_from_u64(seed);
    if settings.docker_name.is_none() {
        settings.docker_name = Some(if settings.docker_names.is_empty() {
            let defaults = [
                "web-server",
                "api-backend",
                "database",
                "cache-redis",
                "worker-queue",
                "nginx-proxy",
                "app-frontend",
                "monitoring",
                "logging-agent",
                "load-balancer",
            ];
            defaults[chooser.gen_range(0..defaults.len())].to_owned()
        } else {
            settings.docker_names[chooser.gen_range(0..settings.docker_names.len())].clone()
        });
    }
    if let Some(name) = &mut settings.docker_name {
        *name = name.replace("{instance_id}", &chooser.gen_range(1..=999).to_string());
    }
    if !settings.docker_drivers.is_empty() {
        settings.docker_driver = Some(
            settings.docker_drivers[chooser.gen_range(0..settings.docker_drivers.len())].clone(),
        );
    }
    if settings.docker_container_id.is_empty() {
        settings.docker_container_id = format!(
            "{:032x}{:032x}",
            chooser.r#gen::<u128>(),
            chooser.r#gen::<u128>()
        );
    }
    if settings.docker_image_name.is_empty() {
        let defaults = [
            "nginx:latest",
            "redis:alpine",
            "postgres:13",
            "node:16-alpine",
            "python:3.9-slim",
            "mysql:8.0",
            "ubuntu:20.04",
            "alpine:latest",
            "mongo:4.4",
            "elasticsearch:7.15.0",
        ];
        settings.docker_image_name = if settings.docker_images.is_empty() {
            defaults[chooser.gen_range(0..defaults.len())].to_owned()
        } else {
            settings.docker_images[chooser.gen_range(0..settings.docker_images.len())].clone()
        };
    }
}

fn initialize_kubernetes_pod(settings: &mut GeneratorFields, seed: u64) {
    let mut rng = StdRng::seed_from_u64(seed);
    if settings.kubernetes_namespace.is_none() {
        settings.kubernetes_namespace = Some(if settings.kubernetes_namespaces.is_empty() {
            let defaults = [
                "default",
                "kube-system",
                "kube-public",
                "monitoring",
                "logging",
                "ingress-nginx",
                "cert-manager",
                "prometheus",
                "grafana",
                "elasticsearch",
            ];
            defaults[rng.gen_range(0..defaults.len())].to_owned()
        } else {
            settings.kubernetes_namespaces[rng.gen_range(0..settings.kubernetes_namespaces.len())]
                .clone()
        });
    }
    if settings.kubernetes_pod_name.is_none() {
        let prefixes = [
            "web-app",
            "api-server",
            "database",
            "redis-cache",
            "worker",
            "nginx-ingress",
            "prometheus",
            "grafana",
            "elasticsearch",
            "fluentd",
        ];
        settings.kubernetes_pod_name = Some(format!(
            "{}-{:010x}",
            prefixes[rng.gen_range(0..prefixes.len())],
            rng.gen_range(0..(1_u64 << 40))
        ));
    }
    if settings.kubernetes_container_name.is_none() {
        let containers = [
            "app",
            "sidecar",
            "proxy",
            "init",
            "monitoring",
            "logging",
            "backup",
            "health-check",
        ];
        settings.kubernetes_container_name =
            Some(containers[rng.gen_range(0..containers.len())].to_owned());
    }
    let applications = ["web", "api", "database", "cache"];
    let environments = ["production", "staging", "development"];
    let tiers = ["frontend", "backend", "database"];
    let mut labels = BTreeMap::from([
        (
            "app".into(),
            applications[rng.gen_range(0..applications.len())].to_owned(),
        ),
        (
            "version".into(),
            format!(
                "v{}.{}.{}",
                rng.gen_range(1..=5),
                rng.gen_range(0..=10),
                rng.gen_range(0..=20)
            ),
        ),
        (
            "environment".into(),
            environments[rng.gen_range(0..environments.len())].to_owned(),
        ),
        (
            "tier".into(),
            tiers[rng.gen_range(0..tiers.len())].to_owned(),
        ),
    ]);
    labels.append(&mut settings.kubernetes_labels);
    settings.kubernetes_labels = labels;
}

impl crate::LogGenerator for BuiltinGenerator {
    fn generate(&mut self, timestamp: DateTime<Utc>) -> Result<String, crate::GeneratorError> {
        Ok(self.generate_record(timestamp)?.message)
    }

    fn generate_record(
        &mut self,
        timestamp: DateTime<Utc>,
    ) -> Result<crate::GeneratedRecord, crate::GeneratorError> {
        self.args.seed = self.rng.r#gen();
        self.args.record_index = self.index;
        let mut bytes = Vec::new();
        let mut level = None;
        generate_with_level(&mut bytes, &self.args, timestamp, &mut level)?;
        self.index = self.index.checked_add(1).ok_or("record count overflow")?;
        Ok(crate::GeneratedRecord {
            message: String::from_utf8(bytes)?.trim_end_matches('\n').to_owned(),
            level,
        })
    }

    fn pattern(&self) -> &str {
        self.validator.pattern()
    }

    fn validate(&self, record: &str) -> bool {
        self.validator.validate(record).is_valid
    }

    fn metadata(&self) -> BTreeMap<String, String> {
        if matches!(self.args.log_type, LogType::KubernetesPod) {
            return BTreeMap::from([
                (
                    "namespace".into(),
                    self.args
                        .settings
                        .kubernetes_namespace
                        .clone()
                        .expect("pod namespace initialized"),
                ),
                (
                    "pod_name".into(),
                    self.args
                        .settings
                        .kubernetes_pod_name
                        .clone()
                        .expect("pod name initialized"),
                ),
                (
                    "container_name".into(),
                    self.args
                        .settings
                        .kubernetes_container_name
                        .clone()
                        .expect("pod container name initialized"),
                ),
            ]);
        }
        if !matches!(self.args.log_type, LogType::Docker) {
            return BTreeMap::new();
        }
        BTreeMap::from([
            (
                "container_id".to_owned(),
                self.args.settings.docker_container_id.clone(),
            ),
            (
                "container_name".to_owned(),
                self.args
                    .settings
                    .docker_name
                    .as_deref()
                    .unwrap_or("web-app")
                    .to_owned(),
            ),
            (
                "image_name".to_owned(),
                self.args.settings.docker_image_name.clone(),
            ),
        ])
    }

    fn structured_metadata(&self) -> serde_json::Value {
        if matches!(self.args.log_type, LogType::KubernetesPod) {
            let mut metadata =
                serde_json::to_value(self.metadata()).expect("string metadata serializes");
            metadata["labels"] = serde_json::json!(self.args.settings.kubernetes_labels);
            metadata
        } else {
            serde_json::json!(self.metadata())
        }
    }
}

pub(crate) fn create_builtin(
    name: &str,
    value: &serde_yaml::Value,
    seed: u64,
) -> Result<Box<dyn crate::LogGenerator>, crate::GeneratorError> {
    let kind = LogType::from_str(name, false)
        .map_err(|_| format!("unknown built-in generator: {name}"))?;
    if matches!(
        kind,
        LogType::Custom
            | LogType::Scenario
            | LogType::Microservice
            | LogType::ErrorPattern
            | LogType::ErrorLibrary
            | LogType::Process
    ) {
        return Err(format!("generator requires additional configuration: {name}").into());
    }
    let config: TypeConfig = if value.is_null() {
        TypeConfig::default()
    } else {
        serde_yaml::from_value(value.clone())?
    };
    let nginx = config
        .nginx_fields(name)
        .map_err(|error| io::Error::other(error.to_string()))?;
    let mut settings = config
        .settings(name)
        .map_err(|error| io::Error::other(error.to_string()))?;
    if matches!(kind, LogType::Docker) {
        initialize_docker(&mut settings, seed);
    }
    if matches!(kind, LogType::KubernetesPod) {
        initialize_kubernetes_pod(&mut settings, seed);
    }
    let format = if config.log_format.as_deref().map_or_else(
        || config.patterns.len() == 1 && config.patterns[0] == "common",
        |format| format == "common",
    ) {
        Format::Common
    } else {
        Format::Combined
    };
    Ok(Box::new(BuiltinGenerator {
        args: Args {
            total: 1,
            output: PathBuf::new(),
            seed: 0,
            start: None,
            format,
            log_type: kind,
            nginx,
            settings,
            record_index: 0,
            trace: None,
        },
        rng: StdRng::seed_from_u64(seed),
        index: 0,
        validator: crate::validation::LogValidator::new(name)?,
    }))
}
static ACTIVE_SIGNAL: Mutex<ActiveSignal> = Mutex::new(None);
static SIGNAL_HANDLER: OnceLock<Result<(), String>> = OnceLock::new();

struct SignalGuard;
impl SignalGuard {
    fn register(stopped: Arc<AtomicBool>) -> Result<Self, Box<dyn Error>> {
        SIGNAL_HANDLER
            .get_or_init(|| {
                ctrlc::set_handler(|| {
                    if let Ok(active) = ACTIVE_SIGNAL.lock()
                        && let Some((stopped, thread)) = active.as_ref()
                    {
                        stopped.store(true, Ordering::SeqCst);
                        thread.unpark();
                    }
                })
                .map_err(|e| e.to_string())
            })
            .as_ref()
            .map_err(|e| io::Error::other(e.clone()))?;
        let mut active = ACTIVE_SIGNAL
            .lock()
            .map_err(|_| "signal state is poisoned")?;
        if active.is_some() {
            return Err("another generation run is active".into());
        }
        *active = Some((stopped, std::thread::current()));
        Ok(Self)
    }
}
impl Drop for SignalGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE_SIGNAL.lock() {
            *active = None;
        }
    }
}

#[derive(Parser)]
#[command(
    about = "Generate logs",
    after_help = "Examples:\n  log-forge start --output console --types redis --total 3 --no-monitor\n  log-forge start --config config.yaml --seed 42"
)]
struct Start {
    #[arg(short, long, help = "Read a YAML or JSON configuration")]
    config: Option<PathBuf>,
    #[arg(
        long,
        conflicts_with = "standalone_config",
        help = "Merge the original Python default configuration"
    )]
    original_defaults: bool,
    #[arg(long, help = "Use only the supplied configuration")]
    standalone_config: bool,
    #[arg(
        short,
        long,
        help = "Select file, multi_file, console, json, or network"
    )]
    output: Option<String>,
    #[arg(short, long, help = "Set the output file or directory")]
    path: Option<PathBuf>,
    #[arg(short, long, help = "Set the number of records; zero has no limit")]
    total: Option<u64>,
    #[arg(short, long, help = "Set the interval between records in seconds")]
    interval: Option<f64>,
    #[arg(long, value_delimiter = ',', help = "Select comma-separated log types")]
    types: Vec<String>,
    #[arg(long, help = "Set the virtual start time")]
    start_time: Option<String>,
    #[arg(
        long,
        conflicts_with = "fixed_interval",
        help = "Randomize the virtual time interval"
    )]
    random_interval: bool,
    #[arg(long, help = "Use the configured interval without randomization")]
    fixed_interval: bool,
    #[arg(long, help = "Generate at the specified real-time interval")]
    real_time: bool,
    #[arg(long, help = "Advance virtual time without waiting")]
    virtual_time: bool,
    #[arg(long, help = "Disable progress and statistics output")]
    no_monitor: bool,
    #[arg(
        long,
        conflicts_with = "no_monitor",
        help = "Report progress and statistics on stderr"
    )]
    monitor: bool,
    #[arg(long, help = "Hide the progress target")]
    no_progress: bool,
    #[arg(
        long,
        conflicts_with = "no_progress",
        help = "Show the progress target"
    )]
    progress: bool,
    #[arg(long, default_value_t = 0, help = "Set the random seed")]
    seed: u64,
    #[arg(long, help = "Set the network output host")]
    host: Option<String>,
    #[arg(long, help = "Set the network output port")]
    port: Option<u16>,
    #[arg(long, help = "Select tcp, udp, or http")]
    protocol: Option<String>,
    #[arg(long, help = "Set the HTTP output path")]
    endpoint: Option<String>,
    #[arg(long, help = "Set the network timeout in seconds")]
    network_timeout: Option<f64>,
    #[arg(long, help = "Set the rotation threshold in bytes")]
    rotation_size: Option<u64>,
    #[arg(long, help = "Set the rotation interval in seconds")]
    rotation_time: Option<u64>,
    #[arg(long, help = "Set the maximum number of rotated files")]
    max_files: Option<usize>,
    #[arg(long, help = "Set the number of generator threads")]
    threads: Option<usize>,
    #[arg(
        long,
        conflicts_with = "fail_fast",
        help = "Report record errors and continue generation"
    )]
    continue_on_error: bool,
    #[arg(
        long,
        conflicts_with = "continue_on_error",
        help = "Stop generation on the first error"
    )]
    fail_fast: bool,
    #[arg(
        long,
        conflicts_with = "sync_io",
        help = "Write output on a separate thread"
    )]
    async_io: bool,
    #[arg(
        long,
        conflicts_with = "async_io",
        help = "Write output on the generator thread"
    )]
    sync_io: bool,
    #[cfg(any(unix, windows))]
    #[arg(long, help = "Create a local control socket")]
    control_socket: Option<PathBuf>,
}

#[derive(Parser)]
#[command(
    about = "Manage configuration",
    after_help = "Examples:\n  log-forge config create --file config.yaml\n  log-forge config validate --file config.yaml"
)]
struct ConfigCommand {
    #[arg(
        value_parser = ["create", "show", "validate", "edit"],
        help = "Create, show, validate, or edit a configuration in $EDITOR"
    )]
    action: String,
    #[arg(short, long, help = "Set the configuration file")]
    file: Option<PathBuf>,
    #[arg(
        long,
        conflicts_with = "standalone_config",
        help = "Merge the original Python default configuration"
    )]
    original_defaults: bool,
    #[arg(long, help = "Use only the supplied configuration")]
    standalone_config: bool,
    #[arg(
        long,
        value_parser = ["yaml", "json"],
        help = "Select the format for create and show"
    )]
    format: Option<String>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Document {
    log_generator: Config,
    #[serde(default)]
    performance: Option<Performance>,
    #[serde(default)]
    log_rotation: Option<RotationConfig>,
}

pub(crate) struct ConfigurationHandle(Document);

impl ConfigurationHandle {
    pub(crate) fn parse(
        source: &str,
        path: Option<&Path>,
        registry: &GeneratorRegistry,
        original_defaults: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let document = parse_document(source, path, original_defaults)?;
        validate_config(&document.log_generator, registry)?;
        Ok(Self(document))
    }

    pub(crate) fn global(&self) -> Result<serde_yaml::Value, serde_yaml::Error> {
        serde_yaml::to_value(&self.0.log_generator.global)
    }

    pub(crate) fn log_type(
        &self,
        name: &str,
    ) -> Result<Option<serde_yaml::Value>, serde_yaml::Error> {
        self.0
            .log_generator
            .log_types
            .get(name)
            .map(serde_yaml::to_value)
            .transpose()
    }

    pub(crate) fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(&self.0)
    }

    pub(crate) fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.0)
    }
}
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default, rename = "global")]
    global: Global,
    #[serde(default)]
    log_types: BTreeMap<String, TypeConfig>,
    #[serde(default)]
    error_patterns: BTreeMap<String, Vec<LibraryPattern>>,
    #[serde(default)]
    performance: Option<Performance>,
    #[serde(default)]
    logging: Option<crate::internal_log::Config>,
    #[serde(default)]
    distributed_tracing: Option<crate::tracing::TraceConfig>,
    #[serde(default)]
    monitoring: Option<crate::monitor::MonitorConfig>,
    #[serde(default)]
    output: Option<OutputConfig>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LibraryPattern {
    pattern: String,
    frequency: f64,
    severity: String,
}

fn library_patterns(
    library: &BTreeMap<String, Vec<LibraryPattern>>,
) -> Result<Vec<(String, LibraryPattern)>, Box<dyn Error>> {
    let mut patterns = Vec::new();
    for (category, entries) in library {
        if category.is_empty() || category.chars().any(char::is_whitespace) || entries.is_empty() {
            return Err("invalid error pattern category".into());
        }
        for entry in entries {
            if !["debug", "info", "warning", "error", "critical"]
                .contains(&entry.severity.to_ascii_lowercase().as_str())
            {
                return Err("invalid error pattern severity".into());
            }
            crate::microservice::validate_event(&entry.pattern)?;
            patterns.push((category.clone(), entry.clone()));
        }
    }
    if !patterns.is_empty() {
        crate::weights::checked(patterns.iter().map(|(_, entry)| entry.frequency))?;
    }
    Ok(patterns)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OutputConfig {
    handlers: Vec<OutputHandlerConfig>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OutputHandlerConfig {
    #[serde(rename = "type")]
    kind: String,
    file_path: Option<PathBuf>,
    protocol: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    format: Option<String>,
    endpoint: Option<String>,
    colored: Option<bool>,
    max_retries: Option<usize>,
    retry_delay: Option<f64>,
    include_raw_message: Option<bool>,
}

fn handler_json_format(format: Option<&str>) -> Option<crate::json_output::JsonFormat> {
    match format {
        Some("json") => Some(crate::json_output::JsonFormat::Standard),
        Some("elk_json") => Some(crate::json_output::JsonFormat::Elk),
        Some("splunk_json") => Some(crate::json_output::JsonFormat::Splunk),
        _ => None,
    }
}

fn validate_handlers(config: &OutputConfig) -> Result<(), Box<dyn Error>> {
    if config.handlers.is_empty() {
        return Err("output handlers must not be empty".into());
    }
    for handler in &config.handlers {
        if ![
            "raw",
            "json",
            "elk_json",
            "splunk_json",
            "prometheus",
            "human_readable",
            "syslog",
        ]
        .contains(&handler.format.as_deref().unwrap_or("raw"))
        {
            return Err(format!(
                "unsupported handler format: {}",
                handler.format.as_deref().unwrap_or("")
            )
            .into());
        }
        if (handler.max_retries.is_some() || handler.retry_delay.is_some())
            && !(handler.kind == "network" && handler.protocol.as_deref().unwrap_or("tcp") == "tcp")
        {
            return Err("TCP retry options require a TCP handler".into());
        }
        if let Some(delay) = handler.retry_delay {
            parse_retry_delay(delay)?;
        }
        if handler.colored.is_some() && handler.kind != "console" {
            return Err("colored is valid only for console handlers".into());
        }
        if handler.colored == Some(true) && handler_json_format(handler.format.as_deref()).is_some()
        {
            return Err("colored console cannot use json format".into());
        }
        if handler.include_raw_message.is_some()
            && handler_json_format(handler.format.as_deref()).is_none()
        {
            return Err("include_raw_message requires a JSON handler".into());
        }
        if handler.format.as_deref() == Some("human_readable") && handler.kind != "console" {
            return Err("human_readable format requires a console handler".into());
        }
        if handler.format.as_deref() == Some("syslog")
            && !(handler.kind == "network"
                && ["tcp", "udp"].contains(&handler.protocol.as_deref().unwrap_or("tcp")))
        {
            return Err("syslog format requires a TCP or UDP network handler".into());
        }
        if handler.format.as_deref() == Some("prometheus")
            && !(handler.kind == "network" && handler.protocol.as_deref() == Some("http"))
        {
            return Err("prometheus format requires an HTTP network handler".into());
        }
        match handler.kind.as_str() {
            "file"
                if handler.file_path.is_some()
                    && handler.host.is_none()
                    && handler.port.is_none()
                    && handler.protocol.is_none()
                    && handler.endpoint.is_none() => {}
            "console"
                if handler.file_path.is_none()
                    && handler.host.is_none()
                    && handler.port.is_none()
                    && handler.protocol.is_none()
                    && handler.endpoint.is_none() => {}
            "network"
                if handler.file_path.is_none()
                    && handler.host.as_deref().is_some_and(|host| !host.is_empty())
                    && handler.port.is_some_and(|port| port > 0)
                    && ((["tcp", "udp"]
                        .contains(&handler.protocol.as_deref().unwrap_or("tcp"))
                        && handler.endpoint.is_none())
                        || (handler.protocol.as_deref() == Some("http")
                            && handler.endpoint.is_some())) =>
            {
                if handler.protocol.as_deref() == Some("http") {
                    crate::http_output::validate_target(
                        handler.host.as_deref().ok_or("handler host required")?,
                        handler
                            .endpoint
                            .as_deref()
                            .ok_or("HTTP endpoint required")?,
                    )?;
                }
            }
            _ => {
                return Err(
                    format!("invalid or unsupported output handler: {}", handler.kind).into(),
                );
            }
        }
    }
    Ok(())
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Performance {
    threads: Option<usize>,
    batch_size: Option<usize>,
    buffer_size: Option<usize>,
    async_io: Option<bool>,
    memory_limit: Option<SizeValue>,
    thread_pool_size: Option<usize>,
    io_buffer_size: Option<usize>,
    gc_threshold: Option<f64>,
    lazy_loading: Option<bool>,
}
#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Global {
    output_format: String,
    output_path: PathBuf,
    total_logs: u64,
    generation_interval: f64,
    random_interval: bool,
    virtual_time: bool,
    start_time: Option<String>,
    multi_file: MultiFile,
    network: Option<NetworkConfig>,
    network_host: String,
    network_port: u16,
    network_protocol: String,
    network_timeout: f64,
    network_max_retries: usize,
    network_retry_delay: f64,
    network_endpoint: String,
    rotation_size: Option<u64>,
    rotation_time: Option<u64>,
    max_files: usize,
    buffer_size: Option<usize>,
    batch_size: Option<usize>,
    threads: Option<usize>,
    thread_pool_size: Option<usize>,
    lazy_loading: Option<bool>,
    async_io: Option<bool>,
    log_rotation: Option<RotationConfig>,
    memory_limit: Option<SizeValue>,
    field_mapping: BTreeMap<String, String>,
    include_raw_message: bool,
    pretty_print: bool,
}
impl Default for Global {
    fn default() -> Self {
        Self {
            output_format: "multi_file".into(),
            output_path: "./logs".into(),
            total_logs: 10_000,
            generation_interval: 0.1,
            random_interval: false,
            virtual_time: true,
            start_time: None,
            multi_file: MultiFile::default(),
            network: None,
            network_host: "localhost".into(),
            network_port: 514,
            network_protocol: "tcp".into(),
            network_timeout: 5.0,
            network_max_retries: 3,
            network_retry_delay: 1.0,
            network_endpoint: "/".into(),
            rotation_size: None,
            rotation_time: None,
            max_files: 10,
            buffer_size: None,
            batch_size: None,
            threads: None,
            thread_pool_size: None,
            lazy_loading: None,
            async_io: None,
            log_rotation: None,
            memory_limit: None,
            field_mapping: BTreeMap::new(),
            include_raw_message: true,
            pretty_print: false,
        }
    }
}
#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct NetworkConfig {
    host: String,
    port: u16,
    protocol: String,
    timeout: Option<f64>,
    max_retries: Option<usize>,
    retry_delay: Option<f64>,
    endpoint: Option<String>,
}
impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 514,
            protocol: "udp".into(),
            timeout: None,
            max_retries: None,
            retry_delay: None,
            endpoint: None,
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct RotationConfig {
    enabled: bool,
    max_size: Option<SizeValue>,
    max_files: Option<usize>,
    backup_count: Option<usize>,
    compress: bool,
    rotation_time: Option<TimeValue>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
enum SizeValue {
    Bytes(u64),
    Text(String),
}
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum TimeValue {
    Seconds(u64),
    Text(String),
}

fn parse_size(value: &SizeValue) -> Result<u64, Box<dyn Error>> {
    match value {
        SizeValue::Bytes(bytes) => Ok(*bytes),
        SizeValue::Text(text) => {
            let text = text.trim();
            let split = text
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(text.len());
            let number: u64 = text[..split].parse()?;
            let factor = match text[split..].trim().to_ascii_uppercase().as_str() {
                "" | "B" => 1,
                "KB" | "KIB" => 1024,
                "MB" | "MIB" => 1024 * 1024,
                "GB" | "GIB" => 1024 * 1024 * 1024,
                _ => return Err("invalid rotation size unit".into()),
            };
            number
                .checked_mul(factor)
                .ok_or_else(|| "rotation size is too large".into())
        }
    }
}
fn parse_rotation_time(value: &TimeValue) -> Result<u64, Box<dyn Error>> {
    match value {
        TimeValue::Seconds(seconds) => Ok(*seconds),
        TimeValue::Text(text) => match text.as_str() {
            "hourly" => Ok(3600),
            "daily" => Ok(86400),
            "weekly" => Ok(604800),
            _ => Err("invalid rotation time".into()),
        },
    }
}
#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct MultiFile {
    file_pattern: String,
    date_format: String,
    separate_by_type: bool,
}
impl Default for MultiFile {
    fn default() -> Self {
        Self {
            file_pattern: "{log_type}_{date}.log".into(),
            date_format: "%Y%m%d".into(),
            separate_by_type: true,
        }
    }
}
#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct TypeConfig {
    enabled: bool,
    frequency: f64,
    service_name: Option<String>,
    operations: Vec<String>,
    databases: Vec<ServiceDatabase>,
    containers: Vec<String>,
    patterns: Vec<String>,
    custom_fields: RawFields,
    error_levels: Option<Choices>,
    error_patterns: Vec<String>,
    log_format: Option<String>,
    virtual_hosts: Vec<String>,
    facilities: Option<Choices>,
    message_patterns: BTreeMap<String, Vec<String>>,
    severities: Option<Choices>,
    rfc_format: Option<String>,
    priority_style: Option<String>,
    hostname: Option<String>,
    broker_ids: Option<Vec<u32>>,
    topics: Option<Vec<String>>,
    partition_count: Option<u32>,
    log_levels: Option<Choices>,
    components: Vec<String>,
    include_sql_queries: Option<bool>,
    sql_query_patterns: Vec<String>,
    endpoints: Vec<String>,
    response_time_distribution: BTreeMap<String, ResponseRange>,
    include_request_id: Option<bool>,
    include_user_id: Option<bool>,
    include_correlation_id: Option<bool>,
    format_type: Option<String>,
    container_names: Vec<String>,
    container_name: Option<String>,
    container_id: Option<String>,
    images: Option<Vec<String>>,
    image_name: Option<String>,
    log_drivers: Vec<String>,
    container_patterns: BTreeMap<String, Vec<String>>,
    namespaces: Vec<String>,
    namespace: Option<String>,
    pod_name: Option<String>,
    labels: Option<BTreeMap<String, String>>,
    pod_patterns: Vec<String>,
    event_patterns: Vec<String>,
    event_types: Option<Choices>,
    query_types: Option<Choices>,
    table_names: Vec<String>,
    models: Vec<String>,
    height_range: Option<[u64; 2]>,
    module_weights: BTreeMap<String, f64>,
    modules: Option<Choices>,
    blockchain: Option<Blockchain>,
    network: Option<GoloopNetwork>,
    template: Option<String>,
    pattern_file: Option<PathBuf>,
    pattern_name: Option<String>,
    frequency_maps: crate::template_frequency::Frequencies,
    correlation_rules: Vec<crate::template_frequency::CorrelationRule>,
    field_frequency: Option<crate::frequency::FieldFrequencyConfig>,
    field_inactive_value: Option<String>,
    fields: BTreeMap<String, serde_yaml::Value>,
    plugin_config: Option<serde_yaml::Value>,
    plugin_command: Option<PathBuf>,
    plugin_args: Vec<String>,
    plugin_timeout: Option<f64>,
    scenario_file: Option<PathBuf>,
    patterns_dir: Option<PathBuf>,
    error_category: Option<String>,
    error_severity: Option<String>,
    error_frequency: Option<crate::frequency::ErrorFrequencyConfig>,
}
impl Default for TypeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            frequency: 1.0,
            service_name: None,
            operations: Vec::new(),
            databases: Vec::new(),
            containers: Vec::new(),
            patterns: Vec::new(),
            custom_fields: RawFields::default(),
            error_levels: None,
            error_patterns: Vec::new(),
            log_format: None,
            virtual_hosts: Vec::new(),
            facilities: None,
            message_patterns: BTreeMap::new(),
            severities: None,
            rfc_format: None,
            priority_style: None,
            hostname: None,
            broker_ids: None,
            topics: None,
            partition_count: None,
            log_levels: None,
            components: Vec::new(),
            include_sql_queries: None,
            sql_query_patterns: Vec::new(),
            endpoints: Vec::new(),
            response_time_distribution: BTreeMap::new(),
            include_request_id: None,
            include_user_id: None,
            include_correlation_id: None,
            format_type: None,
            container_names: Vec::new(),
            container_name: None,
            container_id: None,
            images: None,
            image_name: None,
            log_drivers: Vec::new(),
            container_patterns: BTreeMap::new(),
            namespaces: Vec::new(),
            namespace: None,
            pod_name: None,
            labels: None,
            pod_patterns: Vec::new(),
            event_patterns: Vec::new(),
            event_types: None,
            query_types: None,
            table_names: Vec::new(),
            models: Vec::new(),
            height_range: None,
            module_weights: BTreeMap::new(),
            modules: None,
            blockchain: None,
            network: None,
            template: None,
            pattern_file: None,
            pattern_name: None,
            frequency_maps: BTreeMap::new(),
            correlation_rules: Vec::new(),
            field_frequency: None,
            field_inactive_value: None,
            fields: BTreeMap::new(),
            plugin_config: None,
            plugin_command: None,
            plugin_args: Vec::new(),
            plugin_timeout: None,
            scenario_file: None,
            patterns_dir: None,
            error_category: None,
            error_severity: None,
            error_frequency: None,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
enum Choices {
    List(Vec<String>),
    Weights(BTreeMap<String, f64>),
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Blockchain {
    height_range: Option<[u64; 2]>,
    network_id: Option<String>,
    revision_range: Option<[u32; 2]>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct GoloopNetwork {
    pub use_real_ips: bool,
    pub p2p_port: u16,
    pub local_ip: String,
}
impl Default for GoloopNetwork {
    fn default() -> Self {
        Self {
            use_real_ips: true,
            p2p_port: 7100,
            local_ip: "172.17.0.2".into(),
        }
    }
}

fn weighted_choices(value: Option<&Choices>) -> Result<Vec<(String, f64)>, Box<dyn Error>> {
    let choices: Vec<_> = match value {
        Some(Choices::List(values)) => values.iter().map(|s| (s.clone(), 1.0)).collect(),
        Some(Choices::Weights(values)) => values.iter().map(|(s, w)| (s.clone(), *w)).collect(),
        None => Vec::new(),
    };
    if value.is_some() && choices.is_empty() {
        return Err("weighted choices must not be empty".into());
    }
    if !choices.is_empty() {
        crate::weights::checked(choices.iter().map(|(_, weight)| *weight))?;
    }
    Ok(choices)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ServiceDatabase {
    name: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct RawFields {
    error_levels: Option<Choices>,
    error_patterns: Vec<String>,
    facilities: Option<Choices>,
    severities: Option<Choices>,
    hostname: Option<String>,
    priority_style: Option<String>,
    ip_ranges: Vec<String>,
    status_codes: BTreeMap<u16, f64>,
    user_agents: Vec<String>,
    url_patterns: Vec<String>,
    namespaces: Vec<String>,
    namespace: Option<String>,
    pod_name: Option<String>,
    labels: Option<BTreeMap<String, String>>,
    log_levels: Option<Choices>,
    modules: Option<Choices>,
    container_names: Vec<String>,
    container_name: Option<String>,
    container_id: Option<String>,
    images: Option<Vec<String>>,
    image_name: Option<String>,
    event_types: Option<Choices>,
    query_types: Option<Choices>,
    table_names: Vec<String>,
    endpoints: Vec<String>,
    models: Vec<String>,
    events: Vec<crate::microservice::ServiceEvent>,
    db_operations: Vec<String>,
    external_calls: Vec<String>,
    gateway_calls: Vec<String>,
    upstream_services: Vec<String>,
    response_times: BTreeMap<String, ResponseRange>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ResponseRange {
    min: u64,
    max: u64,
    weight: f64,
}

#[derive(Clone, Default)]
pub(crate) struct NginxFields {
    pub ip_ranges: Vec<(Ipv4Addr, u8)>,
    pub status_codes: Vec<(u16, f64)>,
    pub user_agents: Vec<String>,
    pub url_patterns: Vec<String>,
}

#[derive(Clone, Default)]
pub(crate) struct GeneratorFields {
    pub error_levels: Vec<(String, f64)>,
    pub error_patterns: Vec<String>,
    pub virtual_hosts: Vec<String>,
    pub facilities: Vec<(String, f64)>,
    pub syslog_message_patterns: BTreeMap<String, Vec<String>>,
    pub severities: Vec<(String, f64)>,
    pub syslog_rfc: Option<String>,
    pub syslog_priority_style: Option<String>,
    pub syslog_hostname: Option<String>,
    pub kafka_broker_ids: Vec<u32>,
    pub kafka_topics: Vec<String>,
    pub kafka_partition_count: Option<u32>,
    pub kafka_levels: Vec<(String, f64)>,
    pub redis_levels: Vec<(String, f64)>,
    pub log_levels: Vec<(String, f64)>,
    pub endpoints: Vec<String>,
    pub fastapi_status_codes: Vec<(u16, f64)>,
    fastapi_response_times: Vec<ResponseRange>,
    pub fastapi_no_request_id: bool,
    pub fastapi_user_id: bool,
    pub fastapi_correlation_id: bool,
    pub template: Option<String>,
    pub fields: BTreeMap<String, serde_yaml::Value>,
    pub file_fields: BTreeMap<String, Vec<String>>,
    pub frequency_maps: crate::template_frequency::Frequencies,
    pub correlation_rules: Vec<crate::template_frequency::CorrelationRule>,
    pub goloop_levels: Vec<(String, f64)>,
    pub goloop_modules: Vec<(String, f64)>,
    pub goloop_format: Option<String>,
    pub goloop_height_range: Option<[u64; 2]>,
    pub goloop_network: GoloopNetwork,
    pub goloop_network_id: Option<String>,
    pub goloop_revision_range: Option<[u32; 2]>,
    pub docker_format: Option<String>,
    pub docker_levels: Vec<(String, f64)>,
    pub docker_names: Vec<String>,
    pub docker_name: Option<String>,
    pub docker_drivers: Vec<String>,
    pub docker_driver: Option<String>,
    pub docker_container_id: String,
    pub docker_images: Vec<String>,
    pub docker_image_name: String,
    pub docker_patterns: BTreeMap<String, Vec<String>>,
    pub kubernetes_levels: Vec<(String, f64)>,
    pub kubernetes_namespaces: Vec<String>,
    pub kubernetes_namespace: Option<String>,
    pub kubernetes_pod_name: Option<String>,
    pub kubernetes_container_name: Option<String>,
    pub kubernetes_labels: BTreeMap<String, String>,
    pub kubernetes_pod_patterns: Vec<String>,
    pub kubernetes_event_patterns: Vec<String>,
    pub kubernetes_event_types: Vec<(String, f64)>,
    pub database_levels: Vec<(String, f64)>,
    pub database_queries: Vec<(String, f64)>,
    pub database_tables: Vec<String>,
    pub django_levels: Vec<(String, f64)>,
    pub django_models: Vec<String>,
    pub django_components: Vec<String>,
    pub django_sql_patterns: Vec<String>,
    pub service_name: Option<String>,
    pub service_operations: Vec<String>,
    pub service_events: Vec<crate::microservice::ServiceEvent>,
    pub service_databases: Vec<String>,
    pub service_containers: Vec<String>,
    pub service_levels: Vec<(String, f64)>,
    pub gateway_upstreams: Vec<String>,
    pub gateway_statuses: Vec<(u16, f64)>,
    gateway_response_times: Vec<ResponseRange>,
    error_library: Vec<(String, LibraryPattern)>,
}

pub(crate) fn library_message(
    settings: &GeneratorFields,
    rng: &mut StdRng,
) -> Result<(String, String, String), String> {
    let weights = crate::weights::checked(
        settings
            .error_library
            .iter()
            .map(|(_, entry)| entry.frequency),
    )
    .map_err(|error| error.to_string())?;
    let (category, entry) = &settings.error_library[weights.sample(rng)];
    let message = crate::microservice::render_event(&entry.pattern, category, &[], &[], rng)?;
    Ok((
        category.clone(),
        entry.severity.to_ascii_uppercase(),
        message,
    ))
}

pub(crate) fn fastapi_response_time(
    settings: &GeneratorFields,
    status: u16,
    rng: &mut StdRng,
) -> Result<u64, String> {
    if !settings.fastapi_response_times.is_empty() {
        let weights = crate::weights::checked(
            settings
                .fastapi_response_times
                .iter()
                .map(|range| range.weight),
        )
        .map_err(|error| error.to_string())?;
        let range = &settings.fastapi_response_times[weights.sample(rng)];
        return Ok(rng.gen_range(range.min..=range.max));
    }
    Ok(match status {
        500.. => rng.gen_range(2000..=30000),
        404 => rng.gen_range(10..=100),
        400 | 401 | 403 | 422 => rng.gen_range(20..=200),
        _ => rng.gen_range(50..=1500),
    })
}

pub(crate) fn gateway_message(
    settings: &GeneratorFields,
    rng: &mut StdRng,
) -> Result<String, String> {
    let statuses =
        crate::weights::checked(settings.gateway_statuses.iter().map(|(_, weight)| *weight))
            .map_err(|error| error.to_string())?;
    let times = crate::weights::checked(
        settings
            .gateway_response_times
            .iter()
            .map(|range| range.weight),
    )
    .map_err(|error| error.to_string())?;
    let status = settings.gateway_statuses[statuses.sample(rng)].0;
    let range = &settings.gateway_response_times[times.sample(rng)];
    let duration = rng.gen_range(range.min..=range.max);
    let endpoint =
        choose(&settings.endpoints, "", rng).replace('*', &rng.gen_range(1..=9999).to_string());
    let upstream = choose(&settings.gateway_upstreams, "", rng);
    Ok(format!(
        "method=GET endpoint={endpoint} upstream={upstream} status={status} response_time_ms={duration}"
    ))
}

pub(crate) fn choose_weighted<'a>(
    values: &'a [(String, f64)],
    defaults: &'a [(&'a str, f64)],
    rng: &mut StdRng,
) -> Result<&'a str, String> {
    if values.is_empty() {
        let index = crate::weights::checked(defaults.iter().map(|(_, weight)| *weight))
            .map_err(|e| e.to_string())?;
        Ok(defaults[index.sample(rng)].0)
    } else {
        let index = crate::weights::checked(values.iter().map(|(_, weight)| *weight))
            .map_err(|e| e.to_string())?;
        Ok(&values[index.sample(rng)].0)
    }
}

pub(crate) fn choose<'a>(values: &'a [String], fallback: &'a str, rng: &mut StdRng) -> &'a str {
    if values.is_empty() {
        fallback
    } else {
        &values[rng.gen_range(0..values.len())]
    }
}

pub(crate) fn syslog_priority(facility: &str, severity: &str) -> Result<u8, &'static str> {
    let facility = match facility {
        "kern" => 0,
        "user" => 1,
        "mail" => 2,
        "daemon" => 3,
        "auth" => 4,
        "syslog" => 5,
        "lpr" => 6,
        "news" => 7,
        "uucp" => 8,
        "cron" => 9,
        "authpriv" => 10,
        "ftp" => 11,
        "local0" => 16,
        "local1" => 17,
        "local2" => 18,
        "local3" => 19,
        "local4" => 20,
        "local5" => 21,
        "local6" => 22,
        "local7" => 23,
        _ => return Err("unknown syslog facility"),
    };
    let severity = match severity {
        "emerg" => 0,
        "alert" => 1,
        "crit" => 2,
        "err" => 3,
        "warning" => 4,
        "notice" => 5,
        "info" => 6,
        "debug" => 7,
        _ => return Err("unknown syslog severity"),
    };
    Ok(facility * 8 + severity)
}

pub(crate) fn render_custom(
    settings: &GeneratorFields,
    time: DateTime<Utc>,
    rng: &mut StdRng,
    record_index: u64,
) -> Result<String, String> {
    render_custom_with_controller(settings, time, rng, record_index, None)
}

fn render_custom_with_controller(
    settings: &GeneratorFields,
    time: DateTime<Utc>,
    rng: &mut StdRng,
    record_index: u64,
    mut controller: Option<(&mut crate::frequency::FieldFrequencyController, &str)>,
) -> Result<String, String> {
    let template = settings
        .template
        .as_ref()
        .ok_or("custom log type requires template")?;
    let mut output = String::new();
    let mut generated: HashMap<String, String> = HashMap::new();
    let mut context: HashMap<String, String> = HashMap::new();
    for part in crate::template_syntax::parse(template) {
        let spec = match part? {
            crate::template_syntax::Part::Text(text) => {
                output.push_str(text);
                continue;
            }
            crate::template_syntax::Part::Field(spec) => spec,
        };
        let (name, options) = spec.split_once(':').unwrap_or((spec, ""));
        let value = if let Some(existing) = generated.get(spec) {
            existing.clone()
        } else if let Some(controlled) = if let Some((frequency, inactive)) = controller.as_mut() {
            if !frequency
                .should_generate_field(name, time)
                .map_err(|error| error.to_string())?
            {
                Some((*inactive).to_owned())
            } else {
                frequency
                    .value_override(name, &context, time)
                    .map_err(|error| error.to_string())?
            }
        } else {
            crate::template_frequency::value(
                name,
                &context,
                &settings.frequency_maps,
                &settings.correlation_rules,
                rng,
            )?
        } {
            controlled
        } else if let Some(field) = settings.fields.get(name) {
            match field {
                serde_yaml::Value::Sequence(choices) if !choices.is_empty() => choices
                    [rng.gen_range(0..choices.len())]
                .as_str()
                .ok_or("custom field choices must be strings")?
                .to_owned(),
                serde_yaml::Value::String(kind) if kind == "datetime" || kind == "timestamp" => {
                    time.format("%Y-%m-%d %H:%M:%S").to_string()
                }
                serde_yaml::Value::String(kind) if kind == "sentence" => {
                    "Request completed".to_string()
                }
                serde_yaml::Value::String(kind) => {
                    let (kind, field_options) = kind.split_once(':').unwrap_or((kind, ""));
                    if !options.is_empty() && !field_options.is_empty() {
                        return Err(format!("field {name} has conflicting options"));
                    }
                    generate_builtin(
                        kind,
                        if field_options.is_empty() {
                            options
                        } else {
                            field_options
                        },
                        time,
                        rng,
                        record_index,
                    )?
                }
                _ => return Err(format!("unsupported custom field {name}")),
            }
        } else if options.starts_with("file:") {
            let choices = settings
                .file_fields
                .get(spec)
                .ok_or_else(|| format!("file field {name} is not loaded"))?;
            choices[rng.gen_range(0..choices.len())].clone()
        } else {
            generate_builtin(name, options, time, rng, record_index)?
        };
        generated.insert(spec.to_string(), value.clone());
        context.insert(name.to_owned(), value.clone());
        output.push_str(&value);
    }
    Ok(output)
}

pub(crate) fn load_file_fields(
    template: &str,
    fields: &BTreeMap<String, serde_yaml::Value>,
) -> Result<BTreeMap<String, Vec<String>>, Box<dyn Error>> {
    let mut loaded = BTreeMap::new();
    for part in crate::template_syntax::parse(template) {
        let spec = match part.map_err(io::Error::other)? {
            crate::template_syntax::Part::Text(_) => continue,
            crate::template_syntax::Part::Field(spec) => spec,
        };
        if let Some((name, path)) = spec.split_once(":file:")
            && !fields.contains_key(name)
            && !loaded.contains_key(spec)
        {
            if path.is_empty() {
                return Err("file field path is empty".into());
            }
            let text = fs::read_to_string(path)?;
            let choices: Vec<String> = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect();
            if choices.is_empty() {
                return Err(format!("file field has no values: {path}").into());
            }
            loaded.insert(spec.to_owned(), choices);
        }
    }
    Ok(loaded)
}

fn pick_builtin(values: &[&str], rng: &mut StdRng) -> String {
    values[rng.gen_range(0..values.len())].to_owned()
}

struct ControlledCustom {
    settings: GeneratorFields,
    frequency: crate::frequency::FieldFrequencyController,
    inactive_value: String,
    rng: StdRng,
    index: u64,
}
impl crate::LogGenerator for ControlledCustom {
    fn generate(&mut self, timestamp: DateTime<Utc>) -> Result<String, crate::GeneratorError> {
        let value = render_custom_with_controller(
            &self.settings,
            timestamp,
            &mut self.rng,
            self.index,
            Some((&mut self.frequency, &self.inactive_value)),
        )
        .map_err(io::Error::other)?;
        self.index = self
            .index
            .checked_add(1)
            .ok_or("custom record counter overflow")?;
        Ok(value)
    }
    fn pattern(&self) -> &str {
        "custom"
    }
    fn validate(&self, record: &str) -> bool {
        !record.is_empty()
    }
}

fn generate_builtin(
    name: &str,
    options: &str,
    time: DateTime<Utc>,
    rng: &mut StdRng,
    record_index: u64,
) -> Result<String, String> {
    if !options.is_empty() && !options.contains(':') && options.contains(',') {
        let choices: Vec<_> = options.split(',').map(str::trim).collect();
        if choices.iter().any(|choice| choice.is_empty()) {
            return Err("empty template choice".into());
        }
        return Ok(choices[rng.gen_range(0..choices.len())].to_owned());
    }
    let mut parameters = BTreeMap::new();
    if options.contains(':') {
        for option in options.split(',') {
            let (key, value) = option
                .split_once(':')
                .ok_or_else(|| format!("invalid template option: {option}"))?;
            if parameters.insert(key.trim(), value.trim()).is_some() {
                return Err("duplicate template option".into());
            }
        }
    }
    let value = match name {
        "timestamp" | "datetime" => time.format("%Y-%m-%d %H:%M:%S").to_string(),
        "iso_timestamp" => time.to_rfc3339(),
        "epoch_timestamp" | "unix_timestamp" => time.timestamp().to_string(),
        "counter" => {
            let start: i64 = parameters
                .remove("start")
                .unwrap_or("1")
                .parse()
                .map_err(|_| "invalid counter start")?;
            let step: i64 = parameters
                .remove("step")
                .unwrap_or("1")
                .parse()
                .map_err(|_| "invalid counter step")?;
            let index = i64::try_from(record_index).map_err(|_| "counter overflow")?;
            start
                .checked_add(step.checked_mul(index).ok_or("counter overflow")?)
                .ok_or("counter overflow")?
                .to_string()
        }
        "random_int" => {
            let min: i64 = parameters
                .remove("min_val")
                .unwrap_or("1")
                .parse()
                .map_err(|_| "invalid random_int minimum")?;
            let max: i64 = parameters
                .remove("max_val")
                .unwrap_or("100")
                .parse()
                .map_err(|_| "invalid random_int maximum")?;
            if min > max {
                return Err("random_int minimum exceeds maximum".into());
            }
            rng.gen_range(min..=max).to_string()
        }
        "random_float" => {
            let min: f64 = parameters
                .remove("min_val")
                .unwrap_or("0")
                .parse()
                .map_err(|_| "invalid random_float minimum")?;
            let max: f64 = parameters
                .remove("max_val")
                .unwrap_or("1")
                .parse()
                .map_err(|_| "invalid random_float maximum")?;
            if !min.is_finite() || !max.is_finite() || min > max || !(max - min).is_finite() {
                return Err("invalid random_float range".into());
            }
            format!(
                "{:.3}",
                if min == max {
                    min
                } else {
                    rng.gen_range(min..max)
                }
            )
        }
        "uuid" | "session_id" => {
            let mut bytes: [u8; 16] = rng.r#gen();
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                bytes[0],
                bytes[1],
                bytes[2],
                bytes[3],
                bytes[4],
                bytes[5],
                bytes[6],
                bytes[7],
                bytes[8],
                bytes[9],
                bytes[10],
                bytes[11],
                bytes[12],
                bytes[13],
                bytes[14],
                bytes[15]
            )
        }
        "ip_address" => {
            let pick = rng.gen_range(0..10);
            let first = if pick < 4 {
                (192, 168)
            } else if pick < 7 {
                (10, rng.gen_range(0..=255))
            } else if pick == 7 {
                (172, rng.gen_range(16..=31))
            } else {
                (rng.gen_range(1..=99), rng.gen_range(0..=255))
            };
            format!(
                "{}.{}.{}.{}",
                first.0,
                first.1,
                rng.gen_range(0..=255),
                rng.gen_range(1..=254)
            )
        }
        "ipv4" => format!(
            "{}.{}.{}.{}",
            rng.gen_range(1..=223),
            rng.gen_range(0..=255),
            rng.gen_range(0..=255),
            rng.gen_range(1..=254)
        ),
        "ipv6" => std::net::Ipv6Addr::from(rng.r#gen::<u128>()).to_string(),
        "mac_address" => format!(
            "02:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            rng.r#gen::<u8>(),
            rng.r#gen::<u8>(),
            rng.r#gen::<u8>(),
            rng.r#gen::<u8>(),
            rng.r#gen::<u8>()
        ),
        "domain_name" => format!("service{}.example.test", rng.gen_range(1..=1000)),
        "url" | "referer" => format!("https://service{}.example.test/", rng.gen_range(1..=1000)),
        "uri" => format!(
            "https://service{}.example.test/api/items/{}",
            rng.gen_range(1..=1000),
            rng.gen_range(1..=10000)
        ),
        "port" => if rng.gen_bool(0.6) {
            let ports = [
                80, 443, 22, 21, 25, 53, 110, 143, 993, 995, 8080, 8443, 3000, 5000,
            ];
            ports[rng.gen_range(0..ports.len())]
        } else {
            rng.gen_range(1024..=65535)
        }
        .to_string(),
        "user_agent" => pick_builtin(
            &[
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0",
                "Mozilla/5.0 (X11; Linux x86_64) Firefox/120.0",
                "curl/8.0.0",
            ],
            rng,
        ),
        "browser" => pick_builtin(
            &[
                "Chrome",
                "Firefox",
                "Safari",
                "Edge",
                "Opera",
                "Internet Explorer",
            ],
            rng,
        ),
        "platform" => pick_builtin(
            &[
                "Windows", "macOS", "Linux", "iOS", "Android", "Ubuntu", "CentOS",
            ],
            rng,
        ),
        "http_method" => choose_weighted(
            &[],
            &[
                ("GET", 0.6),
                ("POST", 0.25),
                ("PUT", 0.05),
                ("DELETE", 0.03),
                ("PATCH", 0.02),
                ("HEAD", 0.03),
                ("OPTIONS", 0.02),
            ],
            rng,
        )?
        .to_owned(),
        "http_status_code" => choose_weighted(
            &[],
            &[
                ("200", 0.70),
                ("404", 0.10),
                ("500", 0.05),
                ("301", 0.03),
                ("302", 0.03),
                ("400", 0.02),
                ("401", 0.02),
                ("403", 0.02),
                ("503", 0.01),
                ("502", 0.01),
                ("201", 0.01),
            ],
            rng,
        )?
        .to_owned(),
        "http_status_phrase" => pick_builtin(
            &[
                "OK",
                "Created",
                "Moved Permanently",
                "Found",
                "Bad Request",
                "Unauthorized",
                "Forbidden",
                "Not Found",
                "Internal Server Error",
                "Bad Gateway",
                "Service Unavailable",
            ],
            rng,
        ),
        "content_type" => pick_builtin(
            &[
                "text/html",
                "application/json",
                "text/css",
                "application/javascript",
                "image/png",
                "image/jpeg",
                "text/plain",
                "application/xml",
                "application/pdf",
            ],
            rng,
        ),
        "username" => format!("user{}", rng.gen_range(1..=99999)),
        "email" => format!("user{}@example.test", rng.gen_range(1..=99999)),
        "first_name" => pick_builtin(&["Alex", "Kim", "Morgan", "Sam", "Taylor"], rng),
        "last_name" => pick_builtin(&["Lee", "Park", "Kim", "Smith", "Taylor"], rng),
        "full_name" => format!(
            "{} {}",
            pick_builtin(&["Alex", "Morgan", "Sam", "Taylor"], rng),
            pick_builtin(&["Lee", "Park", "Kim", "Smith"], rng)
        ),
        "company" => pick_builtin(
            &["Example Systems", "Northwind", "Contoso", "Sample Labs"],
            rng,
        ),
        "country" => pick_builtin(
            &["South Korea", "Japan", "United States", "Germany", "Canada"],
            rng,
        ),
        "city" => pick_builtin(&["Seoul", "Tokyo", "London", "Berlin", "Toronto"], rng),
        "latitude" => format!("{:.6}", rng.gen_range(-90.0..=90.0)),
        "longitude" => format!("{:.6}", rng.gen_range(-180.0..=180.0)),
        "date" => time.format("%Y-%m-%d").to_string(),
        "time" => time.format("%H:%M:%S").to_string(),
        "file_name" => format!("file-{}.log", rng.gen_range(1..=10000)),
        "file_path" => format!("/var/log/app/file-{}.log", rng.gen_range(1..=10000)),
        "file_extension" => pick_builtin(&["log", "txt", "json", "csv", "gz"], rng),
        "mime_type" => pick_builtin(
            &[
                "text/plain",
                "application/json",
                "text/html",
                "application/octet-stream",
            ],
            rng,
        ),
        "response_time" => {
            let min: f64 = parameters
                .remove("min")
                .unwrap_or("1")
                .parse()
                .map_err(|_| "invalid response_time minimum")?;
            let max: f64 = parameters
                .remove("max")
                .unwrap_or("5000")
                .parse()
                .map_err(|_| "invalid response_time maximum")?;
            if !min.is_finite()
                || !max.is_finite()
                || min < 0.0
                || min > max
                || !(max - min).is_finite()
            {
                return Err("invalid response_time range".into());
            }
            let value = if min == max {
                min
            } else if min == 1.0 && max == 5000.0 {
                if rng.gen_bool(0.8) {
                    rng.gen_range(1.0..100.0)
                } else {
                    rng.gen_range(100.0..5000.0)
                }
            } else {
                rng.gen_range(min..max)
            };
            format!("{value:.2}")
        }
        "file_size" => {
            let ranges = [
                (1, 1024),
                (1024, 1048576),
                (1048576, 10485760),
                (10485760, 104857600),
            ];
            let index = WeightedIndex::new([3, 4, 2, 1])
                .map_err(|error| error.to_string())?
                .sample(rng);
            rng.gen_range(ranges[index].0..=ranges[index].1).to_string()
        }
        "bandwidth" => rng.gen_range(1024..=10485760).to_string(),
        "cpu_usage" => format!(
            "{:.1}",
            if rng.gen_bool(0.7) {
                rng.gen_range(0.0..30.0)
            } else {
                rng.gen_range(30.0..100.0)
            }
        ),
        "memory_usage" => rng.gen_range(100..=8192).to_string(),
        "api_key" => format!("{:032x}", rng.r#gen::<u128>()),
        "jwt_token" => format!(
            "{:020x}.{:032x}{:08x}.{:020x}",
            rng.gen_range(0..(1_u128 << 80)),
            rng.r#gen::<u128>(),
            rng.r#gen::<u32>(),
            rng.gen_range(0..(1_u128 << 80))
        ),
        "log_level" => choose_weighted(
            &[],
            &[
                ("INFO", 0.5),
                ("WARN", 0.2),
                ("ERROR", 0.15),
                ("DEBUG", 0.1),
                ("FATAL", 0.03),
                ("TRACE", 0.02),
            ],
            rng,
        )?
        .to_owned(),
        "error_code" => format!("E{}", rng.gen_range(1000..=9999)),
        "process_id" => rng.gen_range(1000..=99999).to_string(),
        "thread_id" => rng.gen_range(1..=999).to_string(),
        "database_name" => pick_builtin(
            &[
                "users",
                "products",
                "orders",
                "inventory",
                "logs",
                "analytics",
                "cache",
                "sessions",
            ],
            rng,
        ),
        "table_name" => pick_builtin(
            &[
                "user_accounts",
                "product_catalog",
                "order_history",
                "payment_info",
                "log_entries",
                "user_sessions",
                "api_keys",
                "system_config",
            ],
            rng,
        ),
        "query_duration" => format!(
            "{:.2}",
            if rng.gen_bool(0.9) {
                rng.gen_range(0.1..50.0)
            } else {
                rng.gen_range(50.0..1000.0)
            }
        ),
        "rows_affected" => if rng.gen_bool(0.8) {
            rng.gen_range(0..=10)
        } else {
            rng.gen_range(10..=10000)
        }
        .to_string(),
        "sentence" => "Request completed".to_string(),
        _ => return Err(format!("unsupported custom field {name}")),
    };
    if !parameters.is_empty() {
        return Err(format!("unsupported option for {name}"));
    }
    Ok(value)
}

impl TypeConfig {
    fn is_microservice(&self, name: &str) -> bool {
        name == "microservice"
            || (LogType::from_str(name, false).is_err()
                && (self.service_name.is_some()
                    || !self.custom_fields.events.is_empty()
                    || !self.custom_fields.db_operations.is_empty()
                    || !self.custom_fields.external_calls.is_empty()
                    || !self.custom_fields.gateway_calls.is_empty()))
    }
    fn validate_external(&self) -> Result<(), Box<dyn Error>> {
        if self.plugin_command.is_none()
            && (!self.plugin_args.is_empty() || self.plugin_timeout.is_some())
        {
            return Err("plugin_args and plugin_timeout require plugin_command".into());
        }
        if !self.patterns.is_empty()
            || self.custom_fields.error_levels.is_some()
            || !self.custom_fields.error_patterns.is_empty()
            || !self.custom_fields.ip_ranges.is_empty()
            || !self.custom_fields.status_codes.is_empty()
            || !self.custom_fields.user_agents.is_empty()
            || !self.custom_fields.url_patterns.is_empty()
            || !self.custom_fields.namespaces.is_empty()
            || self.custom_fields.namespace.is_some()
            || self.custom_fields.pod_name.is_some()
            || self.custom_fields.labels.is_some()
            || self.custom_fields.log_levels.is_some()
            || self.custom_fields.modules.is_some()
            || self.custom_fields.facilities.is_some()
            || self.custom_fields.severities.is_some()
            || self.custom_fields.hostname.is_some()
            || self.custom_fields.priority_style.is_some()
            || !self.custom_fields.container_names.is_empty()
            || self.custom_fields.container_name.is_some()
            || self.custom_fields.container_id.is_some()
            || self.custom_fields.images.is_some()
            || self.custom_fields.image_name.is_some()
            || self.custom_fields.event_types.is_some()
            || self.custom_fields.query_types.is_some()
            || !self.custom_fields.table_names.is_empty()
            || !self.custom_fields.endpoints.is_empty()
            || !self.custom_fields.models.is_empty()
            || !self.custom_fields.events.is_empty()
            || self.service_name.is_some()
            || !self.operations.is_empty()
            || !self.databases.is_empty()
            || !self.containers.is_empty()
            || !self.custom_fields.db_operations.is_empty()
            || !self.custom_fields.external_calls.is_empty()
            || !self.custom_fields.gateway_calls.is_empty()
            || !self.custom_fields.upstream_services.is_empty()
            || !self.custom_fields.response_times.is_empty()
            || self.error_levels.is_some()
            || !self.error_patterns.is_empty()
            || self.log_format.is_some()
            || !self.virtual_hosts.is_empty()
            || self.facilities.is_some()
            || !self.message_patterns.is_empty()
            || self.severities.is_some()
            || self.rfc_format.is_some()
            || self.priority_style.is_some()
            || self.hostname.is_some()
            || self.broker_ids.is_some()
            || self.topics.is_some()
            || self.partition_count.is_some()
            || self.log_levels.is_some()
            || !self.components.is_empty()
            || self.include_sql_queries.is_some()
            || !self.sql_query_patterns.is_empty()
            || !self.endpoints.is_empty()
            || !self.response_time_distribution.is_empty()
            || self.include_request_id.is_some()
            || self.include_user_id.is_some()
            || self.include_correlation_id.is_some()
            || self.format_type.is_some()
            || !self.container_names.is_empty()
            || self.container_name.is_some()
            || self.container_id.is_some()
            || self.images.is_some()
            || self.image_name.is_some()
            || !self.log_drivers.is_empty()
            || !self.container_patterns.is_empty()
            || !self.namespaces.is_empty()
            || self.namespace.is_some()
            || self.pod_name.is_some()
            || self.labels.is_some()
            || !self.pod_patterns.is_empty()
            || !self.event_patterns.is_empty()
            || self.event_types.is_some()
            || self.query_types.is_some()
            || !self.table_names.is_empty()
            || !self.models.is_empty()
            || self.height_range.is_some()
            || !self.module_weights.is_empty()
            || self.modules.is_some()
            || self.blockchain.is_some()
            || self.network.is_some()
            || self.template.is_some()
            || self.pattern_file.is_some()
            || self.pattern_name.is_some()
            || !self.frequency_maps.is_empty()
            || !self.correlation_rules.is_empty()
            || self.field_frequency.is_some()
            || self.field_inactive_value.is_some()
            || !self.fields.is_empty()
            || self.scenario_file.is_some()
            || self.patterns_dir.is_some()
            || self.error_category.is_some()
            || self.error_severity.is_some()
            || self.error_frequency.is_some()
        {
            return Err("external generator options must be under plugin_config".into());
        }
        Ok(())
    }
    fn validate_process(&self) -> Result<std::time::Duration, Box<dyn Error>> {
        self.validate_external()?;
        if self
            .plugin_command
            .as_ref()
            .is_none_or(|command| command.as_os_str().is_empty())
        {
            return Err("plugin_command is required".into());
        }
        let seconds = self.plugin_timeout.unwrap_or(5.0);
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err("plugin_timeout must be positive and finite".into());
        }
        let duration = std::time::Duration::try_from_secs_f64(seconds)?;
        if duration.is_zero() {
            return Err("plugin_timeout is too small".into());
        }
        Ok(duration)
    }
    fn settings(&self, name: &str) -> Result<GeneratorFields, Box<dyn Error>> {
        if self.plugin_command.is_some()
            || !self.plugin_args.is_empty()
            || self.plugin_timeout.is_some()
        {
            return Err("process plugin options require a custom log type".into());
        }
        let microservice = self.is_microservice(name);
        if !microservice
            && (self.service_name.is_some()
                || !self.operations.is_empty()
                || !self.custom_fields.events.is_empty()
                || !self.custom_fields.db_operations.is_empty()
                || !self.custom_fields.external_calls.is_empty()
                || !self.custom_fields.gateway_calls.is_empty()
                || !self.databases.is_empty()
                || !self.containers.is_empty()
                || !self.custom_fields.upstream_services.is_empty()
                || !self.custom_fields.response_times.is_empty())
        {
            return Err(format!("service settings are not valid for {name}").into());
        }
        let gateway = microservice && !self.custom_fields.upstream_services.is_empty();
        if microservice {
            if !gateway
                && self.custom_fields.events.is_empty()
                && self.custom_fields.db_operations.is_empty()
                && self.custom_fields.external_calls.is_empty()
                && self.custom_fields.gateway_calls.is_empty()
            {
                return Err("microservice requires at least one event template".into());
            }
            if !gateway
                && (!self.custom_fields.response_times.is_empty()
                    || !self.custom_fields.status_codes.is_empty())
            {
                return Err("response_times requires upstream_services".into());
            }
            if gateway {
                if self.endpoints.is_empty()
                    || self.custom_fields.status_codes.is_empty()
                    || self.custom_fields.response_times.is_empty()
                {
                    return Err(
                        "gateway requires endpoints, status_codes and response_times".into(),
                    );
                }
                if self
                    .custom_fields
                    .upstream_services
                    .iter()
                    .any(|name| name.is_empty() || name.chars().any(char::is_whitespace))
                    || self
                        .custom_fields
                        .status_codes
                        .keys()
                        .any(|code| !(100..=599).contains(code))
                    || self
                        .custom_fields
                        .response_times
                        .values()
                        .any(|range| range.min > range.max)
                {
                    return Err("invalid gateway settings".into());
                }
                crate::weights::checked(self.custom_fields.status_codes.values().copied())?;
                crate::weights::checked(
                    self.custom_fields
                        .response_times
                        .values()
                        .map(|range| range.weight),
                )?;
            }
            if self
                .service_name
                .as_ref()
                .is_some_and(|name| name.is_empty() || name.chars().any(char::is_whitespace))
                || self.operations.iter().any(|operation| {
                    operation.is_empty() || operation.chars().any(char::is_whitespace)
                })
            {
                return Err("invalid microservice name or operation".into());
            }
            if self
                .databases
                .iter()
                .any(|db| db.name.is_empty() || db.kind.is_empty())
                || self.containers.iter().any(String::is_empty)
            {
                return Err("invalid service database or container".into());
            }
            for container in &self.containers {
                if container.contains(['\n', '\r']) {
                    return Err("invalid service container".into());
                }
                crate::patterns::render(container, &[("instance", "1".to_owned())])
                    .map_err(io::Error::other)?;
            }
            for event in &self.custom_fields.events {
                event.validate(&self.operations)?;
            }
            for event in self
                .custom_fields
                .db_operations
                .iter()
                .chain(&self.custom_fields.external_calls)
                .chain(&self.custom_fields.gateway_calls)
            {
                crate::microservice::validate_event(event)?;
            }
        }
        if self.plugin_config.is_some() {
            return Err(format!("plugin_config is not valid for built-in type {name}").into());
        }
        if (name != "scenario" && (self.scenario_file.is_some() || self.error_frequency.is_some()))
            || (!matches!(name, "scenario" | "error_library") && self.patterns_dir.is_some())
            || (name != "error_library"
                && (self.error_category.is_some() || self.error_severity.is_some()))
        {
            return Err(format!("scenario settings are not valid for {name}").into());
        }
        if let Some(frequency) = &self.error_frequency {
            frequency.validate()?;
            if frequency.base_frequency == 0.0
                || frequency.peak_multiplier == 0.0
                || frequency.burst_multiplier == 0.0
            {
                return Err("scenario frequency values must be positive".into());
            }
        }
        if name == "scenario" && self.scenario_file.is_none() {
            return Err("scenario_file is required".into());
        }
        if name != "kafka_broker"
            && (self.broker_ids.is_some()
                || self.topics.is_some()
                || self.partition_count.is_some())
        {
            return Err(format!("Kafka settings are not valid for {name}").into());
        }
        if self.broker_ids.as_ref().is_some_and(Vec::is_empty)
            || self.topics.as_ref().is_some_and(Vec::is_empty)
            || self.partition_count == Some(0)
        {
            return Err(
                "Kafka broker_ids, topics, and partition_count must not be empty or zero".into(),
            );
        }
        if self
            .broker_ids
            .as_ref()
            .is_some_and(|ids| ids.iter().any(|id| *id > i32::MAX as u32))
            || self
                .partition_count
                .is_some_and(|count| count > i32::MAX as u32)
        {
            return Err(
                "Kafka broker IDs and partition count must fit a signed 32-bit integer".into(),
            );
        }
        if self.topics.as_ref().is_some_and(|topics| {
            topics.iter().any(|topic| {
                topic.is_empty()
                    || topic.len() > 249
                    || topic == "."
                    || topic == ".."
                    || !topic
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            })
        }) {
            return Err("invalid Kafka topic name".into());
        }
        if (!matches!(name, "nginx_error" | "apache_error")
            && (self.error_levels.is_some()
                || !self.error_patterns.is_empty()
                || self.custom_fields.error_levels.is_some()
                || !self.custom_fields.error_patterns.is_empty()))
            || (name != "apache_access"
                && (self.log_format.is_some() || !self.virtual_hosts.is_empty()))
            || (name != "syslog"
                && (self.facilities.is_some()
                    || !self.message_patterns.is_empty()
                    || self.severities.is_some()
                    || self.rfc_format.is_some()
                    || self.priority_style.is_some()
                    || self.hostname.is_some()
                    || self.custom_fields.facilities.is_some()
                    || self.custom_fields.severities.is_some()
                    || self.custom_fields.hostname.is_some()
                    || self.custom_fields.priority_style.is_some()))
            || (name != "fastapi"
                && name != "goloop"
                && name != "docker"
                && name != "kubernetes_pod"
                && name != "postgresql"
                && name != "mysql_error"
                && name != "django_request"
                && name != "django"
                && name != "kafka_broker"
                && name != "redis"
                && !microservice
                && self.log_levels.is_some())
            || (!matches!(name, "kubernetes_pod" | "kubernetes")
                && (!self.namespaces.is_empty()
                    || self.pod_name.is_some()
                    || !self.custom_fields.namespaces.is_empty()
                    || self.custom_fields.pod_name.is_some()))
            || (name != "kubernetes_pod"
                && (self.namespace.is_some()
                    || self.custom_fields.namespace.is_some()
                    || self.labels.is_some()
                    || self.custom_fields.labels.is_some()))
            || (!matches!(
                name,
                "kubernetes_pod"
                    | "docker"
                    | "postgresql"
                    | "mysql_error"
                    | "fastapi"
                    | "goloop"
                    | "redis"
                    | "django_request"
                    | "django"
            ) && self.custom_fields.log_levels.is_some())
            || (!matches!(name, "kubernetes" | "kubernetes_event")
                && (!self.pod_patterns.is_empty() || !self.event_patterns.is_empty()))
            || (name != "kubernetes_event"
                && (self.event_types.is_some() || self.custom_fields.event_types.is_some()))
            || (name != "django"
                && (!self.components.is_empty()
                    || self.include_sql_queries.is_some()
                    || !self.sql_query_patterns.is_empty()))
            || (name != "django_sql"
                && (!self.models.is_empty() || !self.custom_fields.models.is_empty()))
            || (!matches!(
                name,
                "mysql_query" | "postgresql" | "mysql_slow" | "postgresql_slow"
            ) && (self.query_types.is_some()
                || self.custom_fields.query_types.is_some()
                || !self.table_names.is_empty()
                || !self.custom_fields.table_names.is_empty()))
            || (name != "fastapi"
                && !gateway
                && (!self.endpoints.is_empty() || !self.custom_fields.endpoints.is_empty()))
            || (name != "fastapi"
                && (!self.response_time_distribution.is_empty()
                    || self.include_request_id.is_some()
                    || self.include_user_id.is_some()
                    || self.include_correlation_id.is_some()))
            || (name != "goloop" && name != "docker" && self.format_type.is_some())
            || (name != "docker"
                && (!self.container_names.is_empty()
                    || self.container_id.is_some()
                    || self.images.is_some()
                    || self.image_name.is_some()
                    || !self.custom_fields.container_names.is_empty()
                    || self.custom_fields.container_id.is_some()
                    || self.custom_fields.images.is_some()
                    || self.custom_fields.image_name.is_some()
                    || !self.log_drivers.is_empty()
                    || !self.container_patterns.is_empty()))
            || (!matches!(name, "docker" | "kubernetes_pod")
                && (self.container_name.is_some() || self.custom_fields.container_name.is_some()))
            || (name != "goloop"
                && (self.height_range.is_some()
                    || !self.module_weights.is_empty()
                    || self.modules.is_some()
                    || self.custom_fields.modules.is_some()
                    || self.blockchain.is_some()))
            || (name != "goloop" && self.network.is_some())
            || (name != "custom"
                && (self.template.is_some()
                    || self.pattern_file.is_some()
                    || self.pattern_name.is_some()
                    || !self.frequency_maps.is_empty()
                    || !self.correlation_rules.is_empty()
                    || self.field_frequency.is_some()
                    || self.field_inactive_value.is_some()
                    || !self.fields.is_empty()))
        {
            return Err(format!("unsupported settings for {name}").into());
        }
        if self.virtual_hosts.iter().any(|host| {
            host.is_empty()
                || !host
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
        }) {
            return Err("invalid Apache virtual host".into());
        }
        if self
            .log_format
            .as_deref()
            .is_some_and(|format| format != "common" && format != "combined")
        {
            return Err("apache log_format must be common or combined".into());
        }
        if (self.error_levels.is_some() && self.custom_fields.error_levels.is_some())
            || (!self.error_patterns.is_empty() && !self.custom_fields.error_patterns.is_empty())
        {
            return Err("error_levels and error_patterns must each appear in one location".into());
        }
        let error_levels = weighted_choices(
            self.error_levels
                .as_ref()
                .or(self.custom_fields.error_levels.as_ref()),
        )?;
        if (self.error_levels.is_some() || self.custom_fields.error_levels.is_some())
            && error_levels.is_empty()
        {
            return Err("error_levels must not be empty".into());
        }
        if error_levels.iter().any(|(level, _)| {
            !["error", "warn", "notice", "info", "crit", "alert", "emerg"].contains(&level.as_str())
        }) {
            return Err("invalid web server error level".into());
        }
        if self
            .error_patterns
            .iter()
            .chain(&self.custom_fields.error_patterns)
            .any(|pattern| pattern.is_empty() || pattern.contains(['\n', '\r']))
        {
            return Err("invalid error pattern".into());
        }
        for pattern in &self.pod_patterns {
            if pattern.is_empty() || pattern.contains(['\n', '\r', ' ']) {
                return Err("invalid Kubernetes pod pattern".into());
            }
            crate::patterns::render(
                pattern,
                &[("deployment", "app".into()), ("replica_id", "abc12".into())],
            )
            .map_err(io::Error::other)?;
        }
        for pattern in &self.event_patterns {
            if pattern.is_empty() || pattern.contains(['\n', '\r']) {
                return Err("invalid Kubernetes event pattern".into());
            }
            crate::patterns::render(
                pattern,
                &[
                    ("pod_name", "app-123".into()),
                    ("node_name", "node-1".into()),
                    ("container_name", "app".into()),
                ],
            )
            .map_err(io::Error::other)?;
        }
        if (self.facilities.is_some() && self.custom_fields.facilities.is_some())
            || (self.severities.is_some() && self.custom_fields.severities.is_some())
            || (self.hostname.is_some() && self.custom_fields.hostname.is_some())
            || (self.priority_style.is_some() && self.custom_fields.priority_style.is_some())
        {
            return Err("syslog settings must each appear in one location".into());
        }
        let facilities = weighted_choices(
            self.facilities
                .as_ref()
                .or(self.custom_fields.facilities.as_ref()),
        )?;
        let severities = weighted_choices(
            self.severities
                .as_ref()
                .or(self.custom_fields.severities.as_ref()),
        )?;
        if (self.facilities.is_some() || self.custom_fields.facilities.is_some())
            && facilities.is_empty()
            || (self.severities.is_some() || self.custom_fields.severities.is_some())
                && severities.is_empty()
        {
            return Err("syslog facilities and severities must not be empty".into());
        }
        for (facility, _) in &facilities {
            syslog_priority(facility, "info")?;
        }
        for (facility, patterns) in &self.message_patterns {
            syslog_priority(facility, "info")?;
            if patterns.is_empty() {
                return Err("syslog message_patterns must not be empty".into());
            }
            for pattern in patterns {
                if pattern.is_empty() || pattern.contains(['\n', '\r']) {
                    return Err("invalid syslog message pattern".into());
                }
                crate::patterns::render_syslog(
                    pattern,
                    Utc::now(),
                    "192.0.2.1",
                    &mut StdRng::seed_from_u64(0),
                )
                .map_err(io::Error::other)?;
            }
        }
        for (severity, _) in &severities {
            syslog_priority("auth", severity)?;
        }
        if self
            .rfc_format
            .as_deref()
            .is_some_and(|s| !["3164", "5424"].contains(&s))
        {
            return Err("syslog rfc_format must be 3164 or 5424".into());
        }
        if self
            .priority_style
            .as_deref()
            .or(self.custom_fields.priority_style.as_deref())
            .is_some_and(|s| !["pri", "bracket", "plain", "none"].contains(&s))
        {
            return Err("invalid syslog priority_style".into());
        }
        if self
            .hostname
            .as_ref()
            .or(self.custom_fields.hostname.as_ref())
            .is_some_and(|s| s.is_empty() || s.chars().any(char::is_whitespace))
        {
            return Err("invalid syslog hostname".into());
        }
        if self.log_levels.is_some() && self.custom_fields.log_levels.is_some() {
            return Err("log_levels must appear in one location".into());
        }
        if (!self.container_names.is_empty() && !self.custom_fields.container_names.is_empty())
            || (self.container_name.is_some() && self.custom_fields.container_name.is_some())
            || (self.container_id.is_some() && self.custom_fields.container_id.is_some())
            || (self.images.is_some() && self.custom_fields.images.is_some())
            || (self.image_name.is_some() && self.custom_fields.image_name.is_some())
        {
            return Err("Docker container settings must appear in one location".into());
        }
        if !self.endpoints.is_empty() && !self.custom_fields.endpoints.is_empty() {
            return Err("endpoints must appear in one location".into());
        }
        if self.event_types.is_some() && self.custom_fields.event_types.is_some() {
            return Err("event_types must appear in one location".into());
        }
        if self.query_types.is_some() && self.custom_fields.query_types.is_some() {
            return Err("query_types must appear in one location".into());
        }
        if !self.table_names.is_empty() && !self.custom_fields.table_names.is_empty() {
            return Err("table_names must appear in one location".into());
        }
        if !self.models.is_empty() && !self.custom_fields.models.is_empty() {
            return Err("models must appear in one location".into());
        }
        if !self.namespaces.is_empty() && !self.custom_fields.namespaces.is_empty() {
            return Err("namespaces must appear in one location".into());
        }
        if (self.namespace.is_some() && self.custom_fields.namespace.is_some())
            || (self.labels.is_some() && self.custom_fields.labels.is_some())
        {
            return Err("Kubernetes metadata must appear in one location".into());
        }
        if self.pod_name.is_some() && self.custom_fields.pod_name.is_some() {
            return Err("pod_name must appear in one location".into());
        }
        let levels = weighted_choices(
            self.log_levels
                .as_ref()
                .or(self.custom_fields.log_levels.as_ref()),
        )?;
        if name == "kafka_broker"
            && ((self.log_levels.is_some() && levels.is_empty())
                || levels
                    .iter()
                    .any(|(level, _)| !["INFO", "WARN", "ERROR"].contains(&level.as_str())))
        {
            return Err("invalid Kafka log level".into());
        }
        if name == "redis"
            && levels.iter().any(|(level, _)| {
                !["DEBUG", "VERBOSE", "NOTICE", "WARNING"].contains(&level.as_str())
            })
        {
            return Err("invalid Redis log level".into());
        }
        if microservice
            && levels.iter().any(|(level, _)| {
                !["DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"].contains(&level.as_str())
            })
        {
            return Err("invalid microservice log level".into());
        }
        if matches!(name, "django_request" | "django")
            && levels.iter().any(|(level, _)| {
                !["INFO", "WARNING", "ERROR", "DEBUG", "CRITICAL"].contains(&level.as_str())
            })
        {
            return Err("invalid Django log level".into());
        }
        if self.components.iter().any(|component| {
            !["django.request", "django.db.backends", "django.security"]
                .contains(&component.as_str())
        }) {
            return Err("unsupported Django component".into());
        }
        if !self.components.is_empty()
            && !self.include_sql_queries.unwrap_or(false)
            && self
                .components
                .iter()
                .all(|component| component == "django.db.backends")
        {
            return Err("no enabled Django components".into());
        }
        if self
            .sql_query_patterns
            .iter()
            .any(|query| query.is_empty() || query.contains(['\n', '\r']))
        {
            return Err("invalid Django SQL pattern".into());
        }
        if self.include_sql_queries == Some(false) && !self.sql_query_patterns.is_empty() {
            return Err("sql_query_patterns requires include_sql_queries".into());
        }
        let models = if self.models.is_empty() {
            &self.custom_fields.models
        } else {
            &self.models
        };
        if models.iter().any(|model| {
            model.is_empty() || !model.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }) {
            return Err("invalid Django model name".into());
        }
        let event_types = weighted_choices(
            self.event_types
                .as_ref()
                .or(self.custom_fields.event_types.as_ref()),
        )?;
        let queries = weighted_choices(
            self.query_types
                .as_ref()
                .or(self.custom_fields.query_types.as_ref()),
        )?;
        let valid_queries: &[&str] = if matches!(name, "mysql_slow" | "postgresql_slow") {
            &["SELECT", "INSERT", "UPDATE", "DELETE"]
        } else if name == "postgresql" {
            &[
                "SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "DROP", "ALTER",
            ]
        } else {
            &["SELECT", "INSERT", "UPDATE", "DELETE", "SHOW", "DESCRIBE"]
        };
        if queries
            .iter()
            .any(|(query, _)| !valid_queries.contains(&query.as_str()))
        {
            return Err("invalid database query type".into());
        }
        if name == "postgresql"
            && levels.iter().any(|(level, _)| {
                !["LOG", "INFO", "WARNING", "ERROR", "FATAL", "PANIC"].contains(&level.as_str())
            })
        {
            return Err("invalid PostgreSQL log level".into());
        }
        if name == "mysql_error"
            && levels
                .iter()
                .any(|(level, _)| !["Note", "Warning", "Error"].contains(&level.as_str()))
        {
            return Err("invalid MySQL log level".into());
        }
        let tables = if self.table_names.is_empty() {
            &self.custom_fields.table_names
        } else {
            &self.table_names
        };
        if tables.iter().any(|name| {
            name.split('.').any(|part| {
                part.is_empty()
                    || !part
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    || !part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        }) {
            return Err("invalid database table name".into());
        }
        if name == "kubernetes_pod"
            && levels.iter().any(|(level, _)| {
                !["INFO", "WARN", "ERROR", "DEBUG", "FATAL"].contains(&level.as_str())
            })
        {
            return Err("invalid Kubernetes log level".into());
        }
        if name == "kubernetes_event"
            && event_types
                .iter()
                .any(|(event, _)| !["Normal", "Warning"].contains(&event.as_str()))
        {
            return Err("invalid Kubernetes event type".into());
        }
        let namespaces = if self.namespaces.is_empty() {
            &self.custom_fields.namespaces
        } else {
            &self.namespaces
        };
        let pod_name = self
            .pod_name
            .as_ref()
            .or(self.custom_fields.pod_name.as_ref());
        let namespace = self
            .namespace
            .as_ref()
            .or(self.custom_fields.namespace.as_ref());
        let container_name = self
            .container_name
            .as_ref()
            .or(self.custom_fields.container_name.as_ref());
        let labels = self.labels.as_ref().or(self.custom_fields.labels.as_ref());
        if namespaces
            .iter()
            .chain(pod_name)
            .chain(namespace)
            .chain(
                (name == "kubernetes_pod")
                    .then_some(container_name)
                    .flatten(),
            )
            .any(|value| value.is_empty() || value.chars().any(char::is_whitespace))
        {
            return Err("invalid Kubernetes name".into());
        }
        if labels.is_some_and(|labels| {
            labels.iter().any(|(key, value)| {
                key.is_empty()
                    || key.chars().any(char::is_whitespace)
                    || value.chars().any(char::is_control)
            })
        }) {
            return Err("invalid Kubernetes label".into());
        }
        if (self.modules.is_some() && self.custom_fields.modules.is_some())
            || (!self.module_weights.is_empty()
                && (self.modules.is_some() || self.custom_fields.modules.is_some()))
        {
            return Err("use modules or module_weights in one location".into());
        }
        let modules = if !self.module_weights.is_empty() {
            weighted_choices(Some(&Choices::Weights(self.module_weights.clone())))?
        } else {
            weighted_choices(
                self.modules
                    .as_ref()
                    .or(self.custom_fields.modules.as_ref()),
            )?
        };
        if name == "goloop" {
            if levels
                .iter()
                .any(|(v, _)| !["D", "I", "W", "E", "F"].contains(&v.as_str()))
            {
                return Err("invalid goloop log level".into());
            }
            if modules
                .iter()
                .any(|(v, _)| v.is_empty() || !v.chars().all(|c| c.is_ascii_alphanumeric()))
            {
                return Err("invalid goloop module".into());
            }
            if self
                .format_type
                .as_deref()
                .is_some_and(|s| s != "text" && s != "json")
            {
                return Err("goloop format_type must be text or json".into());
            }
        }
        if name == "docker" {
            if !self.log_drivers.is_empty() && self.format_type.is_some() {
                return Err("docker log_drivers conflicts with format_type".into());
            }
            if self
                .log_drivers
                .iter()
                .any(|driver| !["json-file", "syslog", "journald"].contains(&driver.as_str()))
            {
                return Err("unsupported docker log driver".into());
            }
            for (name, templates) in &self.container_patterns {
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                    || templates.is_empty()
                {
                    return Err("invalid docker container patterns".into());
                }
                for template in templates {
                    crate::patterns::validate_docker_template(template)
                        .map_err(io::Error::other)?;
                }
            }
            for name in self
                .container_names
                .iter()
                .chain(self.container_name.iter())
                .chain(self.custom_fields.container_names.iter())
                .chain(self.custom_fields.container_name.iter())
            {
                crate::patterns::render(name, &[("instance_id", "1".into())])
                    .map_err(io::Error::other)?;
            }
            if self
                .format_type
                .as_deref()
                .is_some_and(|s| s != "text" && s != "json")
            {
                return Err("docker format_type must be text or json".into());
            }
            if levels.iter().any(|(level, _)| {
                !["info", "warn", "error", "debug", "fatal"].contains(&level.as_str())
            }) {
                return Err("invalid docker log level".into());
            }
            if self
                .container_names
                .iter()
                .chain(self.container_name.iter())
                .chain(self.custom_fields.container_names.iter())
                .chain(self.custom_fields.container_name.iter())
                .any(|name| name.is_empty() || name.contains(['\n', '\r', ':']))
            {
                return Err("invalid docker container name".into());
            }
            if self
                .container_id
                .as_ref()
                .or(self.custom_fields.container_id.as_ref())
                .is_some_and(|id| id.is_empty() || id.chars().any(char::is_control))
            {
                return Err("invalid docker container id".into());
            }
            if self
                .images
                .as_ref()
                .or(self.custom_fields.images.as_ref())
                .is_some_and(Vec::is_empty)
                || self
                    .images
                    .iter()
                    .flatten()
                    .chain(self.image_name.iter())
                    .chain(self.custom_fields.images.iter().flatten())
                    .chain(self.custom_fields.image_name.iter())
                    .any(|image| image.is_empty() || image.chars().any(char::is_whitespace))
            {
                return Err("invalid docker image name".into());
            }
        }
        let height_range = self
            .height_range
            .or_else(|| self.blockchain.as_ref().and_then(|b| b.height_range));
        if height_range.is_some_and(|[min, max]| min > max) {
            return Err("invalid goloop height_range".into());
        }
        let network = self.network.clone().unwrap_or_default();
        if name == "goloop" {
            network
                .local_ip
                .parse::<Ipv4Addr>()
                .map_err(|_| "invalid goloop local_ip")?;
            if network.p2p_port == 0 {
                return Err("goloop p2p_port must be greater than zero".into());
            }
        }
        let network_id = self
            .blockchain
            .as_ref()
            .and_then(|b| b.network_id.as_ref())
            .map_or("0x1", String::as_str);
        if network_id.is_empty() || network_id.chars().any(char::is_whitespace) {
            return Err("invalid goloop network_id".into());
        }
        let revision_range = self
            .blockchain
            .as_ref()
            .and_then(|b| b.revision_range)
            .unwrap_or([10, 25]);
        if revision_range[0] > revision_range[1] {
            return Err("invalid goloop revision_range".into());
        }
        if self
            .response_time_distribution
            .values()
            .any(|range| range.min > range.max)
        {
            return Err("invalid response_time_distribution range".into());
        }
        if !self.response_time_distribution.is_empty() {
            crate::weights::checked(
                self.response_time_distribution
                    .values()
                    .map(|range| range.weight),
            )?;
        }
        let fastapi_levels = if name == "fastapi" {
            levels.clone()
        } else {
            Vec::new()
        };
        let endpoints = if self.endpoints.is_empty() {
            &self.custom_fields.endpoints
        } else {
            &self.endpoints
        };
        let fastapi_status_codes: Vec<_> = if name == "fastapi" {
            self.custom_fields
                .status_codes
                .iter()
                .map(|(status, weight)| (*status, *weight))
                .collect()
        } else {
            Vec::new()
        };
        if !fastapi_status_codes.is_empty() {
            crate::weights::checked(fastapi_status_codes.iter().map(|(_, weight)| *weight))?;
        }
        if error_levels
            .iter()
            .map(|(level, _)| level)
            .chain(fastapi_levels.iter().map(|(level, _)| level))
            .any(|v| v.is_empty() || v.contains(['\n', '\r', '[', ']']))
            || endpoints
                .iter()
                .any(|v| v.is_empty() || v.contains(['\n', '\r', ' ', '"']))
        {
            return Err("invalid log level or endpoint".into());
        }
        if self.pattern_file.is_some() && (self.template.is_some() || !self.fields.is_empty()) {
            return Err("pattern_file conflicts with template and fields".into());
        }
        if self.pattern_file.is_none() && self.pattern_name.is_some() {
            return Err("pattern_name requires pattern_file".into());
        }
        let (template, custom_fields) = if let Some(path) = &self.pattern_file {
            let (template, fields) = crate::pattern_file::load(path, self.pattern_name.as_deref())?;
            (Some(template), fields)
        } else {
            (self.template.clone(), self.fields.clone())
        };
        if self.field_frequency.is_some()
            && (!self.frequency_maps.is_empty() || !self.correlation_rules.is_empty())
        {
            return Err(
                "field_frequency conflicts with frequency_maps and correlation_rules".into(),
            );
        }
        if self.field_inactive_value.is_some() && self.field_frequency.is_none() {
            return Err("field_inactive_value requires field_frequency".into());
        }
        if self
            .field_inactive_value
            .as_deref()
            .is_some_and(|value| value.contains(['\n', '\r']))
        {
            return Err("invalid field_inactive_value".into());
        }
        if let Some(frequency) = &self.field_frequency {
            frequency.validate()?;
            let names: std::collections::HashSet<_> = crate::template_syntax::parse(
                template
                    .as_deref()
                    .ok_or("custom log type requires template")?,
            )
            .filter_map(|part| match part {
                Ok(crate::template_syntax::Part::Field(spec)) => {
                    Some(spec.split(':').next().unwrap_or(""))
                }
                _ => None,
            })
            .collect();
            if frequency
                .time_patterns
                .keys()
                .chain(frequency.burst_patterns.keys())
                .any(|name| !names.contains(name.as_str()))
            {
                return Err("field_frequency field not in template".into());
            }
        }
        let frequencies = self
            .field_frequency
            .as_ref()
            .map_or(&self.frequency_maps, |config| &config.distributions);
        let correlations = self
            .field_frequency
            .as_ref()
            .map_or(&self.correlation_rules, |config| &config.correlation_rules);
        if name == "custom" {
            crate::template_frequency::validate(
                template
                    .as_deref()
                    .ok_or("custom log type requires template")?,
                frequencies,
                correlations,
            )
            .map_err(io::Error::other)?;
        }
        let fields = GeneratorFields {
            error_levels,
            error_patterns: if self.error_patterns.is_empty() {
                self.custom_fields.error_patterns.clone()
            } else {
                self.error_patterns.clone()
            },
            virtual_hosts: self.virtual_hosts.clone(),
            facilities,
            syslog_message_patterns: self.message_patterns.clone(),
            severities,
            syslog_rfc: self.rfc_format.clone(),
            syslog_priority_style: self
                .priority_style
                .clone()
                .or_else(|| self.custom_fields.priority_style.clone()),
            syslog_hostname: self
                .hostname
                .clone()
                .or_else(|| self.custom_fields.hostname.clone()),
            kafka_broker_ids: self.broker_ids.clone().unwrap_or_default(),
            kafka_topics: self.topics.clone().unwrap_or_default(),
            kafka_partition_count: self.partition_count,
            kafka_levels: if name == "kafka_broker" {
                levels.clone()
            } else {
                Vec::new()
            },
            redis_levels: if name == "redis" {
                levels.clone()
            } else {
                Vec::new()
            },
            log_levels: fastapi_levels,
            endpoints: endpoints.clone(),
            fastapi_status_codes,
            fastapi_response_times: self.response_time_distribution.values().cloned().collect(),
            fastapi_no_request_id: self.include_request_id == Some(false),
            fastapi_user_id: self.include_user_id.unwrap_or(false),
            fastapi_correlation_id: self.include_correlation_id.unwrap_or(false),
            template: template.clone(),
            fields: custom_fields.clone(),
            frequency_maps: frequencies.clone(),
            correlation_rules: correlations.clone(),
            file_fields: if name == "custom" {
                load_file_fields(
                    template
                        .as_deref()
                        .ok_or("custom log type requires template")?,
                    &custom_fields,
                )?
            } else {
                BTreeMap::new()
            },
            goloop_levels: if name == "goloop" {
                levels.clone()
            } else {
                Vec::new()
            },
            goloop_modules: modules,
            goloop_format: if name == "goloop" {
                self.format_type.clone()
            } else {
                None
            },
            goloop_height_range: height_range,
            goloop_network: network,
            goloop_network_id: (name == "goloop").then(|| network_id.to_owned()),
            goloop_revision_range: (name == "goloop").then_some(revision_range),
            docker_format: if name == "docker" {
                self.format_type.clone()
            } else {
                None
            },
            docker_levels: if name == "docker" {
                levels.clone()
            } else {
                Vec::new()
            },
            docker_names: if self.container_names.is_empty() {
                self.custom_fields.container_names.clone()
            } else {
                self.container_names.clone()
            },
            docker_name: self
                .container_name
                .clone()
                .or_else(|| self.custom_fields.container_name.clone()),
            docker_drivers: self.log_drivers.clone(),
            docker_driver: None,
            docker_container_id: self
                .container_id
                .clone()
                .or_else(|| self.custom_fields.container_id.clone())
                .unwrap_or_default(),
            docker_images: self
                .images
                .clone()
                .or_else(|| self.custom_fields.images.clone())
                .unwrap_or_default(),
            docker_image_name: self
                .image_name
                .clone()
                .or_else(|| self.custom_fields.image_name.clone())
                .unwrap_or_default(),
            docker_patterns: self.container_patterns.clone(),
            kubernetes_levels: if name == "kubernetes_pod" {
                levels.clone()
            } else {
                Vec::new()
            },
            kubernetes_namespaces: namespaces.clone(),
            kubernetes_namespace: namespace.cloned(),
            kubernetes_pod_name: pod_name.cloned(),
            kubernetes_container_name: if name == "kubernetes_pod" {
                container_name.cloned()
            } else {
                None
            },
            kubernetes_labels: labels.cloned().unwrap_or_default(),
            kubernetes_pod_patterns: self.pod_patterns.clone(),
            kubernetes_event_patterns: self.event_patterns.clone(),
            kubernetes_event_types: event_types,
            database_levels: if matches!(name, "postgresql" | "mysql_error") {
                levels.clone()
            } else {
                Vec::new()
            },
            database_queries: queries,
            database_tables: tables.clone(),
            django_levels: if matches!(name, "django_request" | "django") {
                levels.clone()
            } else {
                Vec::new()
            },
            django_models: models.clone(),
            django_components: if self.components.is_empty()
                && self.include_sql_queries.unwrap_or(false)
            {
                vec!["django.request".into(), "django.db.backends".into()]
            } else {
                self.components
                    .iter()
                    .filter(|component| {
                        component.as_str() != "django.db.backends"
                            || self.include_sql_queries.unwrap_or(false)
                    })
                    .cloned()
                    .collect()
            },
            django_sql_patterns: self.sql_query_patterns.clone(),
            // Legacy service entries can omit a service name and operations.
            service_name: if microservice {
                Some(self.service_name.clone().unwrap_or_else(|| name.to_owned()))
            } else {
                None
            },
            service_operations: self.operations.clone(),
            service_events: self
                .custom_fields
                .events
                .iter()
                .cloned()
                .chain(
                    self.custom_fields
                        .db_operations
                        .iter()
                        .chain(&self.custom_fields.external_calls)
                        .chain(&self.custom_fields.gateway_calls)
                        .cloned()
                        .map(crate::microservice::ServiceEvent::Message),
                )
                .collect(),
            service_databases: self.databases.iter().map(|db| db.name.clone()).collect(),
            service_containers: self.containers.clone(),
            error_library: Vec::new(),
            gateway_upstreams: self.custom_fields.upstream_services.clone(),
            gateway_statuses: if gateway {
                self.custom_fields
                    .status_codes
                    .iter()
                    .map(|(code, weight)| (*code, *weight))
                    .collect()
            } else {
                Vec::new()
            },
            gateway_response_times: self
                .custom_fields
                .response_times
                .values()
                .cloned()
                .collect(),
            service_levels: if microservice {
                levels.clone()
            } else {
                Vec::new()
            },
        };
        if name == "custom" {
            render_custom(&fields, Utc::now(), &mut StdRng::seed_from_u64(0), 0)
                .map_err(io::Error::other)?;
        }
        Ok(fields)
    }
    fn nginx_fields(&self, name: &str) -> Result<NginxFields, Box<dyn Error>> {
        let fields = &self.custom_fields;
        if name != "nginx_access"
            && name != "apache_access"
            && (!fields.ip_ranges.is_empty()
                || (!matches!(name, "fastapi" | "django_request" | "django")
                    && !self.is_microservice(name)
                    && !fields.status_codes.is_empty())
                || !fields.user_agents.is_empty()
                || (!matches!(name, "django_request" | "django")
                    && !fields.url_patterns.is_empty())
                || !self.patterns.is_empty())
        {
            return Err(format!("custom fields or patterns for {name} are not supported").into());
        }
        let mut ranges = Vec::new();
        for range in &fields.ip_ranges {
            let (address, prefix) = range.split_once('/').ok_or("IP range must be IPv4 CIDR")?;
            let prefix: u8 = prefix.parse()?;
            if prefix > 32 {
                return Err("IPv4 prefix must be at most 32".into());
            }
            ranges.push((address.parse()?, prefix));
        }
        let statuses: Vec<_> = fields
            .status_codes
            .iter()
            .map(|(code, weight)| (*code, *weight))
            .collect();
        if !statuses.is_empty() {
            crate::weights::checked(statuses.iter().map(|(_, weight)| *weight))?;
        }
        if fields
            .user_agents
            .iter()
            .any(|s| s.contains(['\n', '\r', '"']))
            || fields
                .url_patterns
                .iter()
                .any(|s| s.contains(['\n', '\r', '"', ' ']))
        {
            return Err("invalid characters in Nginx field".into());
        }
        if self
            .patterns
            .iter()
            .any(|p| p != "common" && p != "combined")
        {
            return Err("Nginx patterns must be common or combined".into());
        }
        Ok(NginxFields {
            ip_ranges: ranges,
            status_codes: statuses,
            user_agents: fields.user_agents.clone(),
            url_patterns: fields.url_patterns.clone(),
        })
    }
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, Box<dyn Error>> {
    if let Ok(time) = DateTime::parse_from_rfc3339(value) {
        return Ok(time.with_timezone(&Utc));
    }
    if let Ok(time) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return Ok(Utc.from_utc_datetime(&time));
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")?;
    Ok(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).ok_or("invalid date")?))
}

fn check_memory_limit(limit: u64) -> io::Result<()> {
    let usage = memory_stats::memory_stats()
        .ok_or_else(|| io::Error::other("cannot read process memory usage"))?;
    if usage.physical_mem as u64 > limit {
        return Err(io::Error::other(format!(
            "memory limit exceeded: {} bytes used, limit {limit} bytes",
            usage.physical_mem
        )));
    }
    Ok(())
}

fn parse_retry_delay(seconds: f64) -> io::Result<std::time::Duration> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(io::Error::other(
            "retry_delay must be finite and nonnegative",
        ));
    }
    std::time::Duration::try_from_secs_f64(seconds).map_err(io::Error::other)
}

fn network_timeout(seconds: f64) -> io::Result<std::time::Duration> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "network_timeout must be a positive finite number",
        ));
    }
    let timeout = std::time::Duration::try_from_secs_f64(seconds).map_err(io::Error::other)?;
    if timeout.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "network_timeout is too small",
        ));
    }
    Ok(timeout)
}

fn resolve_kind(
    name: &str,
    settings: Option<&TypeConfig>,
    registry: &GeneratorRegistry,
) -> Result<Option<LogType>, Box<dyn Error>> {
    if name == "kubernetes" {
        return Ok(Some(LogType::KubernetesEvent));
    }
    if name == "django" {
        return Ok(Some(LogType::DjangoRequest));
    }
    if let Ok(kind) = LogType::from_str(name, false) {
        return Ok(Some(kind));
    }
    if registry.contains(name) && settings.is_some_and(|settings| settings.plugin_command.is_some())
    {
        return Err(format!("plugin command conflicts with registered generator: {name}").into());
    }
    if settings.is_some_and(|settings| settings.plugin_command.is_some()) {
        return Ok(Some(LogType::Process));
    }
    if registry.contains(name) {
        return Ok(None);
    }
    if settings.is_some_and(|settings| settings.is_microservice(name)) {
        return Ok(Some(LogType::Microservice));
    }
    let mut message = format!("unknown log type: {name}");
    let known = LogType::value_variants()
        .iter()
        .filter_map(|kind| kind.to_possible_value())
        .map(|value| value.get_name().to_owned())
        .chain(registry.types().map(str::to_owned));
    if let Some(closest) = closest_name(name, known) {
        message.push_str(&format!("\n  tip: did you mean '{closest}'?"));
    }
    message.push_str("\n  try: log-forge list-types");
    Err(message.into())
}

fn closest_name(name: &str, candidates: impl Iterator<Item = String>) -> Option<String> {
    let limit = (name.chars().count() / 3).max(1);
    candidates
        .map(|candidate| (edit_distance(name, &candidate), candidate))
        .filter(|(distance, _)| *distance <= limit)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_char) in left.chars().enumerate() {
        let mut current = vec![row + 1];
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != *right_char);
            current.push(
                substitution
                    .min(previous[column + 1] + 1)
                    .min(current[column] + 1),
            );
        }
        previous = current;
    }
    previous[right.len()]
}

fn select_traced(
    weights: &WeightedIndex<f64>,
    planner: &mut Option<crate::tracing::TracePlanner>,
    rng: &mut StdRng,
) -> io::Result<(usize, Option<crate::tracing::TraceContext>)> {
    if let Some(planner) = planner {
        if let Some(step) = planner.next(None, rng)? {
            return Ok(step);
        }
        return planner
            .next(Some(weights.sample(rng)), rng)?
            .ok_or_else(|| io::Error::other("missing trace selection"));
    }
    Ok((weights.sample(rng), None))
}

fn advance_virtual(
    time: DateTime<Utc>,
    interval: f64,
    random: bool,
    rng: &mut StdRng,
) -> Result<DateTime<Utc>, &'static str> {
    let seconds = interval * if random { rng.gen_range(0.0..1.0) } else { 1.0 };
    if seconds > i64::MAX as f64 / 1_000_000.0 {
        return Err("interval is too large");
    }
    time.checked_add_signed(Duration::microseconds((seconds * 1_000_000.0) as i64))
        .ok_or("virtual time overflow")
}

fn merge_defaults(default: &mut serde_json::Value, overrides: serde_json::Value) {
    match (default, overrides) {
        (serde_json::Value::Object(base), serde_json::Value::Object(overrides)) => {
            for (key, value) in overrides {
                if let Some(existing) = base.get_mut(&key) {
                    merge_defaults(existing, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, value) => *base = value,
    }
}

fn remove_inherited_aliases(defaults: &mut serde_json::Value, overrides: &serde_json::Value) {
    let Some(types) = overrides
        .get("log_generator")
        .and_then(|value| value.get("log_types"))
        .and_then(serde_json::Value::as_object)
    else {
        return;
    };
    let Some(default_types) = defaults
        .get_mut("log_generator")
        .and_then(|value| value.get_mut("log_types"))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    for (name, settings) in types {
        let Some(user) = settings.as_object() else {
            continue;
        };
        let Some(base) = default_types
            .get_mut(name)
            .and_then(serde_json::Value::as_object_mut)
        else {
            continue;
        };
        if name == "goloop"
            && user
                .get("module_weights")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|weights| !weights.is_empty())
            && !user.contains_key("modules")
        {
            base.remove("modules");
        }
        if name == "goloop"
            && user
                .get("custom_fields")
                .and_then(|v| v.get("modules"))
                .is_some_and(|v| !v.is_null())
            && !user.contains_key("modules")
            && !user.contains_key("module_weights")
        {
            base.remove("modules");
            base.remove("module_weights");
        }
        if name == "syslog"
            && let Some(custom) = user
                .get("custom_fields")
                .and_then(serde_json::Value::as_object)
        {
            for field in ["facilities", "severities", "hostname", "priority_style"] {
                if custom.get(field).is_some_and(|value| !value.is_null())
                    && !user.contains_key(field)
                {
                    base.remove(field);
                }
            }
        }
        if matches!(name.as_str(), "fastapi" | "goloop")
            && let Some(custom) = user
                .get("custom_fields")
                .and_then(serde_json::Value::as_object)
        {
            for &field in if name == "fastapi" {
                &["log_levels", "endpoints"][..]
            } else {
                &["log_levels"][..]
            } {
                if custom.get(field).is_some_and(|value| {
                    !value.is_null()
                        && value.as_array().is_none_or(|values| !values.is_empty())
                        && value.as_object().is_none_or(|values| !values.is_empty())
                }) && !user.contains_key(field)
                {
                    base.remove(field);
                }
            }
        }
        if matches!(name.as_str(), "nginx_error" | "apache_error")
            && let Some(custom) = user
                .get("custom_fields")
                .and_then(serde_json::Value::as_object)
        {
            for field in ["error_levels", "error_patterns"] {
                if custom.get(field).is_some_and(|value| {
                    !value.is_null()
                        && value.as_array().is_none_or(|values| !values.is_empty())
                        && value.as_object().is_none_or(|values| !values.is_empty())
                }) && !user.contains_key(field)
                {
                    base.remove(field);
                }
            }
        }
        if name == "custom"
            && user
                .get("pattern_file")
                .is_some_and(|value| !value.is_null())
        {
            for field in ["template", "fields"] {
                if !user.contains_key(field) {
                    base.remove(field);
                }
            }
        }
    }
}

fn parse_document(
    source: &str,
    path: Option<&Path>,
    original_defaults: bool,
) -> Result<Document, Box<dyn Error>> {
    if let Some(path) = path {
        let valid = path.extension().is_some_and(|ext| {
            ext.eq_ignore_ascii_case("json")
                || ext.eq_ignore_ascii_case("yaml")
                || ext.eq_ignore_ascii_case("yml")
        });
        if !valid {
            return Err("configuration path must end in .json, .yaml, or .yml".into());
        }
    }
    let mut document: Document = if original_defaults {
        let mut defaults: serde_json::Value =
            serde_json::from_str(include_str!("../data/original_default_config.json"))?;
        let overrides = if path
            .and_then(Path::extension)
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            serde_json::from_str(source)?
        } else {
            let overrides: serde_yaml::Value = serde_yaml::from_str(source)?;
            serde_json::to_value(overrides)?
        };
        if !overrides.is_null() {
            remove_inherited_aliases(&mut defaults, &overrides);
            merge_defaults(&mut defaults, overrides);
        }
        serde_json::from_value(defaults)?
    } else if path
        .and_then(Path::extension)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        serde_json::from_str(source)?
    } else {
        serde_yaml::from_str(source)?
    };
    if let Some(config_path) = path {
        let base = config_path.parent().unwrap_or_else(|| Path::new("."));
        for settings in document.log_generator.log_types.values_mut() {
            if let Some(pattern_path) = &mut settings.pattern_file
                && pattern_path.is_relative()
            {
                *pattern_path = base.join(&*pattern_path);
            }
        }
    }
    if let Some(network) = document.log_generator.global.network.take() {
        if network.endpoint.is_some() && network.protocol != "http" {
            return Err("network endpoint requires HTTP".into());
        }
        if (network.max_retries.is_some() || network.retry_delay.is_some())
            && network.protocol != "tcp"
        {
            return Err("TCP retry options require TCP protocol".into());
        }
        if let Some(delay) = network.retry_delay {
            parse_retry_delay(delay)?;
        }
        let global = &mut document.log_generator.global;
        if let Some(retries) = network.max_retries {
            global.network_max_retries = retries;
        }
        if let Some(delay) = network.retry_delay {
            global.network_retry_delay = delay;
        }
        global.network_host = network.host;
        global.network_port = network.port;
        global.network_protocol = network.protocol;
        if let Some(timeout) = network.timeout {
            global.network_timeout = timeout;
        }
        if let Some(endpoint) = network.endpoint {
            global.network_endpoint = endpoint;
        }
    }
    if document.performance.is_some() && document.log_generator.performance.is_some() {
        return Err("performance must appear in one location".into());
    }
    if let Some(performance) = document
        .performance
        .as_ref()
        .or(document.log_generator.performance.as_ref())
    {
        if let Some(limit) = &performance.memory_limit
            && parse_size(limit)? == 0
        {
            return Err("memory_limit must be greater than zero".into());
        }
        if performance.thread_pool_size == Some(0) || performance.io_buffer_size == Some(0) {
            return Err("thread_pool_size and io_buffer_size must be greater than zero".into());
        }
        if performance
            .gc_threshold
            .is_some_and(|threshold| !threshold.is_finite() || threshold <= 0.0)
        {
            return Err("gc_threshold must be positive and finite".into());
        }
        if performance.gc_threshold.is_some() {
            eprintln!(
                "warning: gc_threshold is not applied because Rust has no cyclic garbage collector"
            );
        }
        if performance.threads == Some(0) {
            return Err("threads must be greater than zero".into());
        }
        let global = &mut document.log_generator.global;
        if global.buffer_size.is_none() {
            global.buffer_size = performance.io_buffer_size.or(performance.buffer_size);
        }
        if global.batch_size.is_none() {
            global.batch_size = performance.batch_size;
        }
        if global.threads.is_none() {
            global.threads = performance.threads;
        }
        if global.async_io.is_none() {
            global.async_io = performance.async_io;
        }
        if global.thread_pool_size.is_none() {
            global.thread_pool_size = performance.thread_pool_size;
        }
        if global.lazy_loading.is_none() {
            global.lazy_loading = performance.lazy_loading;
        }
        if global.memory_limit.is_none() {
            global.memory_limit = performance.memory_limit.clone();
        }
    }
    if let Some(logging) = &document.log_generator.logging {
        logging.validate()?;
    }
    if let Some(rotation) = document.log_rotation.take() {
        if document.log_generator.global.log_rotation.is_some() {
            return Err("log_rotation must appear in one location".into());
        }
        document.log_generator.global.log_rotation = Some(rotation);
    }
    Ok(document)
}

fn validate_service_metrics(config: &Config) -> Result<(), Box<dyn Error>> {
    if let Some(monitoring) = &config.monitoring {
        for requested in &monitoring.service_metrics {
            let found = config.log_types.iter().any(|(name, settings)| {
                settings.enabled
                    && settings.is_microservice(name)
                    && settings.service_name.as_deref().unwrap_or(name) == requested.service
                    && (!requested.metrics.iter().any(|metric| {
                        ["request_count", "response_time", "error_rate"].contains(&metric.as_str())
                    }) || !settings.custom_fields.upstream_services.is_empty())
            });
            if !found {
                return Err(format!(
                    "service metrics require an enabled matching service or gateway: {}",
                    requested.service
                )
                .into());
            }
        }
    }
    Ok(())
}

pub(crate) fn check_config_source(
    source: &str,
    path: &Path,
    registry: &GeneratorRegistry,
) -> Result<(), Box<dyn Error>> {
    let document = parse_document(source, Some(path), false)?;
    validate_config(&document.log_generator, registry)
}

pub(crate) fn log_type_names() -> Vec<String> {
    LogType::value_variants()
        .iter()
        .filter_map(|kind| kind.to_possible_value())
        .map(|value| value.get_name().to_owned())
        .collect()
}

fn validate_config(config: &Config, registry: &GeneratorRegistry) -> Result<(), Box<dyn Error>> {
    validate_service_metrics(config)?;
    let global = &config.global;
    validate_json_mappings(global, config.output.as_ref())?;
    if let Some(monitoring) = &config.monitoring {
        monitoring.validate()?;
    }
    let patterns = library_patterns(&config.error_patterns)?;
    if config
        .log_types
        .get("error_pattern")
        .is_some_and(|settings| settings.enabled)
        && patterns.is_empty()
    {
        return Err("error_pattern requires a nonempty error_patterns library".into());
    }
    resolve_rotation(global, None)?;
    if let Some(limit) = &global.memory_limit
        && parse_size(limit)? == 0
    {
        return Err("memory_limit must be greater than zero".into());
    }
    if let Some(output) = &config.output {
        validate_handlers(output)?;
        validate_handler_network(output, global)?;
        if output
            .handlers
            .iter()
            .any(|handler| handler.kind == "network")
        {
            network_timeout(global.network_timeout)?;
        }
    }
    if !global.generation_interval.is_finite() || global.generation_interval < 0.0 {
        return Err("generation_interval must be a finite nonnegative number".into());
    }
    if global.buffer_size == Some(0) || global.batch_size == Some(0) {
        return Err("buffer_size and batch_size must be greater than zero".into());
    }
    if global.thread_pool_size == Some(0) {
        return Err("thread_pool_size must be greater than zero".into());
    }
    if global.threads == Some(0) {
        return Err("threads must be greater than zero".into());
    }
    if !["file", "multi_file", "console", "json", "network"]
        .contains(&global.output_format.as_str())
    {
        return Err("unknown output format".into());
    }
    if global.output_format == "network"
        && !["tcp", "udp", "http"].contains(&global.network_protocol.as_str())
    {
        return Err("network protocol must be tcp, udp, or http".into());
    }
    if global.output_format == "network" {
        parse_retry_delay(global.network_retry_delay)?;
        if global.network_host.is_empty() || global.network_port == 0 {
            return Err("network host and port are required".into());
        }
        network_timeout(global.network_timeout)?;
        if global.network_protocol == "http" {
            crate::http_output::validate_target(&global.network_host, &global.network_endpoint)?;
        }
    }
    if let Some(time) = &global.start_time {
        parse_time(time)?;
    }
    if chrono::format::StrftimeItems::new(&global.multi_file.date_format)
        .any(|item| matches!(item, chrono::format::Item::Error))
    {
        return Err("invalid multi-file date_format".into());
    }
    let mut weights = Vec::new();
    let mut service_names = Vec::new();
    for (name, settings) in &config.log_types {
        if settings.enabled {
            if let Some(kind) = resolve_kind(name, Some(settings), registry)? {
                if matches!(kind, LogType::Process) {
                    settings.validate_process()?;
                    weights.push(settings.frequency);
                    continue;
                }
                settings.nginx_fields(name)?;
                let generator = settings.settings(name)?;
                if matches!(kind, LogType::Microservice) {
                    service_names.push((
                        weights.len(),
                        generator.service_name.ok_or("missing service name")?,
                    ));
                }
                if name == "error_library" {
                    crate::error_library::ErrorLibrary::from_sources(
                        settings.patterns_dir.as_deref(),
                        settings.error_category.as_deref(),
                        settings.error_severity.as_deref(),
                        0,
                    )
                    .map_err(|error| -> Box<dyn Error> { error })?;
                }
                if name == "scenario" {
                    crate::scenario::ScenarioGenerator::from_sources(
                        settings
                            .scenario_file
                            .as_ref()
                            .ok_or("scenario_file is required")?,
                        settings.patterns_dir.as_deref(),
                        0,
                    )
                    .map_err(|e| -> Box<dyn Error> { e })?;
                }
            } else {
                settings.validate_external()?;
                registry
                    .create(
                        name,
                        settings
                            .plugin_config
                            .as_ref()
                            .unwrap_or(&serde_yaml::Value::Null),
                    )
                    .map_err(|e| -> Box<dyn Error> { e })?;
            }
            weights.push(settings.frequency);
        }
    }
    if !config.log_types.is_empty() {
        crate::weights::checked(weights)?;
    }
    if let Some(tracing) = &config.distributed_tracing
        && tracing.enabled
    {
        if service_names.is_empty() {
            return Err("distributed tracing requires service logs".into());
        }
        crate::tracing::TracePlanner::new(tracing, &service_names)?;
    }
    Ok(())
}

fn original_command(program: &str) -> bool {
    Path::new(program)
        .file_stem()
        .is_some_and(|name| name == "log-generator")
}

fn config_command(args: Vec<String>, registry: &GeneratorRegistry) -> Result<(), Box<dyn Error>> {
    let use_original = original_command(&args[0]);
    let cli = ConfigCommand::try_parse_from(
        std::iter::once(crate::app::command_name(&args[0], "config"))
            .chain(args.into_iter().skip(2)),
    )?;
    if cli.standalone_config && (cli.file.is_none() || cli.action == "create") {
        return Err("--standalone-config requires an existing configuration file".into());
    }
    if cli.action == "edit" {
        let path = cli.file.ok_or("--file is required for edit")?;
        if !path.is_file() {
            return Err("configuration file not found".into());
        }
        let editor = std::env::var_os("EDITOR").unwrap_or_else(|| "nano".into());
        let status = std::process::Command::new(editor).arg(path).status()?;
        if !status.success() {
            return Err(format!("editor exited with {status}").into());
        }
        return Ok(());
    }
    let source = if cli.action == "create" || cli.file.is_none() {
        "{}".to_string()
    } else {
        fs::read_to_string(cli.file.as_ref().ok_or("--file is required")?)?
    };
    if cli.action == "validate" && cli.file.is_none() {
        return Err("--file is required for validate".into());
    }
    let input_path = if cli.action == "create" {
        None
    } else {
        cli.file.as_deref()
    };
    let document = parse_document(
        &source,
        input_path,
        !cli.standalone_config
            && (use_original
                || cli.original_defaults
                || cli.action == "create"
                || cli.file.is_none()),
    )?;
    validate_config(&document.log_generator, registry)?;
    if cli.action == "validate" {
        println!(
            "Configuration is valid: {}",
            cli.file.ok_or("--file is required")?.display()
        );
    } else {
        let requested_format = cli.format.as_deref().unwrap_or("yaml");
        let output_path = if cli.action == "create" {
            Some(cli.file.clone().unwrap_or_else(|| {
                PathBuf::from(format!("log_generator_config.{requested_format}"))
            }))
        } else {
            None
        };
        let format = if let Some(path) = &output_path {
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            let file_format = if extension.eq_ignore_ascii_case("json") {
                "json"
            } else if extension.eq_ignore_ascii_case("yaml")
                || extension.eq_ignore_ascii_case("yml")
            {
                "yaml"
            } else {
                return Err("configuration path must end in .json, .yaml, or .yml".into());
            };
            if cli
                .format
                .as_deref()
                .is_some_and(|format| format != file_format)
            {
                return Err("--format conflicts with configuration file extension".into());
            }
            file_format
        } else {
            requested_format
        };
        let formatted = if format == "json" {
            serde_json::to_string_pretty(&document)?
        } else {
            serde_yaml::to_string(&document)?
        };
        if cli.action == "create" {
            let path = output_path.ok_or("missing output path")?;
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            writeln!(file, "{formatted}")?;
            println!("Configuration created: {}", path.display());
        } else {
            println!("{formatted}");
        }
    }
    Ok(())
}

enum Output {
    File(RotatingFile),
    Console,
    HumanConsole {
        human: bool,
        colored: bool,
    },
    Syslog(Box<Output>),
    Multi {
        base: PathBuf,
        pattern: String,
        date_format: String,
        files: HashMap<PathBuf, (RotatingFile, u64)>,
        closed_opened: HashMap<PathBuf, std::time::Instant>,
        type_paths: Option<HashMap<String, PathBuf>>,
        use_tick: u64,
        rotation: Rotation,
    },
    Tcp(crate::network_output::TcpOutput),
    Udp(UdpSocket),
    Http(crate::http_output::HttpOutput),
    Prometheus(crate::prometheus::PrometheusOutput),
    Json {
        output: Box<Output>,
        options: crate::json_output::Options,
    },
    Fanout(Vec<Output>),
}

pub(crate) struct OutputHandle(Output);

impl OutputHandle {
    pub(crate) fn network_info(&self) -> io::Result<Option<crate::output::NetworkInfo>> {
        self.0.network_info()
    }
    pub(crate) fn field_mapping(&self) -> io::Result<BTreeMap<String, String>> {
        let Output::Json { options, .. } = &self.0 else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "field mappings require JSON output",
            ));
        };
        let mut mapping: BTreeMap<String, String> = [
            ("timestamp", "@timestamp"),
            ("log_type", "type"),
            ("level", "level"),
            ("message", "message"),
            ("source_ip", "source.ip"),
            ("user_agent", "user_agent"),
            ("status_code", "http.response.status_code"),
            ("response_time", "http.response.time"),
            ("trace_id", "trace_id"),
            ("span_id", "span_id"),
            ("parent_span_id", "parent_span_id"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        mapping.extend(options.mapping.clone());
        Ok(mapping)
    }

    pub(crate) fn set_field_mapping(
        &mut self,
        mapping: BTreeMap<String, String>,
    ) -> io::Result<()> {
        let mut effective = self.field_mapping()?;
        effective.extend(mapping.clone());
        let fields = [
            "timestamp",
            "log_type",
            "level",
            "message",
            "source_ip",
            "user_agent",
            "status_code",
            "response_time",
            "trace_id",
            "span_id",
            "parent_span_id",
        ];
        if let Some(name) = effective
            .keys()
            .find(|name| !fields.contains(&name.as_str()))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown JSON mapping field: {name}"),
            ));
        }
        let mut paths: Vec<_> = fields
            .iter()
            .filter_map(|key| effective.get(*key).map(String::as_str))
            .collect();
        paths.push("@version");
        if let Output::Json { options, .. } = &self.0
            && options.include_raw
        {
            paths.push("raw_message");
        }
        for (index, path) in paths.iter().enumerate() {
            if path.split('.').any(str::is_empty) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid JSON field path",
                ));
            }
            if paths[index + 1..].iter().any(|other| {
                *path == *other
                    || path
                        .strip_prefix(*other)
                        .is_some_and(|tail| tail.starts_with('.'))
                    || other
                        .strip_prefix(*path)
                        .is_some_and(|tail| tail.starts_with('.'))
            }) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "conflicting JSON field mappings",
                ));
            }
        }
        let Output::Json { options, .. } = &mut self.0 else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "field mappings require JSON output",
            ));
        };
        options.mapping.extend(mapping);
        Ok(())
    }

    fn rotation(options: crate::output::RotationOptions) -> io::Result<Rotation> {
        if options.size_bytes == Some(0)
            || options.interval_seconds == Some(0)
            || options.max_files == 0
            || options.buffer_size == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "rotation options must be positive",
            ));
        }
        Ok(Rotation {
            size: options.size_bytes,
            seconds: options.interval_seconds,
            max_files: options.max_files,
            backup_count: options.max_files,
            buffer_size: options.buffer_size,
            compress: options.compress,
        })
    }

    pub(crate) fn file(path: &Path, options: crate::output::RotationOptions) -> io::Result<Self> {
        Ok(Self(Output::File(RotatingFile::open(
            path,
            Self::rotation(options)?,
        )?)))
    }

    pub(crate) fn multi_file(
        base: &Path,
        pattern: &str,
        date_format: &str,
        options: crate::output::RotationOptions,
    ) -> io::Result<Self> {
        if chrono::format::StrftimeItems::new(date_format)
            .any(|item| matches!(item, chrono::format::Item::Error))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid multi-file date_format",
            ));
        }
        Ok(Self(Output::Multi {
            base: base.to_owned(),
            pattern: pattern.to_owned(),
            date_format: date_format.to_owned(),
            files: HashMap::new(),
            closed_opened: HashMap::new(),
            type_paths: Some(HashMap::new()),
            use_tick: 0,
            rotation: Self::rotation(options)?,
        }))
    }

    pub(crate) fn console() -> Self {
        Self(Output::Console)
    }

    pub(crate) fn network(
        host: &str,
        port: u16,
        protocol: &str,
        timeout: std::time::Duration,
        max_retries: usize,
        retry_delay: std::time::Duration,
    ) -> Result<Self, Box<dyn Error>> {
        if host.is_empty() || port == 0 || timeout.is_zero() {
            return Err("network host, port, and timeout are required".into());
        }
        Ok(Self(open_network(
            host,
            port,
            protocol,
            timeout,
            max_retries,
            retry_delay,
        )?))
    }

    pub(crate) fn http(
        host: &str,
        port: u16,
        endpoint: &str,
        timeout: std::time::Duration,
    ) -> io::Result<Self> {
        if port == 0 || timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "HTTP port and timeout are required",
            ));
        }
        Ok(Self(Output::Http(crate::http_output::HttpOutput::new(
            host, port, endpoint, timeout, false,
        )?)))
    }

    pub(crate) fn json(
        mut self,
        mapping: BTreeMap<String, String>,
        include_raw: bool,
        pretty: bool,
        format: crate::json_output::JsonFormat,
    ) -> Self {
        self.0.set_json_content_type();
        Self(Output::Json {
            output: Box::new(self.0),
            options: crate::json_output::options(format, mapping, include_raw, pretty),
        })
    }

    pub(crate) fn fanout(outputs: Vec<Self>) -> Self {
        Self(Output::Fanout(
            outputs.into_iter().map(|output| output.0).collect(),
        ))
    }

    pub(crate) fn write_with_level(
        &mut self,
        kind: &str,
        timestamp: DateTime<Utc>,
        data: &[u8],
        level: Option<&str>,
    ) -> Result<(), Box<dyn Error>> {
        self.0.write_with_level(kind, timestamp, data, level)
    }

    pub(crate) fn write_entry(
        &mut self,
        entry: &crate::models::LogEntry,
    ) -> Result<(), Box<dyn Error>> {
        self.0.write_entry(entry)
    }

    pub(crate) fn flush(&mut self) -> io::Result<()> {
        self.0.finish()
    }

    pub(crate) fn close(&mut self) -> io::Result<()> {
        self.0.close()
    }

    pub(crate) fn reconnect_count(&self) -> u64 {
        self.0.reconnect_count()
    }

    pub(crate) fn file_info(&self) -> Option<crate::output::FileInfo> {
        self.0.file_info()
    }

    pub(crate) fn force_rotation(&mut self) -> io::Result<()> {
        self.0.force_rotation()
    }

    pub(crate) fn file_info_for_type(
        &self,
        kind: &str,
    ) -> io::Result<Option<crate::output::FileInfo>> {
        self.0.file_info_for_type(kind)
    }

    pub(crate) fn force_rotation_for_type(&mut self, kind: &str) -> io::Result<()> {
        self.0.force_rotation_for_type(kind)
    }

    pub(crate) fn active_file_types(&self) -> Vec<String> {
        self.0.active_file_types()
    }

    pub(crate) fn is_multi_file(&self) -> bool {
        self.0.is_multi_file()
    }
}
#[derive(Clone, Copy)]
struct Rotation {
    size: Option<u64>,
    seconds: Option<u64>,
    max_files: usize,
    backup_count: usize,
    buffer_size: usize,
    compress: bool,
}

fn resolve_rotation(
    global: &Global,
    cli_max_files: Option<usize>,
) -> Result<Rotation, Box<dyn Error>> {
    let mut size = global.rotation_size;
    let mut seconds = global.rotation_time;
    let mut max_files = global.max_files;
    let mut compress = false;
    let mut backup_count = None;
    if let Some(config) = &global.log_rotation
        && config.enabled
    {
        compress = config.compress;
        if size.is_none() {
            size = config.max_size.as_ref().map(parse_size).transpose()?;
        }
        if seconds.is_none() {
            seconds = config
                .rotation_time
                .as_ref()
                .map(parse_rotation_time)
                .transpose()?;
        }
        if let Some(count) = config.max_files {
            max_files = count;
        }
        backup_count = config.backup_count;
    }
    if let Some(count) = cli_max_files {
        max_files = count;
        backup_count = Some(count);
    }
    Ok(Rotation {
        size,
        seconds,
        max_files,
        backup_count: backup_count.unwrap_or(max_files),
        buffer_size: global.buffer_size.unwrap_or(8192),
        compress,
    })
}
struct RotatingFile {
    path: PathBuf,
    writer: BufWriter<fs::File>,
    bytes: u64,
    opened: std::time::Instant,
    rotation: Rotation,
}
impl RotatingFile {
    fn open(path: &Path, rotation: Rotation) -> io::Result<Self> {
        let writer = open_file(path, rotation.buffer_size)?;
        let bytes = writer.get_ref().metadata()?.len();
        Ok(Self {
            path: path.to_owned(),
            writer,
            bytes,
            opened: std::time::Instant::now(),
            rotation,
        })
    }
    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let size_due = self
            .rotation
            .size
            .is_some_and(|size| size > 0 && self.bytes >= size);
        let time_due = self
            .rotation
            .seconds
            .is_some_and(|seconds| seconds > 0 && self.opened.elapsed().as_secs() >= seconds);
        if size_due || time_due {
            let max_files = match (size_due, time_due) {
                (true, true) => self.rotation.max_files.min(self.rotation.backup_count),
                (true, false) => self.rotation.max_files,
                (false, true) => self.rotation.backup_count,
                (false, false) => unreachable!(),
            };
            self.rotate(max_files)?;
        }
        self.writer.write_all(data)?;
        self.bytes = self
            .bytes
            .checked_add(data.len() as u64)
            .ok_or_else(|| io::Error::other("file size overflow"))?;
        Ok(())
    }

    fn rotate(&mut self, max_files: usize) -> io::Result<()> {
        self.writer.flush()?;
        if !self.writer.get_ref().metadata()?.file_type().is_file()
            || !fs::symlink_metadata(&self.path)?.file_type().is_file()
        {
            return Err(io::Error::other(
                "rotation requires a regular file without a symlink",
            ));
        }
        let target = (1..=max_files)
            .map(|index| {
                let mut path = self.path.as_os_str().to_os_string();
                path.push(format!(".{index}"));
                let plain = PathBuf::from(path.clone());
                path.push(".gz");
                (plain, PathBuf::from(path))
            })
            .find(|(plain, compressed)| !plain.exists() && !compressed.exists())
            .ok_or_else(|| {
                io::Error::other("rotation limit reached; preserve or move existing rotated files")
            })?;
        if self.rotation.compress {
            let parent = self
                .path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
            let mut source = fs::File::open(&self.path)?;
            let mut encoder = flate2::write::GzEncoder::new(
                temporary.as_file_mut(),
                flate2::Compression::default(),
            );
            io::copy(&mut source, &mut encoder)?;
            encoder.finish()?;
            temporary.as_file().sync_all()?;
            temporary
                .persist_noclobber(&target.1)
                .map_err(|error| error.error)?;
        } else {
            fs::hard_link(&self.path, &target.0)?;
        }
        fs::remove_file(&self.path)?;
        self.writer = open_file(&self.path, self.rotation.buffer_size)?;
        self.bytes = 0;
        self.opened = std::time::Instant::now();
        Ok(())
    }
}
fn open_file(path: &Path, capacity: usize) -> io::Result<BufWriter<fs::File>> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    Ok(BufWriter::with_capacity(
        capacity,
        OpenOptions::new().create(true).append(true).open(path)?,
    ))
}

fn open_network(
    host: &str,
    port: u16,
    protocol: &str,
    timeout: std::time::Duration,
    max_retries: usize,
    retry_delay: std::time::Duration,
) -> Result<Output, Box<dyn Error>> {
    let address = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or("network host did not resolve")?;
    match protocol {
        "tcp" => Ok(Output::Tcp(crate::network_output::TcpOutput::connect(
            address,
            timeout,
            max_retries,
            retry_delay,
        )?)),
        "udp" => {
            let socket = UdpSocket::bind(if address.is_ipv4() {
                "0.0.0.0:0"
            } else {
                "[::]:0"
            })?;
            socket.connect(address)?;
            socket.set_write_timeout(Some(timeout))?;
            Ok(Output::Udp(socket))
        }
        _ => Err("network protocol must be tcp or udp".into()),
    }
}

fn json_options(global: &Global) -> crate::json_output::Options {
    crate::json_output::options(
        crate::json_output::JsonFormat::Standard,
        global.field_mapping.clone(),
        global.include_raw_message,
        global.pretty_print,
    )
}

fn validate_json_mappings(global: &Global, outputs: Option<&OutputConfig>) -> io::Result<()> {
    let validate = |options| {
        let mut output = OutputHandle(Output::Json {
            output: Box::new(Output::Console),
            options,
        });
        output.set_field_mapping(BTreeMap::new())
    };
    if let Some(outputs) = outputs {
        for handler in &outputs.handlers {
            if let Some(format) = handler_json_format(handler.format.as_deref()) {
                let include_raw = handler.include_raw_message.unwrap_or(
                    !matches!(format, crate::json_output::JsonFormat::Splunk)
                        && global.include_raw_message,
                );
                validate(crate::json_output::options(
                    format,
                    global.field_mapping.clone(),
                    include_raw,
                    global.pretty_print,
                ))?;
            }
        }
    } else if global.output_format == "json" {
        validate(json_options(global))?;
    }
    Ok(())
}

fn validate_handler_network(config: &OutputConfig, global: &Global) -> Result<(), Box<dyn Error>> {
    for handler in &config.handlers {
        if handler.kind == "network" && handler.protocol.as_deref() != Some("http") {
            parse_retry_delay(handler.retry_delay.unwrap_or(global.network_retry_delay))?;
        }
    }
    Ok(())
}

fn open_handlers(
    config: &OutputConfig,
    global: &Global,
    rotation: Rotation,
) -> Result<Output, Box<dyn Error>> {
    validate_handlers(config)?;
    validate_handler_network(config, global)?;
    let timeout = if config
        .handlers
        .iter()
        .any(|handler| handler.kind == "network")
    {
        Some(network_timeout(global.network_timeout)?)
    } else {
        None
    };
    let mut outputs = Vec::with_capacity(config.handlers.len());
    for handler in &config.handlers {
        let output = match handler.kind.as_str() {
            "console"
                if handler.colored.unwrap_or(false)
                    || handler.format.as_deref() == Some("human_readable") =>
            {
                Output::HumanConsole {
                    human: handler.format.as_deref() == Some("human_readable"),
                    colored: handler.colored.unwrap_or(false)
                        && io::stdout().is_terminal()
                        && std::env::var_os("NO_COLOR").is_none(),
                }
            }
            "console" => Output::Console,
            "file" => Output::File(RotatingFile::open(
                handler
                    .file_path
                    .as_deref()
                    .ok_or("handler file path required")?,
                rotation,
            )?),
            "network" if handler.protocol.as_deref() == Some("http") => {
                let http = crate::http_output::HttpOutput::new(
                    handler.host.as_deref().ok_or("handler host required")?,
                    handler.port.ok_or("handler port required")?,
                    handler
                        .endpoint
                        .as_deref()
                        .ok_or("HTTP endpoint required")?,
                    timeout.ok_or("network timeout missing")?,
                    handler_json_format(handler.format.as_deref()).is_some(),
                )?;
                if handler.format.as_deref() == Some("prometheus") {
                    Output::Prometheus(crate::prometheus::PrometheusOutput::new(http))
                } else {
                    Output::Http(http)
                }
            }
            "network" => open_network(
                handler.host.as_deref().ok_or("handler host required")?,
                handler.port.ok_or("handler port required")?,
                handler.protocol.as_deref().unwrap_or("tcp"),
                timeout.ok_or("network timeout missing")?,
                handler.max_retries.unwrap_or(global.network_max_retries),
                parse_retry_delay(handler.retry_delay.unwrap_or(global.network_retry_delay))?,
            )?,
            _ => return Err("unsupported output handler".into()),
        };
        let output = if handler.format.as_deref() == Some("syslog") {
            Output::Syslog(Box::new(output))
        } else {
            output
        };
        outputs.push(
            if let Some(format) = handler_json_format(handler.format.as_deref()) {
                Output::Json {
                    output: Box::new(output),
                    options: crate::json_output::options(
                        format,
                        global.field_mapping.clone(),
                        handler.include_raw_message.unwrap_or({
                            if matches!(format, crate::json_output::JsonFormat::Splunk) {
                                false
                            } else {
                                global.include_raw_message
                            }
                        }),
                        global.pretty_print,
                    ),
                }
            } else {
                output
            },
        );
    }
    Ok(Output::Fanout(outputs))
}
impl Output {
    fn close_continuing(&mut self) -> Vec<String> {
        if let Self::Fanout(outputs) = self {
            return outputs
                .iter_mut()
                .enumerate()
                .flat_map(|(index, output)| {
                    output
                        .close_continuing()
                        .into_iter()
                        .map(move |error| format!("output handler {index} failed: {error}"))
                })
                .collect();
        }
        self.close()
            .err()
            .map(|error| vec![error.to_string()])
            .unwrap_or_default()
    }
    fn flush_continuing(&mut self) -> Vec<String> {
        if let Self::Fanout(outputs) = self {
            return outputs
                .iter_mut()
                .enumerate()
                .flat_map(|(index, output)| {
                    output
                        .flush_continuing()
                        .into_iter()
                        .map(move |error| format!("output handler {index} failed: {error}"))
                })
                .collect();
        }
        self.finish()
            .err()
            .map(|error| vec![error.to_string()])
            .unwrap_or_default()
    }
    fn write_continuing(
        &mut self,
        kind: &str,
        time: DateTime<Utc>,
        data: &[u8],
        level: Option<&str>,
        entry: Option<&crate::models::LogEntry>,
    ) -> Vec<String> {
        if let Self::Fanout(outputs) = self {
            return outputs
                .iter_mut()
                .enumerate()
                .flat_map(|(index, output)| {
                    output
                        .write_continuing(kind, time, data, level, entry)
                        .into_iter()
                        .map(move |error| format!("output handler {index} failed: {error}"))
                })
                .collect();
        }
        let result = if let Some(entry) = entry {
            self.write_entry(entry)
        } else {
            self.write_with_level(kind, time, data, level)
        };
        result
            .err()
            .map(|error| vec![error.to_string()])
            .unwrap_or_default()
    }
    fn network_info(&self) -> io::Result<Option<crate::output::NetworkInfo>> {
        let info = match self {
            Self::Tcp(output) => Some(("tcp", output.target().to_string())),
            Self::Udp(socket) => Some(("udp", socket.peer_addr()?.to_string())),
            Self::Http(output) => Some(("http", output.target())),
            Self::Json { output, .. } | Self::Syslog(output) => return output.network_info(),
            _ => None,
        };
        Ok(info.map(|(protocol, target)| crate::output::NetworkInfo {
            protocol: protocol.to_owned(),
            target,
        }))
    }
    fn is_multi_file(&self) -> bool {
        match self {
            Self::Multi {
                type_paths: Some(_),
                ..
            } => true,
            Self::Json { output, .. } => output.is_multi_file(),
            _ => false,
        }
    }

    fn file_info_for_type(&self, kind: &str) -> io::Result<Option<crate::output::FileInfo>> {
        match self {
            Self::Multi {
                files,
                type_paths: Some(type_paths),
                ..
            } => {
                let Some(path) = type_paths.get(kind) else {
                    return Ok(None);
                };
                let size_bytes = if let Some((file, _)) = files.get(path) {
                    file.bytes
                } else {
                    fs::metadata(path)?.len()
                };
                Ok(Some(crate::output::FileInfo {
                    path: path.clone(),
                    size_bytes,
                }))
            }
            Self::Json { output, .. } => output.file_info_for_type(kind),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "file type information requires multi-file output",
            )),
        }
    }

    fn active_file_types(&self) -> Vec<String> {
        match self {
            Self::Multi {
                type_paths: Some(paths),
                ..
            } => {
                let mut kinds: Vec<_> = paths.keys().cloned().collect();
                kinds.sort();
                kinds
            }
            Self::Json { output, .. } => output.active_file_types(),
            _ => Vec::new(),
        }
    }

    fn rotate_multi_path(
        path: &Path,
        files: &mut HashMap<PathBuf, (RotatingFile, u64)>,
        closed_opened: &mut HashMap<PathBuf, std::time::Instant>,
        rotation: Rotation,
    ) -> io::Result<()> {
        if let Some((file, _)) = files.get_mut(path) {
            return file.rotate(rotation.max_files);
        }
        fs::metadata(path)?;
        let mut file = RotatingFile::open(path, rotation)?;
        file.rotate(rotation.max_files)?;
        if rotation.seconds.is_some() {
            closed_opened.insert(path.to_owned(), file.opened);
        }
        Ok(())
    }

    fn force_rotation_for_type(&mut self, kind: &str) -> io::Result<()> {
        match self {
            Self::Multi {
                files,
                closed_opened,
                type_paths: Some(paths),
                rotation,
                ..
            } => {
                let path = paths.get(kind).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "no file for log type")
                })?;
                Self::rotate_multi_path(path, files, closed_opened, *rotation)
            }
            Self::Json { output, .. } => output.force_rotation_for_type(kind),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "type rotation requires multi-file output",
            )),
        }
    }

    fn file_info(&self) -> Option<crate::output::FileInfo> {
        match self {
            Self::File(file) => Some(crate::output::FileInfo {
                path: file.path.clone(),
                size_bytes: file.bytes,
            }),
            Self::Json { output, .. } => output.file_info(),
            _ => None,
        }
    }

    fn force_rotation(&mut self) -> io::Result<()> {
        match self {
            Self::File(file) => file.rotate(file.rotation.max_files),
            Self::Multi {
                files,
                closed_opened,
                type_paths: Some(paths),
                rotation,
                ..
            } => {
                let paths: std::collections::BTreeSet<_> = paths.values().cloned().collect();
                for path in paths {
                    Self::rotate_multi_path(&path, files, closed_opened, *rotation)?;
                }
                Ok(())
            }
            Self::Json { output, .. } => output.force_rotation(),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "force rotation requires file output",
            )),
        }
    }

    fn reconnect_count(&self) -> u64 {
        match self {
            Self::Tcp(output) => output.reconnect_count(),
            Self::Json { output, .. } | Self::Syslog(output) => output.reconnect_count(),
            Self::Fanout(outputs) => outputs.iter().map(Self::reconnect_count).sum(),
            _ => 0,
        }
    }

    fn set_json_content_type(&mut self) {
        match self {
            Self::Http(output) => output.set_json_content_type(),
            Self::Fanout(outputs) => outputs.iter_mut().for_each(Self::set_json_content_type),
            Self::Json { output, .. } | Self::Syslog(output) => output.set_json_content_type(),
            _ => (),
        }
    }

    fn write_entry(&mut self, entry: &crate::models::LogEntry) -> Result<(), Box<dyn Error>> {
        match self {
            Self::Json { output, options } => {
                let record = crate::json_output::format_entry(entry, options)?;
                output.write_with_level(&entry.log_type, entry.timestamp, &record, None)
            }
            Self::Fanout(outputs) => {
                for (index, output) in outputs.iter_mut().enumerate() {
                    output.write_entry(entry).map_err(|error| {
                        io::Error::other(format!("output handler {index} failed: {error}"))
                    })?;
                }
                Ok(())
            }
            _ => {
                let record = format!("{entry}\n");
                self.write_with_level(
                    &entry.log_type,
                    entry.timestamp,
                    record.as_bytes(),
                    Some(&entry.level),
                )
            }
        }
    }

    fn write_with_level(
        &mut self,
        kind: &str,
        time: DateTime<Utc>,
        data: &[u8],
        level: Option<&str>,
    ) -> Result<(), Box<dyn Error>> {
        match self {
            Self::File(file) => file.write(data)?,
            Self::Console => io::stdout()
                .lock()
                .write_all(data)
                .map_err(crate::stdout_error)?,
            Self::HumanConsole { human, colored } => {
                let message = std::str::from_utf8(data)?.trim_end_matches('\n');
                let prefix = if *human {
                    format!(
                        "[{}] [{}] ",
                        time.format("%Y-%m-%d %H:%M:%S"),
                        kind.to_ascii_uppercase()
                    )
                } else {
                    String::new()
                };
                let color = if *colored {
                    let upper = message.to_ascii_uppercase();
                    if upper.contains("ERROR")
                        || upper.contains("CRITICAL")
                        || upper.contains("FATAL")
                    {
                        "\x1b[31m"
                    } else if upper.contains("WARNING") || upper.contains("WARN") {
                        "\x1b[33m"
                    } else {
                        "\x1b[32m"
                    }
                } else {
                    ""
                };
                let reset = if *colored { "\x1b[0m" } else { "" };
                writeln!(io::stdout().lock(), "{prefix}{color}{message}{reset}")
                    .map_err(crate::stdout_error)?;
            }
            Self::Syslog(output) => {
                let message = std::str::from_utf8(data)?
                    .trim_end_matches('\n')
                    .replace('\n', "\\n")
                    .replace('\r', "\\r");
                let record = format!("<14>1 {} - {} - - - {message}\n", time.to_rfc3339(), kind);
                output.write_with_level(kind, time, record.as_bytes(), level)?;
            }
            Self::Tcp(stream) => stream.write_record(data)?,
            Self::Udp(socket) => {
                if socket.send(data)? != data.len() {
                    return Err("incomplete UDP datagram".into());
                }
            }
            Self::Http(output) => output.write(data)?,
            Self::Prometheus(output) => output.record(kind)?,
            Self::Json { output, options } => {
                let message = std::str::from_utf8(data)?.trim_end_matches('\n');
                let record = crate::json_output::format_record_with_level(
                    kind, time, message, options, level,
                )?;
                output.write_with_level(kind, time, &record, None)?;
            }
            Self::Fanout(outputs) => {
                for (index, output) in outputs.iter_mut().enumerate() {
                    output
                        .write_with_level(kind, time, data, level)
                        .map_err(|error| {
                            io::Error::other(format!("output handler {index} failed: {error}"))
                        })?;
                }
            }
            Self::Multi {
                base,
                pattern,
                date_format,
                files,
                closed_opened,
                type_paths,
                use_tick,
                rotation,
            } => {
                let name = pattern
                    .replace("{log_type}", kind)
                    .replace("{date}", &time.format(date_format).to_string())
                    .replace("{time}", &time.format("%H%M%S").to_string())
                    .replace(
                        "{datetime}",
                        &format!("{}_{}", time.format(date_format), time.format("%H%M%S")),
                    );
                let path = base.join(name);
                if !path.starts_with(&*base)
                    || path
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    return Err("invalid multi-file pattern".into());
                }
                *use_tick = use_tick
                    .checked_add(1)
                    .ok_or("file access counter overflow")?;
                if !files.contains_key(&path) {
                    if files.len() >= 32 {
                        let oldest = files
                            .iter()
                            .min_by_key(|(_, (_, tick))| *tick)
                            .map(|(path, _)| path.clone())
                            .ok_or("no file available for eviction")?;
                        let (mut previous, _) =
                            files.remove(&oldest).ok_or("missing file for eviction")?;
                        previous.writer.flush()?;
                        if rotation.seconds.is_some_and(|seconds| seconds > 0) {
                            closed_opened.insert(oldest, previous.opened);
                        }
                    }
                    let mut file = RotatingFile::open(&path, *rotation)?;
                    if let Some(opened) = closed_opened.remove(&path) {
                        file.opened = opened;
                    }
                    files.insert(path.clone(), (file, *use_tick));
                }
                let (file, tick) = files.get_mut(&path).ok_or("missing output file")?;
                *tick = *use_tick;
                file.write(data)?;
                if let Some(type_paths) = type_paths {
                    type_paths.insert(kind.to_owned(), path);
                }
            }
        }
        Ok(())
    }
    fn finish(&mut self) -> io::Result<()> {
        match self {
            Self::File(f) => f.writer.flush(),
            Self::Json { output, .. } => output.finish(),
            Self::Fanout(outputs) => {
                let mut first_error = None;
                for output in outputs {
                    if let Err(error) = output.finish()
                        && first_error.is_none()
                    {
                        first_error = Some(error);
                    }
                }
                match first_error {
                    Some(error) => Err(error),
                    None => Ok(()),
                }
            }
            Self::Multi { files, .. } => {
                for (f, _) in files.values_mut() {
                    f.writer.flush()?;
                }
                Ok(())
            }
            Self::Tcp(s) => s.flush(),
            Self::Console | Self::HumanConsole { .. } => {
                io::stdout().flush().map_err(crate::stdout_error)
            }
            Self::Syslog(output) => output.finish(),
            Self::Udp(_) => Ok(()),
            Self::Http(_) => Ok(()),
            Self::Prometheus(output) => output.flush(),
        }
    }
    fn close(&mut self) -> io::Result<()> {
        match self {
            Self::Prometheus(output) => output.close(),
            Self::Json { output, .. } | Self::Syslog(output) => output.close(),
            Self::Fanout(outputs) => {
                let mut first_error = None;
                for output in outputs {
                    if let Err(error) = output.close()
                        && first_error.is_none()
                    {
                        first_error = Some(error);
                    }
                }
                match first_error {
                    Some(error) => Err(error),
                    None => Ok(()),
                }
            }
            _ => self.finish(),
        }
    }
}

const ASYNC_QUEUE_CAPACITY: usize = 64;

enum OutputCommand {
    Write {
        kind: String,
        time: DateTime<Utc>,
        data: Vec<u8>,
        level: Option<String>,
    },
    WriteEntry(Box<crate::models::LogEntry>),
    Flush(std::sync::mpsc::Sender<Result<(), String>>),
    WriteContinuing {
        kind: String,
        time: DateTime<Utc>,
        data: Vec<u8>,
        level: Option<String>,
        entry: Option<Box<crate::models::LogEntry>>,
        reply: std::sync::mpsc::Sender<Vec<String>>,
    },
    FlushContinuing(std::sync::mpsc::Sender<Vec<String>>),
    CloseContinuing(std::sync::mpsc::Sender<Vec<String>>),
}

struct AsyncOutput {
    sender: Option<std::sync::mpsc::SyncSender<OutputCommand>>,
    worker: Option<std::thread::JoinHandle<io::Result<()>>>,
}

impl AsyncOutput {
    fn new(mut output: Output) -> io::Result<Self> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(ASYNC_QUEUE_CAPACITY);
        let worker = std::thread::Builder::new()
            .name("log-forge-output".into())
            .spawn(move || {
                for command in receiver {
                    match command {
                        OutputCommand::Write {
                            kind,
                            time,
                            data,
                            level,
                        } => output
                            .write_with_level(&kind, time, &data, level.as_deref())
                            .map_err(|error| io::Error::other(error.to_string()))?,
                        OutputCommand::WriteEntry(entry) => output
                            .write_entry(&entry)
                            .map_err(|error| io::Error::other(error.to_string()))?,
                        OutputCommand::WriteContinuing {
                            kind,
                            time,
                            data,
                            level,
                            entry,
                            reply,
                        } => {
                            let errors = output.write_continuing(
                                &kind,
                                time,
                                &data,
                                level.as_deref(),
                                entry.as_deref(),
                            );
                            let _ = reply.send(errors);
                        }
                        OutputCommand::FlushContinuing(reply) => {
                            let _ = reply.send(output.flush_continuing());
                        }
                        OutputCommand::CloseContinuing(reply) => {
                            let _ = reply.send(output.close_continuing());
                            return Ok(());
                        }
                        OutputCommand::Flush(reply) => match output.finish() {
                            Ok(()) => {
                                let _ = reply.send(Ok(()));
                            }
                            Err(error) => {
                                let _ = reply.send(Err(error.to_string()));
                                return Err(error);
                            }
                        },
                    }
                }
                output.close()
            })?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }
    fn send(&mut self, command: OutputCommand) -> io::Result<()> {
        if self
            .sender
            .as_ref()
            .ok_or_else(|| io::Error::other("output worker is closed"))?
            .send(command)
            .is_err()
        {
            return match self.finish() {
                Err(error) => Err(error),
                Ok(()) => Err(io::Error::other("output worker stopped")),
            };
        }
        Ok(())
    }
    fn write(
        &mut self,
        kind: &str,
        time: DateTime<Utc>,
        data: &[u8],
        level: Option<&str>,
    ) -> io::Result<()> {
        self.send(OutputCommand::Write {
            kind: kind.to_owned(),
            time,
            data: data.to_vec(),
            level: level.map(str::to_owned),
        })
    }
    fn continuing(
        &mut self,
        command: impl FnOnce(std::sync::mpsc::Sender<Vec<String>>) -> OutputCommand,
    ) -> io::Result<Vec<String>> {
        let (reply, response) = std::sync::mpsc::channel();
        self.send(command(reply))?;
        response
            .recv()
            .map_err(|_| io::Error::other("output worker stopped before reporting errors"))
    }
    fn flush(&mut self) -> io::Result<()> {
        let (reply, response) = std::sync::mpsc::channel();
        self.send(OutputCommand::Flush(reply))?;
        match response.recv() {
            Ok(result) => result.map_err(io::Error::other),
            Err(_) => match self.finish() {
                Err(error) => Err(error),
                Ok(()) => Err(io::Error::other("output worker stopped before flush")),
            },
        }
    }
    fn finish(&mut self) -> io::Result<()> {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| io::Error::other("output worker panicked"))??;
        }
        Ok(())
    }
}
impl Drop for AsyncOutput {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

enum OutputSink {
    Sync(Box<Output>),
    Async(AsyncOutput),
}
impl OutputSink {
    fn close_continuing(&mut self) -> io::Result<Vec<String>> {
        match self {
            Self::Sync(output) => Ok(output.close_continuing()),
            Self::Async(output) => {
                let errors = output.continuing(OutputCommand::CloseContinuing)?;
                output.finish()?;
                Ok(errors)
            }
        }
    }
    fn flush_continuing(&mut self) -> io::Result<Vec<String>> {
        match self {
            Self::Sync(output) => Ok(output.flush_continuing()),
            Self::Async(output) => output.continuing(OutputCommand::FlushContinuing),
        }
    }
    fn write_continuing(
        &mut self,
        kind: &str,
        time: DateTime<Utc>,
        data: &[u8],
        level: Option<&str>,
        entry: Option<&crate::models::LogEntry>,
    ) -> Result<Vec<String>, Box<dyn Error>> {
        match self {
            Self::Sync(output) => Ok(output.write_continuing(kind, time, data, level, entry)),
            Self::Async(output) => {
                Ok(output.continuing(|reply| OutputCommand::WriteContinuing {
                    kind: kind.to_owned(),
                    time,
                    data: data.to_vec(),
                    level: level.map(str::to_owned),
                    entry: entry.cloned().map(Box::new),
                    reply,
                })?)
            }
        }
    }
    fn write_entry(&mut self, entry: &crate::models::LogEntry) -> Result<(), Box<dyn Error>> {
        match self {
            Self::Sync(output) => output.write_entry(entry),
            Self::Async(output) => {
                Ok(output.send(OutputCommand::WriteEntry(Box::new(entry.clone())))?)
            }
        }
    }
    fn write(
        &mut self,
        kind: &str,
        time: DateTime<Utc>,
        data: &[u8],
        level: Option<&str>,
    ) -> Result<(), Box<dyn Error>> {
        match self {
            Self::Sync(output) => output.write_with_level(kind, time, data, level),
            Self::Async(output) => Ok(output.write(kind, time, data, level)?),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Sync(output) => output.finish(),
            Self::Async(output) => output.flush(),
        }
    }
    fn finish(&mut self) -> io::Result<()> {
        match self {
            Self::Sync(output) => output.close(),
            Self::Async(output) => output.finish(),
        }
    }
}

pub fn run(args: Vec<String>, registry: &GeneratorRegistry) -> Result<(), Box<dyn Error>> {
    let use_original = original_command(&args[0]);
    for name in registry.types() {
        if LogType::from_str(name, false).is_ok() && !registry.is_builtin(name) {
            return Err(format!("external generator conflicts with built-in type: {name}").into());
        }
    }
    if args[1] == "config" {
        return config_command(args, registry);
    }
    if args[1] == "list-types" {
        clap::Command::new("log-forge list-types")
            .about("List log types")
            .try_get_matches_from(
                std::iter::once(crate::app::command_name(&args[0], "list-types"))
                    .chain(args.iter().skip(2).cloned()),
            )?;
        for kind in LogType::value_variants() {
            println!(
                "{}",
                kind.to_possible_value().ok_or("invalid type")?.get_name()
            );
        }
        for name in registry.types() {
            if !registry.is_builtin(name) {
                println!("{name}");
            }
        }
        return Ok(());
    }
    let program = crate::app::command_name(&args[0], "start");
    let cli = Start::try_parse_from(std::iter::once(program).chain(args.into_iter().skip(2)))?;
    let continue_on_error = cli.continue_on_error || (use_original && !cli.fail_fast);
    if cli.standalone_config && cli.config.is_none() {
        return Err("--standalone-config requires --config".into());
    }
    let mut config = if let Some(path) = &cli.config {
        parse_document(
            &fs::read_to_string(path)?,
            Some(path),
            !cli.standalone_config && (use_original || cli.original_defaults),
        )?
        .log_generator
    } else {
        parse_document("{}", None, true)?.log_generator
    };
    let memory_limit = config
        .global
        .memory_limit
        .as_ref()
        .map(parse_size)
        .transpose()?;
    if memory_limit == Some(0) {
        return Err("memory_limit must be greater than zero".into());
    }
    if let Some(limit) = memory_limit {
        check_memory_limit(limit)?;
    }
    if let Some(monitoring) = &config.monitoring {
        monitoring.validate()?;
    }
    validate_service_metrics(&config)?;
    let output_overridden = cli.output.is_some();
    let global = &mut config.global;
    if let Some(v) = cli.output {
        global.output_format = v;
    }
    if let Some(v) = cli.path {
        global.output_path = v;
    }
    if let Some(v) = cli.total {
        global.total_logs = v;
    }
    if let Some(v) = cli.interval {
        global.generation_interval = v;
    }
    if let Some(v) = cli.start_time {
        global.start_time = Some(v);
    }
    if cli.random_interval {
        global.random_interval = true;
    } else if cli.fixed_interval {
        global.random_interval = false;
    }
    if cli.real_time && cli.virtual_time {
        return Err("--real-time conflicts with --virtual-time".into());
    }
    if cli.real_time {
        global.virtual_time = false;
    }
    if cli.virtual_time || (use_original && !cli.real_time) {
        global.virtual_time = true;
    }
    if let Some(v) = cli.host {
        global.network_host = v;
    }
    if let Some(v) = cli.port {
        global.network_port = v;
    }
    if let Some(v) = cli.protocol {
        global.network_protocol = v;
    }
    if let Some(v) = cli.endpoint {
        global.network_endpoint = v;
    }
    if let Some(v) = cli.network_timeout {
        global.network_timeout = v;
    }
    if let Some(v) = cli.rotation_size {
        global.rotation_size = Some(v);
    }
    if let Some(v) = cli.rotation_time {
        global.rotation_time = Some(v);
    }
    if let Some(v) = cli.max_files {
        global.max_files = v;
    }
    if let Some(v) = cli.threads {
        global.threads = Some(v);
    }
    if cli.async_io {
        global.async_io = Some(true);
    }
    if cli.sync_io {
        global.async_io = Some(false);
    }
    if !global.generation_interval.is_finite() || global.generation_interval < 0.0 {
        return Err("generation_interval must be a finite nonnegative number".into());
    }
    if global.buffer_size == Some(0) || global.batch_size == Some(0) {
        return Err("buffer_size and batch_size must be greater than zero".into());
    }
    if global.thread_pool_size == Some(0) {
        return Err("thread_pool_size must be greater than zero".into());
    }
    if global.threads == Some(0) {
        return Err("threads must be greater than zero".into());
    }
    if chrono::format::StrftimeItems::new(&global.multi_file.date_format)
        .any(|item| matches!(item, chrono::format::Item::Error))
    {
        return Err("invalid multi-file date_format".into());
    }
    if let Some(handlers) = &config.output {
        if output_overridden {
            eprintln!("warning: configured output handlers are overridden by --output");
        } else {
            validate_handlers(handlers)?;
        }
    }
    validate_json_mappings(
        global,
        if output_overridden {
            None
        } else {
            config.output.as_ref()
        },
    )?;
    let patterns = library_patterns(&config.error_patterns)?;
    let mut kinds = Vec::new();
    if !cli.types.is_empty() {
        let mut seen = std::collections::HashSet::new();
        for raw_name in cli.types {
            let name = raw_name.trim();
            if name.is_empty() || !seen.insert(name.to_owned()) {
                continue;
            }
            let kind = resolve_kind(name, config.log_types.get(name), registry)?;
            let weight = config
                .log_types
                .get(name)
                .map_or(1.0, |config| config.frequency);
            kinds.push((name.to_owned(), kind, weight));
        }
    } else if config.log_types.is_empty() {
        for (name, weight) in [
            ("nginx_access", 0.4),
            ("nginx_error", 0.05),
            ("syslog", 0.15),
            ("fastapi", 0.15),
            ("goloop", 0.2),
        ] {
            kinds.push((
                name.to_string(),
                Some(LogType::from_str(name, false).map_err(|_| "invalid default type")?),
                weight,
            ));
        }
    } else {
        for (name, cfg) in &config.log_types {
            if cfg.enabled {
                let kind = resolve_kind(name, Some(cfg), registry)?;
                kinds.push((name.clone(), kind, cfg.frequency));
            }
        }
    }
    if !patterns.is_empty()
        && !kinds
            .iter()
            .any(|(_, kind, _)| matches!(kind, Some(LogType::ErrorPattern)))
    {
        eprintln!("warning: error_patterns library is not used; enable the error_pattern log type");
    }
    let weights = crate::weights::checked(kinds.iter().map(|(_, _, w)| *w))?;
    let mut builtin_options = HashMap::new();
    let mut external = HashMap::new();
    let mut frequency_controllers = HashMap::new();
    let initialization: Result<(), Box<dyn Error>> = (|| {
        for (name, kind, _) in &kinds {
            if matches!(kind, Some(LogType::Custom))
                && let Some(settings) = config.log_types.get(name)
                && let Some(frequency) = &settings.field_frequency
            {
                let generator = ControlledCustom {
                    settings: settings.settings(name)?,
                    frequency: crate::frequency::FieldFrequencyController::new(
                        frequency.clone(),
                        cli.seed,
                    )?,
                    inactive_value: settings
                        .field_inactive_value
                        .clone()
                        .unwrap_or_else(|| "-".into()),
                    rng: StdRng::seed_from_u64(cli.seed),
                    index: 0,
                };
                external.insert(
                    name.clone(),
                    Box::new(generator) as Box<dyn crate::LogGenerator>,
                );
                continue;
            }
            if matches!(kind, Some(LogType::Process)) {
                let settings = config
                    .log_types
                    .get(name)
                    .ok_or("process plugin configuration required")?;
                let timeout = settings.validate_process()?;
                let generator = crate::process_plugin::ProcessGenerator::spawn(
                    settings
                        .plugin_command
                        .as_deref()
                        .ok_or("plugin_command is required")?,
                    &settings.plugin_args,
                    settings
                        .plugin_config
                        .as_ref()
                        .unwrap_or(&serde_yaml::Value::Null),
                    cli.seed,
                    timeout,
                )
                .map_err(|error| -> Box<dyn Error> { error })?;
                external.insert(
                    name.clone(),
                    Box::new(generator) as Box<dyn crate::LogGenerator>,
                );
                continue;
            }
            if matches!(kind, Some(LogType::ErrorLibrary)) {
                let settings = config.log_types.get(name);
                if let Some(settings) = settings {
                    settings.settings(name)?;
                }
                let generator = crate::error_library::ErrorLibrary::from_sources(
                    settings.and_then(|s| s.patterns_dir.as_deref()),
                    settings.and_then(|s| s.error_category.as_deref()),
                    settings.and_then(|s| s.error_severity.as_deref()),
                    cli.seed,
                )
                .map_err(|error| -> Box<dyn Error> { error })?;
                external.insert(
                    name.clone(),
                    Box::new(generator) as Box<dyn crate::LogGenerator>,
                );
                continue;
            }
            if matches!(kind, Some(LogType::Scenario)) {
                let settings = config
                    .log_types
                    .get(name)
                    .ok_or("scenario configuration required")?;
                settings.settings(name)?;
                if let Some(frequency) = &settings.error_frequency {
                    frequency_controllers.insert(
                        name.clone(),
                        crate::frequency::ErrorFrequencyController::new(
                            frequency.clone(),
                            cli.seed,
                        )?,
                    );
                }
                let generator = crate::scenario::ScenarioGenerator::from_sources(
                    settings
                        .scenario_file
                        .as_ref()
                        .ok_or("scenario_file is required")?,
                    settings.patterns_dir.as_deref(),
                    cli.seed,
                )
                .map_err(|e| -> Box<dyn Error> { e })?;
                external.insert(
                    name.clone(),
                    Box::new(generator) as Box<dyn crate::LogGenerator>,
                );
                continue;
            }
            if kind.is_none() {
                if let Some(settings) = config.log_types.get(name) {
                    settings.validate_external()?;
                }
                let value = config
                    .log_types
                    .get(name)
                    .and_then(|cfg| cfg.plugin_config.as_ref())
                    .unwrap_or(&serde_yaml::Value::Null);
                external.insert(
                    name.clone(),
                    registry
                        .create(name, value)
                        .map_err(|e| -> Box<dyn Error> { e })?,
                );
                continue;
            }
            let type_config = config.log_types.get(name);
            let builtin_kind = kind.ok_or("missing built-in type")?;
            let mut generator_settings = type_config
                .map(|cfg| cfg.settings(name))
                .transpose()?
                .unwrap_or_default();
            if matches!(builtin_kind, LogType::ErrorPattern) {
                if patterns.is_empty() {
                    return Err("error_pattern requires a nonempty error_patterns library".into());
                }
                generator_settings.error_library = patterns.clone();
            }
            if matches!(builtin_kind, LogType::Docker) {
                initialize_docker(&mut generator_settings, cli.seed);
            }
            if matches!(builtin_kind, LogType::KubernetesPod) {
                initialize_kubernetes_pod(&mut generator_settings, cli.seed);
            }
            builtin_options.insert(
                name.clone(),
                Args {
                    total: 1,
                    output: PathBuf::new(),
                    seed: 0,
                    start: None,
                    format: if type_config.is_some_and(|cfg| {
                        cfg.log_format.as_deref().map_or_else(
                            || cfg.patterns.len() == 1 && cfg.patterns[0] == "common",
                            |format| format == "common",
                        )
                    }) {
                        Format::Common
                    } else {
                        Format::Combined
                    },
                    log_type: builtin_kind,
                    nginx: type_config
                        .map(|cfg| cfg.nginx_fields(name))
                        .transpose()?
                        .unwrap_or_default(),
                    settings: generator_settings,
                    record_index: 0,
                    trace: None,
                },
            );
        }
        Ok(())
    })();
    if let Err(error) = initialization {
        let cleanup_errors: Vec<_> = external
            .iter_mut()
            .filter_map(|(name, generator)| {
                generator
                    .finish()
                    .err()
                    .map(|error| format!("{name}: {error}"))
            })
            .collect();
        if cleanup_errors.is_empty() {
            return Err(error);
        }
        return Err(format!(
            "{error}; generator cleanup failed: {}",
            cleanup_errors.join(", ")
        )
        .into());
    }
    let run_result: Result<(), Box<dyn Error>> = (|| {
        let mut tracing = if let Some(config) = &config.distributed_tracing
            && config.enabled
        {
            let services: Vec<_> = kinds
                .iter()
                .enumerate()
                .filter_map(|(index, (name, kind, _))| {
                    if matches!(kind, Some(LogType::Microservice)) {
                        builtin_options
                            .get(name)
                            .and_then(|options| options.settings.service_name.as_ref())
                            .map(|service| (index, service.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            if services.is_empty() {
                return Err("distributed tracing requires service logs".into());
            }
            Some(crate::tracing::TracePlanner::new(config, &services)?)
        } else {
            None
        };
        if global.lazy_loading == Some(false) {
            crate::patterns::load_builtins();
        }
        let thread_count = match (global.threads, global.thread_pool_size) {
            (Some(threads), Some(capacity)) => threads.min(capacity),
            (Some(threads), None) => threads,
            (None, Some(capacity)) => capacity,
            (None, None) => 1,
        };
        let parallel = if thread_count > 1
            && global.virtual_time
            && external.is_empty()
            && (!continue_on_error
                || !kinds
                    .iter()
                    .any(|(_, kind, _)| matches!(kind, Some(LogType::Custom))))
        {
            Some(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(thread_count)
                    .build()?,
            )
        } else {
            // Stateful generators and real-time scheduling require sequential processing.
            if thread_count > 1 {
                eprintln!(
                    "warning: threads are not applied to real-time or stateful generator runs"
                );
            }
            None
        };
        if global.output_format == "network"
            && (global.network_host.is_empty() || global.network_port == 0)
        {
            return Err("network host and port are required".into());
        }
        let rotation = resolve_rotation(global, cli.max_files)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let _signal_guard = SignalGuard::register(Arc::clone(&stopped))?;
        let output = if let Some(handlers) = config.output.as_ref().filter(|_| !output_overridden) {
            open_handlers(handlers, global, rotation)?
        } else {
            match global.output_format.as_str() {
                "console" => Output::Console,
                "file" | "json" => {
                    let mut path = global.output_path.clone();
                    if path.is_dir() {
                        path.push(if global.output_format == "file" {
                            "generated_logs.log"
                        } else {
                            "generated_logs.json"
                        });
                    }
                    if global.output_format == "file" {
                        Output::File(RotatingFile::open(&path, rotation)?)
                    } else {
                        Output::Json {
                            output: Box::new(Output::File(RotatingFile::open(&path, rotation)?)),
                            options: json_options(global),
                        }
                    }
                }
                "multi_file" => Output::Multi {
                    base: global.output_path.clone(),
                    pattern: if global.multi_file.separate_by_type {
                        global.multi_file.file_pattern.clone()
                    } else {
                        global.multi_file.file_pattern.replace("{log_type}", "all")
                    },
                    date_format: global.multi_file.date_format.clone(),
                    files: HashMap::new(),
                    closed_opened: HashMap::new(),
                    type_paths: None,
                    use_tick: 0,
                    rotation,
                },
                "network" if global.network_protocol == "http" => {
                    Output::Http(crate::http_output::HttpOutput::new(
                        &global.network_host,
                        global.network_port,
                        &global.network_endpoint,
                        network_timeout(global.network_timeout)?,
                        false,
                    )?)
                }
                "network" => open_network(
                    &global.network_host,
                    global.network_port,
                    &global.network_protocol,
                    network_timeout(global.network_timeout)?,
                    global.network_max_retries,
                    parse_retry_delay(global.network_retry_delay)?,
                )?,
                _ => return Err(format!("unknown output format: {}", global.output_format).into()),
            }
        };
        let mut output = if global.async_io.unwrap_or(false) {
            OutputSink::Async(AsyncOutput::new(output)?)
        } else {
            OutputSink::Sync(Box::new(output))
        };
        #[cfg(any(unix, windows))]
        let mut control = cli
            .control_socket
            .as_deref()
            .map(|path| crate::control::ControlServer::start(path, Arc::clone(&stopped)))
            .transpose()?;
        let mut internal_log = config
            .logging
            .as_ref()
            .map(crate::internal_log::InternalLog::open)
            .transpose()?;
        if let Some(logger) = &mut internal_log {
            logger.info("generation started", line!())?;
        }
        let mut rng = StdRng::seed_from_u64(cli.seed);
        let mut time = match &global.start_time {
            Some(v) => parse_time(v)?,
            None => Utc::now(),
        };
        let mut count = 0;
        let mut custom_count = 0;
        let mut record = Vec::new();
        type GeneratedBytes = (Vec<u8>, Option<String>);
        let mut pending: VecDeque<(usize, DateTime<Utc>, io::Result<GeneratedBytes>)> =
            VecDeque::new();
        let mut deferred_error: Option<&'static str> = None;
        let mut time_needs_advance = false;
        let monitoring_config = config.monitoring.clone().unwrap_or_default();
        let monitor_enabled =
            !cli.no_monitor && (cli.monitor || monitoring_config.enable_statistics);
        let mut monitor = if monitor_enabled
            || monitoring_config.metrics.enabled
            || monitoring_config.health_check.enabled
        {
            Some(crate::monitor::Monitor::new(
                kinds.iter().map(|(name, _, _)| name.clone()),
                global.total_logs,
                !cli.no_progress,
                monitor_enabled,
                &monitoring_config,
            )?)
        } else {
            None
        };
        let mut last_memory_check = std::time::Instant::now();
        while !stopped.load(Ordering::SeqCst)
            && (global.total_logs == 0 || count < global.total_logs)
        {
            if let Some(limit) = memory_limit
                && last_memory_check.elapsed() >= std::time::Duration::from_millis(100)
            {
                if let Err(error) = check_memory_limit(limit) {
                    output.finish()?;
                    return Err(error.into());
                }
                last_memory_check = std::time::Instant::now();
            }
            #[cfg(any(unix, windows))]
            if let Some(control) = &control {
                while control.state.paused() && !stopped.load(Ordering::SeqCst) {
                    std::thread::park();
                }
                if stopped.load(Ordering::SeqCst) {
                    break;
                }
            }
            if let Some(pool) = &parallel
                && pending.is_empty()
            {
                if let Some(error) = deferred_error {
                    output.flush()?;
                    return Err(error.into());
                }
                if time_needs_advance {
                    time = advance_virtual(
                        time,
                        global.generation_interval,
                        global.random_interval,
                        &mut rng,
                    )?;
                    time_needs_advance = false;
                }
                let size = thread_count.saturating_mul(64).min(1024);
                let mut jobs = Vec::with_capacity(size);
                let mut planned_custom = custom_count;
                for _ in 0..size {
                    if global.total_logs > 0 && count + jobs.len() as u64 >= global.total_logs {
                        break;
                    }
                    let (index, trace) = select_traced(&weights, &mut tracing, &mut rng)?;
                    let (name, kind, _) = &kinds[index];
                    let mut options = builtin_options
                        .get(name)
                        .ok_or("missing built-in generator")?
                        .clone();
                    options.seed = rng.r#gen();
                    options.record_index = planned_custom;
                    options.trace = trace;
                    if matches!(kind, Some(LogType::Custom)) {
                        planned_custom += 1;
                    }
                    jobs.push((index, time, options));
                    if global.total_logs == 0 || count + (jobs.len() as u64) < global.total_logs {
                        match advance_virtual(
                            time,
                            global.generation_interval,
                            global.random_interval,
                            &mut rng,
                        ) {
                            Ok(next) => time = next,
                            Err(error) => {
                                deferred_error = Some(error);
                                break;
                            }
                        }
                    } else {
                        time_needs_advance = true;
                    }
                }
                let generated: Vec<_> = pool.install(|| {
                    jobs.into_par_iter()
                        .map(|(index, stamp, options)| {
                            let mut bytes = Vec::new();
                            let mut level = None;
                            let result =
                                generate_with_level(&mut bytes, &options, stamp, &mut level)
                                    .map(|()| (bytes, level));
                            (index, stamp, result)
                        })
                        .collect()
                });
                pending.extend(generated);
                continue;
            }
            let mut record_level = None;
            let mut structured_record = None;
            let (index, stamp, emit) = if parallel.is_some() {
                let (index, stamp, result) =
                    pending.pop_front().ok_or("missing generated record")?;
                match result {
                    Ok(generated) => (record, record_level) = generated,
                    Err(error) if continue_on_error => {
                        #[cfg(any(unix, windows))]
                        if let Some(control) = &control {
                            control.state.record_error()?;
                        }
                        eprintln!("generation error for {}: {error}", kinds[index].0);
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                }
                (index, stamp, true)
            } else {
                let (index, trace) = select_traced(&weights, &mut tracing, &mut rng)?;
                let (name, kind, _) = &kinds[index];
                let stamp = if global.virtual_time {
                    time
                } else {
                    Utc::now()
                };
                let emit = match frequency_controllers.get_mut(name) {
                    Some(controller) => controller.should_generate(stamp)?,
                    None => true,
                };
                if emit {
                    record.clear();
                    let generation = (|| -> Result<(), Box<dyn Error>> {
                        if !external.contains_key(name) && kind.is_some() {
                            let options = builtin_options
                                .get_mut(name)
                                .ok_or("missing built-in generator")?;
                            options.seed = rng.r#gen();
                            options.record_index = custom_count;
                            options.trace = trace;
                            generate_with_level(&mut record, options, stamp, &mut record_level)?;
                        } else {
                            let generator =
                                external.get_mut(name).ok_or("missing external generator")?;
                            if let Some(entry) = generator
                                .generate_entry(stamp)
                                .map_err(|e| -> Box<dyn Error> { e })?
                            {
                                record.extend_from_slice(entry.to_string().as_bytes());
                                record_level = Some(entry.level.clone());
                                structured_record = Some(entry);
                            } else {
                                let generated = generator
                                    .generate_record(stamp)
                                    .map_err(|e| -> Box<dyn Error> { e })?;
                                record.extend_from_slice(generated.message.as_bytes());
                                record_level = generated.level;
                            }
                            if !record.ends_with(b"\n") {
                                record.push(b'\n');
                            }
                        }
                        Ok(())
                    })();
                    if let Err(error) = generation {
                        if continue_on_error {
                            #[cfg(any(unix, windows))]
                            if let Some(control) = &control {
                                control.state.record_error()?;
                            }
                            eprintln!("generation error for {name}: {error}");
                            continue;
                        }
                        return Err(error);
                    }
                }
                (index, stamp, emit)
            };
            if emit {
                let (name, kind, _) = &kinds[index];
                if continue_on_error {
                    for error in output.write_continuing(
                        name,
                        stamp,
                        &record,
                        record_level.as_deref(),
                        structured_record.as_ref(),
                    )? {
                        #[cfg(any(unix, windows))]
                        if let Some(control) = &control {
                            control.state.record_error()?;
                        }
                        eprintln!("output error for {name}: {error}");
                    }
                } else if let Some(entry) = &structured_record {
                    output.write_entry(entry)?;
                } else {
                    output.write(name, stamp, &record, record_level.as_deref())?;
                }
                if let Some(monitor) = &monitor {
                    let service = builtin_options
                        .get(name)
                        .and_then(|options| options.settings.service_name.as_deref());
                    monitor.record(index, service, &record)?;
                }
                #[cfg(any(unix, windows))]
                if let Some(control) = &control {
                    control.state.record(name, &record)?;
                }
                if matches!(kind, Some(LogType::Custom)) {
                    custom_count += 1;
                }
                count += 1;
                if global
                    .batch_size
                    .is_some_and(|batch| count % batch as u64 == 0)
                {
                    if continue_on_error {
                        for error in output.flush_continuing()? {
                            #[cfg(any(unix, windows))]
                            if let Some(control) = &control {
                                control.state.record_error()?;
                            }
                            eprintln!("flush error: {error}");
                        }
                    } else {
                        output.flush()?;
                    }
                }
                if global.total_logs > 0 && count >= global.total_logs {
                    break;
                }
            }
            if global.virtual_time && parallel.is_none() {
                time = advance_virtual(
                    time,
                    global.generation_interval,
                    global.random_interval,
                    &mut rng,
                )?;
            } else if !global.virtual_time && global.generation_interval > 0.0 {
                let delay = std::time::Duration::try_from_secs_f64(global.generation_interval)?;
                let deadline = std::time::Instant::now()
                    .checked_add(delay)
                    .ok_or("interval is too large")?;
                while !stopped.load(Ordering::SeqCst) {
                    #[cfg(any(unix, windows))]
                    if control
                        .as_ref()
                        .is_some_and(|control| control.state.paused())
                    {
                        break;
                    }
                    let Some(remaining) =
                        deadline.checked_duration_since(std::time::Instant::now())
                    else {
                        break;
                    };
                    std::thread::park_timeout(remaining);
                }
            }
        }
        if continue_on_error {
            for error in output.close_continuing()? {
                #[cfg(any(unix, windows))]
                if let Some(control) = &control {
                    control.state.record_error()?;
                }
                eprintln!("close error: {error}");
            }
        } else {
            output.finish()?;
        }
        if let Some(limit) = memory_limit {
            check_memory_limit(limit)?;
        }
        #[cfg(any(unix, windows))]
        if let Some(control) = &mut control {
            control.finish()?;
        }
        if let Some(monitor) = &mut monitor {
            monitor.finish()?;
        }
        if let Some(logger) = &mut internal_log {
            logger.info(&format!("generation finished: {count} records"), line!())?;
        }
        Ok(())
    })();
    let cleanup_errors: Vec<_> = external
        .iter_mut()
        .filter_map(|(name, generator)| {
            generator
                .finish()
                .err()
                .map(|error| format!("{name}: {error}"))
        })
        .collect();
    match (run_result, cleanup_errors.is_empty()) {
        (Ok(()), true) => Ok(()),
        (Err(error), true) => Err(error),
        (Ok(()), false) => {
            Err(format!("generator cleanup failed: {}", cleanup_errors.join(", ")).into())
        }
        (Err(error), false) => Err(format!(
            "{error}; generator cleanup failed: {}",
            cleanup_errors.join(", ")
        )
        .into()),
    }
}
