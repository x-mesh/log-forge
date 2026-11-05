use chrono::{DateTime, Utc};
use clap::{Command, Parser, ValueEnum};
use rand::{Rng, SeedableRng, distributions::Distribution, rngs::StdRng, seq::SliceRandom};
use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::PathBuf,
};
#[path = "compat.rs"]
pub(crate) mod compat;

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Common,
    Combined,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "snake_case")]
enum LogType {
    NginxAccess,
    NginxError,
    ApacheAccess,
    ApacheError,
    DjangoRequest,
    DjangoSql,
    Docker,
    KubernetesPod,
    KubernetesEvent,
    MysqlQuery,
    MysqlError,
    MysqlSlow,
    Postgresql,
    PostgresqlSlow,
    Syslog,
    Fastapi,
    Goloop,
    Redis,
    SshAuth,
    KafkaBroker,
    Haproxy,
    Custom,
    Scenario,
    Microservice,
    ErrorPattern,
    ErrorLibrary,
    Process,
}

#[derive(Clone, Parser)]
#[command(about = "Generate Nginx access logs")]
struct Args {
    #[arg(long)]
    total: u64,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value_t = 0)]
    seed: u64,
    #[arg(long)]
    start: Option<DateTime<Utc>>,
    #[arg(long, value_enum, default_value_t = Format::Combined)]
    format: Format,
    #[arg(long, value_enum, default_value = "nginx_access")]
    log_type: LogType,
    #[arg(skip)]
    nginx: compat::NginxFields,
    #[arg(skip)]
    settings: compat::GeneratorFields,
    #[arg(skip)]
    record_index: u64,
    #[arg(skip)]
    trace: Option<crate::tracing::TraceContext>,
}

#[derive(Parser)]
#[command(
    about = "Print sample records on stdout",
    after_help = "Example:\n  log-forge sample --type nginx_access --count 5 --seed 42"
)]
struct SampleArgs {
    #[arg(long = "type", help = "Select a log type from list-types")]
    kind: String,
    #[arg(long, default_value_t = 5, help = "Set the number of records")]
    count: u64,
    #[arg(long, default_value_t = 0, help = "Set the random seed")]
    seed: u64,
    #[arg(long, help = "Set the virtual start time")]
    start_time: Option<String>,
    #[arg(long, help = "Read type settings from a YAML or JSON configuration")]
    config: Option<PathBuf>,
}

fn database_sql(
    kind: &str,
    table: &str,
    value: u32,
    tables: &[String],
    rng: &mut StdRng,
) -> String {
    let columns = [
        "id",
        "name",
        "email",
        "password",
        "created_at",
        "updated_at",
        "status",
        "type",
        "value",
        "description",
    ];
    match kind {
        "SELECT" => {
            let count = rng.gen_range(1..=3);
            let column = columns
                .choose_multiple(rng, count)
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            if rng.gen_bool(0.3) {
                format!("SELECT {column} FROM {table}")
            } else if rng.gen_bool(0.6) {
                format!("SELECT {column} FROM {table} WHERE id = {value}")
            } else {
                let choices: Vec<&str> = if tables.is_empty() {
                    MYSQL_TABLES
                        .iter()
                        .copied()
                        .filter(|other| *other != table)
                        .collect()
                } else {
                    tables
                        .iter()
                        .map(String::as_str)
                        .filter(|other| *other != table)
                        .collect()
                };
                if choices.is_empty() {
                    format!("SELECT {column} FROM {table} WHERE id = {value}")
                } else {
                    let other = choices[rng.gen_range(0..choices.len())];
                    let name = table.rsplit('.').next().unwrap_or(table);
                    let foreign_key = format!("{}_id", &name[..name.len() - 1]);
                    format!(
                        "SELECT {column} FROM {table} t1 JOIN {other} t2 ON t1.id = t2.{foreign_key}"
                    )
                }
            }
        }
        "INSERT" => {
            let count = rng.gen_range(2..=4);
            let selected: Vec<_> = columns[1..].choose_multiple(rng, count).copied().collect();
            let values: Vec<_> = selected
                .iter()
                .map(|column| mysql_field_value(column, value))
                .collect();
            format!(
                "INSERT INTO {table} ({}) VALUES ({})",
                selected.join(", "),
                values.join(", ")
            )
        }
        "UPDATE" => {
            let column = columns[rng.gen_range(1..columns.len())];
            let condition = columns[rng.gen_range(0..columns.len())];
            format!(
                "UPDATE {table} SET {column} = {} WHERE {condition} = {value}",
                mysql_field_value(column, value)
            )
        }
        "DELETE" => {
            let condition = columns[rng.gen_range(0..columns.len())];
            format!("DELETE FROM {table} WHERE {condition} = {value}")
        }
        "SHOW" => match rng.gen_range(0..4) {
            0 => "SHOW TABLES".to_owned(),
            1 => "SHOW DATABASES".to_owned(),
            2 => format!("SHOW COLUMNS FROM {table}"),
            _ => format!("SHOW INDEX FROM {table}"),
        },
        "DESCRIBE" => format!("DESCRIBE {table}"),
        _ => unreachable!("query types are validated before generation"),
    }
}

fn mysql_field_value(column: &str, value: u32) -> String {
    if column.contains("email") {
        format!("'user{value}@example.test'")
    } else if column.contains("name") {
        format!("'User {value}'")
    } else if column.contains("at") {
        "NOW()".to_owned()
    } else {
        format!("'value{value}'")
    }
}

fn postgres_statement(kind: &str, table: &str, other: &str, rng: &mut StdRng) -> String {
    let columns = [
        "id",
        "name",
        "email",
        "password_hash",
        "created_at",
        "updated_at",
        "status",
        "type",
        "value",
        "description",
    ];
    let column = columns[rng.gen_range(0..columns.len())];
    match kind {
        "SELECT" => {
            if rng.gen_bool(0.3) {
                let count = rng.gen_range(1..=3);
                let selected: Vec<_> = columns.choose_multiple(rng, count).copied().collect();
                format!("SELECT {} FROM {table}", selected.join(", "))
            } else if rng.gen_bool(0.6) || table == other {
                format!("SELECT * FROM {table} WHERE {column} = $1")
            } else {
                let name = other.rsplit('.').next().unwrap_or(other);
                let foreign_key = format!("{}_id", &name[..name.len() - 1]);
                format!(
                    "SELECT t1.*, t2.name FROM {table} t1 JOIN {other} t2 ON t1.{foreign_key} = t2.id"
                )
            }
        }
        "INSERT" => {
            let count = rng.gen_range(2..=4);
            let selected: Vec<_> = columns[1..].choose_multiple(rng, count).copied().collect();
            let placeholders: Vec<_> = (1..=count).map(|index| format!("${index}")).collect();
            format!(
                "INSERT INTO {table} ({}) VALUES ({})",
                selected.join(", "),
                placeholders.join(", ")
            )
        }
        "UPDATE" => {
            let selected = columns[rng.gen_range(1..columns.len())];
            format!("UPDATE {table} SET {selected} = $1 WHERE {column} = $2")
        }
        "DELETE" => format!("DELETE FROM {table} WHERE {column} = $1"),
        "CREATE" => {
            if rng.gen_bool(0.7) {
                format!(
                    "CREATE TABLE {table} (id SERIAL PRIMARY KEY, name VARCHAR(255), created_at TIMESTAMP DEFAULT NOW())"
                )
            } else {
                format!("CREATE INDEX idx_generated ON {table} ({column})")
            }
        }
        "DROP" => {
            if rng.gen_bool(0.8) {
                format!("DROP TABLE IF EXISTS {table}")
            } else {
                "DROP INDEX IF EXISTS idx_generated".into()
            }
        }
        "ALTER" => {
            if rng.gen_bool(0.5) {
                format!("ALTER TABLE {table} ADD COLUMN extra VARCHAR(255)")
            } else {
                format!("ALTER TABLE {table} DROP COLUMN IF EXISTS {column}")
            }
        }
        _ => unreachable!("query types are validated before generation"),
    }
}
fn postgres_error(level: &str, table: &str, rng: &mut StdRng) -> String {
    match level {
        "PANIC" => match rng.gen_range(0..3) {
            0 => "PANIC: could not write to file: No space left on device".into(),
            1 => "PANIC: corrupted page pointers: lower = 0, upper = 0, special = 8192".into(),
            _ => "PANIC: WAL contains references to invalid pages".into(),
        },
        "FATAL" => match rng.gen_range(0..5) {
            0 => "FATAL: database \"production\" does not exist".into(),
            1 => "FATAL: role \"app\" does not exist".into(),
            2 => "FATAL: too many connections for role \"user\"".into(),
            3 => "FATAL: password authentication failed for user \"postgres\"".into(),
            _ => "FATAL: no pg_hba.conf entry for host \"192.0.2.1\", user \"app\", database \"production\"".into(),
        },
        _ => match rng.gen_range(0..10) {
            0 => format!("ERROR: relation \"{table}\" does not exist at character {}", rng.gen_range(10..=100)),
            1 => "ERROR: column \"status\" does not exist".into(),
            2 => format!("ERROR: duplicate key value violates unique constraint \"{}_pkey\"", table.replace('.', "_")),
            3 => "ERROR: foreign key constraint violation".into(),
            4 => "ERROR: null value in column violates not-null constraint".into(),
            5 => "ERROR: connection failure".into(),
            6 => "ERROR: connection does not exist".into(),
            7 => "ERROR: current transaction is aborted".into(),
            8 => "ERROR: serialization failure".into(),
            _ => "ERROR: too many connections".into(),
        },
    }
}
fn postgres_admin(level: &str, table: &str, ip: (u32, u32, u32, u32), rng: &mut StdRng) -> String {
    match level {
        "INFO" => [
            "database system is ready to accept connections",
            "autovacuum launcher started",
            "logical replication launcher started",
            "background writer process started",
        ][rng.gen_range(0..4)]
        .into(),
        "WARNING" => [
            "could not receive data from client: Connection reset by peer",
            "terminating connection due to administrator command",
            "canceling statement due to user request",
            "there is no transaction in progress",
        ][rng.gen_range(0..4)]
        .into(),
        _ => match rng.gen_range(0..7) {
            0 => format!(
                "connection received: host={}.{}.{}.{} port={}",
                ip.0,
                ip.1,
                ip.2,
                ip.3,
                rng.gen_range(30000..=60000)
            ),
            1 => "connection authorized: user=app database=production".into(),
            2 => format!(
                "disconnection: session time: {} seconds",
                rng.gen_range(1..=3600)
            ),
            3 => "checkpoint starting: time".into(),
            4 => format!(
                "checkpoint complete: wrote {} buffers",
                rng.gen_range(100..=10000)
            ),
            5 => format!(
                "automatic vacuum of table \"{table}\": index scans: {}",
                rng.gen_range(0..=5)
            ),
            _ => format!(
                "automatic analyze of table \"{table}\" system usage: CPU {:.2}s",
                rng.gen_range(0.1..5.0)
            ),
        },
    }
}

fn slow_query_duration(rng: &mut StdRng) -> f64 {
    let choice = rng.gen_range(0..100);
    if choice < 60 {
        rng.gen_range(1.0..10.0)
    } else if choice < 96 {
        rng.gen_range(10.0..60.0)
    } else {
        rng.gen_range(60.0..300.0)
    }
}
fn slow_query(kind: &str, table: &str, other: &str, postgres: bool, rng: &mut StdRng) -> String {
    let name = table.rsplit('.').next().unwrap_or(table);
    let foreign_key = format!("{}_id", &name[..name.len() - 1]);
    let date_expr = if postgres {
        "NOW() - INTERVAL '30 days'"
    } else {
        "DATE_SUB(NOW(), INTERVAL 30 DAY)"
    };
    match kind {
        "UPDATE" => {
            format!("UPDATE {table} SET status = 'processed' WHERE created_at < {date_expr}")
        }
        "DELETE" => format!(
            "DELETE FROM {table} WHERE id NOT IN (SELECT DISTINCT {foreign_key} FROM {other} WHERE status = 'active')"
        ),
        "INSERT" if !postgres => format!(
            "INSERT INTO {table} (name, email, created_at) SELECT CONCAT('user_', id), CONCAT('user_', id, '@example.com'), NOW() FROM {other} WHERE id < 10000"
        ),
        "INSERT" => format!(
            "INSERT INTO {table} (name, created_at) SELECT name, created_at FROM {other} WHERE id < 10000"
        ),
        _ => match if postgres {
            rng.gen_range(0..6)
        } else if rng.gen_bool(0.4) {
            0
        } else if rng.gen_bool(0.7) {
            1
        } else {
            2
        } {
            0 => format!("SELECT * FROM {table}"),
            1 => {
                if postgres {
                    format!(
                        "SELECT t1.*, t2.* FROM {table} t1 JOIN {other} t2 ON t1.name ILIKE '%' || t2.name || '%'"
                    )
                } else {
                    format!(
                        "SELECT t1.*, t2.* FROM {table} t1 JOIN {other} t2 ON t1.name LIKE CONCAT('%', t2.name, '%')"
                    )
                }
            }
            2 => {
                let interval = if postgres {
                    "NOW() - INTERVAL '1 year'"
                } else {
                    "DATE_SUB(NOW(), INTERVAL 1 YEAR)"
                };
                format!(
                    "SELECT * FROM {table} WHERE id IN (SELECT {foreign_key} FROM {other} WHERE created_at > {interval})"
                )
            }
            3 => format!(
                "SELECT COUNT(*), AVG(id), MAX(created_at) FROM {table} GROUP BY EXTRACT(month FROM created_at)"
            ),
            4 => format!(
                "SELECT *, ROW_NUMBER() OVER (PARTITION BY status ORDER BY created_at DESC) FROM {table}"
            ),
            _ => format!(
                "WITH RECURSIVE tree AS (SELECT id, name, 0 AS level FROM {table} WHERE id = 1 UNION ALL SELECT c.id, c.name, p.level + 1 FROM {table} c JOIN tree p ON c.parent_id = p.id) SELECT * FROM tree"
            ),
        },
    }
}

fn database_table<'a>(
    tables: &'a [String],
    defaults: &'static [&'static str],
    rng: &mut StdRng,
) -> &'a str {
    if tables.is_empty() {
        defaults[rng.gen_range(0..defaults.len())]
    } else {
        &tables[rng.gen_range(0..tables.len())]
    }
}

const MYSQL_TABLES: &[&str] = &[
    "users",
    "orders",
    "products",
    "categories",
    "sessions",
    "logs",
    "settings",
    "permissions",
    "roles",
    "articles",
];
const MYSQL_SLOW_TABLES: &[&str] = &["users", "orders", "products", "categories", "logs"];
const POSTGRES_TABLES: &[&str] = &[
    "users",
    "orders",
    "products",
    "categories",
    "sessions",
    "audit_log",
    "settings",
    "permissions",
    "roles",
    "articles",
];
const POSTGRES_SLOW_TABLES: &[&str] = &["users", "orders", "products", "categories", "audit_log"];

pub(crate) fn command_name(program: &str, command: &str) -> String {
    let name = std::path::Path::new(program)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("log-forge");
    format!("{name} {command}")
}

fn root_command(name: &'static str) -> Command {
    let command = Command::new(name)
        .version(env!("CARGO_PKG_VERSION"))
        .about("Generate and inspect synthetic logs")
        .after_help(format!(
            "Examples:\n  {name} start --output console --types redis --total 3 --no-monitor\n  {name} validate --input ./logs\n\nRun '{name} <COMMAND> --help' for the options of a command."
        ))
        .subcommand(Command::new("init").about("Create a configuration interactively"))
        .subcommand(Command::new("start").about("Generate logs"))
        .subcommand(Command::new("config").about("Manage configuration"))
        .subcommand(Command::new("list-types").about("List log types"))
        .subcommand(Command::new("sample").about("Generate sample logs"))
        .subcommand(Command::new("validate").about("Validate log files"))
        .subcommand(Command::new("analyze").about("Analyze log files"));
    #[cfg(any(unix, windows))]
    let command = command
        .subcommand(Command::new("status").about("Show generator status"))
        .subcommand(Command::new("monitor").about("Monitor a generator"))
        .subcommand(Command::new("pause").about("Pause a generator"))
        .subcommand(Command::new("resume").about("Resume a generator"))
        .subcommand(Command::new("stop").about("Stop a generator"));
    command
}

fn nginx_method(rng: &mut StdRng) -> &'static str {
    let value = rng.gen_range(0..100);
    match value {
        0..=72 => "GET",
        73..=88 => "POST",
        89..=91 => "PUT",
        92..=93 => "DELETE",
        94..=96 => "HEAD",
        97..=98 => "OPTIONS",
        _ => "PATCH",
    }
}
fn nginx_status(rng: &mut StdRng) -> u16 {
    let distribution = [
        (200, 630),
        (301, 50),
        (302, 40),
        (304, 60),
        (204, 10),
        (206, 20),
        (400, 20),
        (401, 10),
        (403, 20),
        (404, 80),
        (418, 2),
        (429, 8),
        (499, 10),
        (500, 20),
        (502, 10),
        (503, 10),
        (504, 8),
    ];
    let total: u32 = distribution.iter().map(|(_, weight)| weight).sum();
    let mut pick = rng.gen_range(0..total);
    for (status, weight) in distribution {
        if pick < weight {
            return status;
        }
        pick -= weight;
    }
    unreachable!()
}
fn apache_method(rng: &mut StdRng) -> &'static str {
    match rng.gen_range(0..100) {
        0..=74 => "GET",
        75..=86 => "POST",
        87..=91 => "HEAD",
        92..=94 => "PUT",
        95..=96 => "DELETE",
        97..=98 => "OPTIONS",
        _ => "PATCH",
    }
}
fn apache_status(rng: &mut StdRng) -> u16 {
    match rng.gen_range(0..100) {
        0..=74 => 200,
        75..=82 => 404,
        83..=86 => 301,
        87..=90 => 302,
        91..=92 => 500,
        93..=94 => 403,
        95..=96 => 400,
        97 => 503,
        98 => 502,
        _ => 401,
    }
}
fn web_address(rng: &mut StdRng, private_probability: f64) -> (u32, u32, u32, u32) {
    if rng.gen_bool(private_probability) {
        return match rng.gen_range(0..3) {
            0 => (
                10,
                rng.gen_range(0..=255),
                rng.gen_range(0..=255),
                rng.gen_range(1..=254),
            ),
            1 => (
                172,
                rng.gen_range(16..=31),
                rng.gen_range(0..=255),
                rng.gen_range(1..=254),
            ),
            _ => (192, 168, rng.gen_range(0..=255), rng.gen_range(1..=254)),
        };
    }
    let first = loop {
        let value = rng.gen_range(1..=223);
        if ![10, 100, 127, 169, 172, 192, 198, 203].contains(&value) {
            break value;
        }
    };
    (
        first,
        rng.gen_range(0..=255),
        rng.gen_range(0..=255),
        rng.gen_range(1..=254),
    )
}

fn generate<W: Write>(mut output: W, args: &Args, start: DateTime<Utc>) -> io::Result<()> {
    generate_with_level(&mut output, args, start, &mut None)
}

fn generate_with_level<W: Write>(
    mut output: W,
    args: &Args,
    start: DateTime<Utc>,
    generated_level: &mut Option<String>,
) -> io::Result<()> {
    let mut rng = StdRng::seed_from_u64(args.seed);
    let timestamp = start.format("%d/%b/%Y:%H:%M:%S +0000").to_string();
    let paths = [
        "/",
        "/index.html",
        "/api/users",
        "/api/orders",
        "/api/products",
        "/favicon.ico",
    ];
    let methods = ["GET", "GET", "GET", "GET", "POST", "PUT", "DELETE"];
    let statuses = [200, 200, 200, 200, 200, 200, 301, 304, 400, 404, 500];
    for _ in 0..args.total {
        let mut ip = (
            rng.gen_range(1..=223),
            rng.gen_range(0..=255),
            rng.gen_range(0..=255),
            rng.gen_range(1..=254),
        );
        let method = if matches!(args.log_type, LogType::NginxAccess) {
            nginx_method(&mut rng)
        } else if matches!(args.log_type, LogType::ApacheAccess) {
            apache_method(&mut rng)
        } else if matches!(args.log_type, LogType::DjangoRequest) {
            match rng.gen_range(0..100) {
                0..=64 => "GET",
                65..=89 => "POST",
                90..=93 => "PUT",
                94..=96 => "DELETE",
                97..=98 => "PATCH",
                _ => "HEAD",
            }
        } else if matches!(args.log_type, LogType::Fastapi) {
            match rng.gen_range(0..100) {
                0..=59 => "GET",
                60..=79 => "POST",
                80..=89 => "PUT",
                90..=94 => "DELETE",
                95..=97 => "PATCH",
                _ => "OPTIONS",
            }
        } else {
            methods[rng.gen_range(0..methods.len())]
        };
        if matches!(args.log_type, LogType::NginxAccess | LogType::ApacheAccess)
            && args.nginx.ip_ranges.is_empty()
        {
            let probability = if matches!(args.log_type, LogType::NginxAccess) {
                0.7
            } else {
                0.6
            };
            ip = web_address(&mut rng, probability);
        }
        if matches!(args.log_type, LogType::DjangoRequest) {
            ip = web_address(&mut rng, 0.6);
        }
        if !args.nginx.ip_ranges.is_empty()
            && matches!(args.log_type, LogType::NginxAccess | LogType::ApacheAccess)
        {
            let (network, prefix) =
                args.nginx.ip_ranges[rng.gen_range(0..args.nginx.ip_ranges.len())];
            let mask = u32::MAX.checked_shl(u32::from(32 - prefix)).unwrap_or(0);
            let address = (u32::from(network) & mask) | (rng.r#gen::<u32>() & !mask);
            let octets = address.to_be_bytes();
            ip = (
                u32::from(octets[0]),
                u32::from(octets[1]),
                u32::from(octets[2]),
                u32::from(octets[3]),
            );
        }
        let dynamic_path;
        let path = if !args.nginx.url_patterns.is_empty()
            && matches!(
                args.log_type,
                LogType::NginxAccess | LogType::ApacheAccess | LogType::DjangoRequest
            ) {
            &args.nginx.url_patterns[rng.gen_range(0..args.nginx.url_patterns.len())]
        } else if matches!(args.log_type, LogType::NginxAccess) {
            let defaults = [
                "/",
                "/index.html",
                "/api/users",
                "/api/orders",
                "/api/products",
                "/static/css/style.css",
                "/static/js/app.js",
                "/images/logo.png",
                "/favicon.ico",
                "/robots.txt",
                "/sitemap.xml",
                "/admin/login",
                "/user/profile",
                "/search",
                "/contact",
                "/about",
            ];
            if rng.gen_bool(0.8) {
                defaults[rng.gen_range(0..defaults.len())]
            } else {
                let words = ["service", "account", "report", "search", "product", "order"];
                dynamic_path = match rng.gen_range(0..5) {
                    0 => format!("/api/users/{}", rng.gen_range(1..=10000)),
                    1 => format!("/products/{}", words[rng.gen_range(0..words.len())]),
                    2 => format!("/search?q={}", words[rng.gen_range(0..words.len())]),
                    3 => format!("/category/{}", words[rng.gen_range(0..words.len())]),
                    _ => format!("/page/{}", rng.gen_range(1..=100)),
                };
                &dynamic_path
            }
        } else if matches!(args.log_type, LogType::ApacheAccess) {
            let defaults = [
                "/",
                "/index.html",
                "/index.php",
                "/wp-admin/",
                "/wp-login.php",
                "/wp-content/themes/style.css",
                "/wp-content/uploads/image.jpg",
                "/cgi-bin/script.cgi",
                "/phpmyadmin/",
                "/admin/",
                "/images/logo.png",
                "/css/style.css",
                "/js/script.js",
                "/favicon.ico",
                "/robots.txt",
                "/sitemap.xml",
            ];
            if rng.gen_bool(0.8) {
                defaults[rng.gen_range(0..defaults.len())]
            } else {
                let words = ["service", "account", "report", "search", "product", "order"];
                dynamic_path = match rng.gen_range(0..6) {
                    0 => format!("/user/{}", rng.gen_range(1..=10000)),
                    1 => format!("/product/{}", words[rng.gen_range(0..words.len())]),
                    2 => format!("/search.php?q={}", words[rng.gen_range(0..words.len())]),
                    3 => format!("/category/{}", words[rng.gen_range(0..words.len())]),
                    4 => format!("/page.php?id={}", rng.gen_range(1..=100)),
                    _ => format!(
                        "/wp-content/uploads/{}.jpg",
                        words[rng.gen_range(0..words.len())]
                    ),
                };
                &dynamic_path
            }
        } else if matches!(args.log_type, LogType::DjangoRequest) {
            let defaults = [
                "/",
                "/admin/",
                "/admin/login/",
                "/admin/logout/",
                "/api/users/",
                "/api/auth/login/",
                "/api/auth/logout/",
                "/accounts/login/",
                "/accounts/logout/",
                "/accounts/profile/",
                "/static/admin/css/base.css",
                "/static/admin/js/admin.js",
                "/media/uploads/image.jpg",
                "/favicon.ico",
                "/robots.txt",
            ];
            if rng.gen_bool(0.8) {
                defaults[rng.gen_range(0..defaults.len())]
            } else {
                dynamic_path = match rng.gen_range(0..6) {
                    0 => format!("/api/users/{}/", rng.gen_range(1..=10000)),
                    1 => format!("/blog/post-{}/", rng.gen_range(1..=10000)),
                    2 => format!("/products/{}/", rng.gen_range(1..=1000)),
                    3 => format!("/admin/item/{}/change/", rng.gen_range(1..=100)),
                    4 => format!("/search/?q=product{}", rng.gen_range(1..=10000)),
                    _ => format!("/category/category-{}/", rng.gen_range(1..=10000)),
                };
                &dynamic_path
            }
        } else {
            paths[rng.gen_range(0..paths.len())]
        };
        let status = if args.nginx.status_codes.is_empty()
            || !matches!(
                args.log_type,
                LogType::NginxAccess | LogType::ApacheAccess | LogType::DjangoRequest
            ) {
            if matches!(args.log_type, LogType::NginxAccess) {
                nginx_status(&mut rng)
            } else if matches!(args.log_type, LogType::ApacheAccess) {
                apache_status(&mut rng)
            } else {
                statuses[rng.gen_range(0..statuses.len())]
            }
        } else {
            let weights =
                crate::weights::checked(args.nginx.status_codes.iter().map(|(_, weight)| *weight))
                    .map_err(io::Error::other)?;
            args.nginx.status_codes[weights.sample(&mut rng)].0
        };
        let size = if matches!(args.log_type, LogType::NginxAccess) {
            match status {
                404 => rng.gen_range(150..=500),
                500.. => rng.gen_range(100..=300),
                301 | 302 => rng.gen_range(200..=400),
                _ => rng.gen_range(500..=50000),
            }
        } else if matches!(args.log_type, LogType::ApacheAccess) {
            match status {
                404 => rng.gen_range(200..=600),
                500.. => rng.gen_range(150..=400),
                301 | 302 => rng.gen_range(250..=500),
                _ => rng.gen_range(800..=75000),
            }
        } else {
            rng.gen_range(0..=65535)
        };
        if !matches!(args.log_type, LogType::NginxAccess | LogType::ApacheAccess) {
            let time = start.format("%Y-%m-%d %H:%M:%S%.3f");
            let iso = start.format("%Y-%m-%dT%H:%M:%S%.3fZ");
            let line = match args.log_type {
                LogType::NginxError | LogType::ApacheError => {
                    let nginx = matches!(args.log_type, LogType::NginxError);
                    let default_levels = if nginx {
                        &[
                            ("error", 0.48),
                            ("warn", 0.25),
                            ("notice", 0.15),
                            ("info", 0.08),
                            ("crit", 0.02),
                            ("alert", 0.01),
                            ("emerg", 0.01),
                        ][..]
                    } else {
                        &[
                            ("error", 0.45),
                            ("warn", 0.30),
                            ("notice", 0.15),
                            ("info", 0.05),
                            ("crit", 0.03),
                            ("alert", 0.01),
                            ("emerg", 0.01),
                        ][..]
                    };
                    let level = if args.settings.error_levels.is_empty() {
                        compat::choose_weighted(&[], default_levels, &mut rng)
                            .map_err(io::Error::other)?
                    } else {
                        compat::choose_weighted(&args.settings.error_levels, &[], &mut rng)
                            .map_err(io::Error::other)?
                    };
                    let critical = if nginx {
                        &[
                            "worker process died unexpectedly",
                            "could not open error log file",
                            "malloc() failed",
                            "mmap() failed",
                        ][..]
                    } else {
                        &[
                            "server is within MinSpareServers of MaxRequestWorkers, consider raising the MaxRequestWorkers setting",
                            "caught SIGTERM, shutting down",
                            "aborting: unable to determine if daemon is running",
                            "No space left on device: mod_rewrite: could not create rewrite_log_lock",
                        ][..]
                    };
                    let message =
                        if ["crit", "alert"].contains(&level) || (!nginx && level == "emerg") {
                            critical[rng.gen_range(0..critical.len())]
                        } else if !args.settings.error_patterns.is_empty() {
                            compat::choose(&args.settings.error_patterns, "", &mut rng)
                        } else {
                            crate::patterns::choose(
                                if nginx {
                                    "NginxErrorLogGenerator.ERROR_PATTERNS"
                                } else {
                                    "ApacheErrorLogGenerator.ERROR_PATTERNS"
                                },
                                &mut rng,
                            )
                        };
                    let pid = rng.gen_range(1000..=99999);
                    if nginx {
                        let hosts = [
                            "api.example.test",
                            "web.example.test",
                            "example.test",
                            "service.example.test",
                        ];
                        let server = hosts[rng.gen_range(0..hosts.len())];
                        let host = hosts[rng.gen_range(0..hosts.len())];
                        let error_paths = [
                            "/",
                            "/admin",
                            "/api/users",
                            "/nonexistent",
                            "/favicon.ico",
                            "/robots.txt",
                        ];
                        let path = error_paths[rng.gen_range(0..error_paths.len())];
                        format!(
                            "{} [{level}] {pid}#{}: *{} {message}, client: {}.{}.{}.{}, server: {server}, request: \"GET {path} HTTP/1.1\", host: \"{host}\"",
                            start.format("%Y/%m/%d %H:%M:%S"),
                            rng.gen_range(0..=999),
                            rng.gen_range(1..=99999),
                            ip.0,
                            ip.1,
                            ip.2,
                            ip.3
                        )
                    } else {
                        let modules = [
                            "core",
                            "mod_ssl",
                            "mod_rewrite",
                            "mod_php",
                            "mod_fcgid",
                            "mod_security",
                            "mod_dir",
                            "mod_auth",
                        ];
                        let module = modules[rng.gen_range(0..modules.len())];
                        format!(
                            "[{}] [{level}] [{module}:error] [pid {pid}] [client {}.{}.{}.{}] {message}",
                            start.format("%a %b %d %H:%M:%S%.6f %Y"),
                            ip.0,
                            ip.1,
                            ip.2,
                            ip.3
                        )
                    }
                }
                LogType::DjangoRequest => {
                    let level = compat::choose_weighted(
                        &args.settings.django_levels,
                        &[
                            ("INFO", 0.60),
                            ("WARNING", 0.25),
                            ("ERROR", 0.10),
                            ("DEBUG", 0.04),
                            ("CRITICAL", 0.01),
                        ],
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    let component = compat::choose(
                        &args.settings.django_components,
                        "django.request",
                        &mut rng,
                    );
                    if component == "django.db.backends" {
                        let query = compat::choose(
                            &args.settings.django_sql_patterns,
                            "SELECT * FROM users WHERE id = %s",
                            &mut rng,
                        );
                        format!(
                            "{} DEBUG django.db.backends: ({:.3}) {}; args=({},)",
                            start.format("%Y-%m-%d %H:%M:%S,%3f"),
                            rng.gen_range(0.001..1.0),
                            query.trim_end_matches(';'),
                            rng.gen_range(1..=1000)
                        )
                    } else if component == "django.security" {
                        format!(
                            "{} {level} django.security: Suspicious request from {}.{}.{}.{}",
                            start.format("%Y-%m-%d %H:%M:%S,%3f"),
                            ip.0,
                            ip.1,
                            ip.2,
                            ip.3
                        )
                    } else if level == "ERROR" {
                        let exceptions = [
                            "DoesNotExist: User matching query does not exist.",
                            "ValueError: invalid literal for int() with base 10: 'abc'",
                            "KeyError: 'required_field'",
                            "AttributeError: 'NoneType' object has no attribute 'name'",
                            "ValidationError: This field is required.",
                            "PermissionDenied: You do not have permission to perform this action.",
                        ];
                        let source_lines = [
                            "return self.get_object().name",
                            "user = User.objects.get(id=user_id)",
                            "result = some_function(param)",
                            "data = json.loads(request.body)",
                            "return render(request, template_name, context)",
                        ];
                        format!(
                            "{} ERROR django.request: Internal Server Error: {}\nTraceback (most recent call last):\n  File \"/app/views.py\", line {}, in view\n    {}\n{}",
                            start.format("%Y-%m-%d %H:%M:%S,%3f"),
                            path,
                            rng.gen_range(10..=200),
                            source_lines[rng.gen_range(0..source_lines.len())],
                            exceptions[rng.gen_range(0..exceptions.len())]
                        )
                    } else {
                        let status = if args.nginx.status_codes.is_empty() {
                            let distribution = [
                                (200, 0.70),
                                (404, 0.12),
                                (302, 0.08),
                                (500, 0.03),
                                (403, 0.02),
                                (400, 0.02),
                                (401, 0.02),
                                (301, 0.01),
                            ];
                            let weights = crate::weights::checked(
                                distribution.iter().map(|(_, weight)| *weight),
                            )
                            .map_err(io::Error::other)?;
                            distribution[weights.sample(&mut rng)].0
                        } else {
                            status
                        };
                        let user = if rng.gen_bool(0.7) {
                            "AnonymousUser".to_owned()
                        } else {
                            format!("user{}", rng.gen_range(1..=10000))
                        };
                        let response_time = if rng.gen_bool(0.8) {
                            rng.gen_range(10..=200)
                        } else if rng.gen_bool(0.95) {
                            rng.gen_range(200..=1000)
                        } else {
                            rng.gen_range(1000..=5000)
                        };
                        format!(
                            "{} {} django.request: \"{} {}\" {} [{}ms] (user: {}, ip: {}.{}.{}.{})",
                            start.format("%Y-%m-%d %H:%M:%S,%3f"),
                            level,
                            method,
                            path,
                            status,
                            response_time,
                            user,
                            ip.0,
                            ip.1,
                            ip.2,
                            ip.3
                        )
                    }
                }
                LogType::DjangoSql => {
                    let models = [
                        "User",
                        "Group",
                        "Permission",
                        "Session",
                        "ContentType",
                        "LogEntry",
                        "Product",
                        "Order",
                        "Category",
                        "Article",
                        "Comment",
                    ];
                    let model = if args.settings.django_models.is_empty() {
                        models[rng.gen_range(0..models.len())]
                    } else {
                        &args.settings.django_models
                            [rng.gen_range(0..args.settings.django_models.len())]
                    };
                    let table = format!("app_{}", model.to_ascii_lowercase());
                    let query_type = compat::choose_weighted(
                        &[],
                        &[
                            ("SELECT", 0.70),
                            ("INSERT", 0.15),
                            ("UPDATE", 0.10),
                            ("DELETE", 0.05),
                        ],
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    let query = match query_type {
                        "SELECT" if rng.gen_bool(0.3) => {
                            format!("SELECT * FROM \"{table}\" WHERE \"id\" = %s")
                        }
                        "SELECT" if rng.gen_bool(0.6) => format!(
                            "SELECT \"id\", \"name\", \"created_at\" FROM \"{table}\" WHERE \"active\" = %s ORDER BY \"created_at\" DESC"
                        ),
                        "SELECT" => format!(
                            "SELECT * FROM \"{table}\" INNER JOIN \"app_category\" ON (\"{table}\".\"category_id\" = \"app_category\".\"id\") WHERE \"app_category\".\"name\" = %s"
                        ),
                        "INSERT" => format!(
                            "INSERT INTO \"{table}\" (\"name\", \"email\", \"created_at\") VALUES (%s, %s, %s)"
                        ),
                        "UPDATE" => format!(
                            "UPDATE \"{table}\" SET \"name\" = %s, \"updated_at\" = %s WHERE \"id\" = %s"
                        ),
                        _ => format!("DELETE FROM \"{table}\" WHERE \"id\" = %s"),
                    };
                    let id = rng.gen_range(1..=10000);
                    let timestamp = start.to_rfc3339();
                    let arguments = match query_type {
                        "SELECT" if rng.gen_bool(0.5) => "(True,)".to_owned(),
                        "INSERT" => {
                            format!("('User {id}', 'user{id}@example.test', '{timestamp}')")
                        }
                        "UPDATE" => format!("('User {id}', '{timestamp}', {id})"),
                        _ => format!("({id},)"),
                    };
                    let duration = if rng.gen_bool(0.8) {
                        rng.gen_range(0.001..0.050)
                    } else if rng.gen_bool(0.95) {
                        rng.gen_range(0.050..0.500)
                    } else {
                        rng.gen_range(0.500..2.000)
                    };
                    format!(
                        "{} DEBUG django.db.backends: ({duration:.3}) {query}; args={arguments}",
                        start.format("%Y-%m-%d %H:%M:%S,%3f"),
                    )
                }
                LogType::Docker => {
                    let level = compat::choose_weighted(
                        &args.settings.docker_levels,
                        &[
                            ("info", 0.50),
                            ("warn", 0.25),
                            ("error", 0.15),
                            ("debug", 0.08),
                            ("fatal", 0.02),
                        ],
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    *generated_level = Some(level.to_ascii_uppercase());
                    let group = if level == "error" || level == "fatal" {
                        "DockerLogGenerator.ERROR_PATTERNS"
                    } else {
                        "DockerLogGenerator.APP_LOG_PATTERNS"
                    };
                    let container = args.settings.docker_name.as_deref().unwrap_or("web-app");
                    let custom = args
                        .settings
                        .docker_patterns
                        .iter()
                        .filter(|(prefix, _)| {
                            container == prefix.as_str()
                                || container
                                    .strip_prefix(prefix.as_str())
                                    .is_some_and(|tail| tail.starts_with('-'))
                        })
                        .max_by_key(|(prefix, _)| prefix.len())
                        .map(|(_, patterns)| compat::choose(patterns, "", &mut rng));
                    let template =
                        custom.unwrap_or_else(|| crate::patterns::choose(group, &mut rng));
                    let values = [
                        ("port", rng.gen_range(3000..=9000).to_string()),
                        ("method", method.to_owned()),
                        ("path", path.to_owned()),
                        ("query", "SELECT * FROM users".to_owned()),
                        ("client_ip", format!("{}.{}.{}.{}", ip.0, ip.1, ip.2, ip.3)),
                        ("client_port", rng.gen_range(1024..=65535).to_string()),
                        ("host", format!("{}.{}.{}.{}", ip.0, ip.1, ip.2, ip.3)),
                        ("ip", format!("{}.{}.{}.{}", ip.0, ip.1, ip.2, ip.3)),
                        ("request_id", format!("{:08x}", rng.r#gen::<u32>())),
                        ("job_id", format!("{:08x}", rng.r#gen::<u32>())),
                        ("user", format!("user{}", rng.gen_range(1..=10000))),
                        ("key", format!("cache:item:{}", rng.gen_range(1..=1000))),
                        ("duration", rng.gen_range(10..=5000).to_string()),
                        (
                            "service",
                            ["database", "cache", "api", "auth"][rng.gen_range(0..4)].to_owned(),
                        ),
                        (
                            "config_file",
                            format!("/app/config/config-{}.yml", rng.gen_range(1..=1000)),
                        ),
                        (
                            "endpoint",
                            if group == "DockerLogGenerator.ERROR_PATTERNS" {
                                format!("/api/item{}", rng.gen_range(1..=1000))
                            } else {
                                format!(
                                    "http://service{}.example.test/metrics",
                                    rng.gen_range(1..=1000)
                                )
                            },
                        ),
                        ("pool_size", rng.gen_range(5..=50).to_string()),
                        ("size", format!("{}MB", rng.gen_range(1..=100))),
                        (
                            "file",
                            format!("/app/data/file-{}.log", rng.gen_range(1..=1000)),
                        ),
                        (
                            "error",
                            ["missing required field", "invalid format", "parse error"]
                                [rng.gen_range(0..3)]
                            .to_owned(),
                        ),
                        ("available", format!("{}GB", rng.gen_range(1..=10))),
                        (
                            "domain",
                            format!("service{}.example.test", rng.gen_range(1..=1000)),
                        ),
                    ];
                    let message =
                        crate::patterns::render(template, &values).map_err(io::Error::other)?;
                    if args.settings.docker_driver.as_deref() == Some("syslog") {
                        let severity = match level {
                            "fatal" => "crit",
                            "error" => "err",
                            "warn" => "warning",
                            "debug" => "debug",
                            _ => "info",
                        };
                        let pri = compat::syslog_priority("daemon", severity)
                            .map_err(io::Error::other)?;
                        format!("<{pri}>1 {iso} server {container} 1234 - - {message}")
                    } else if args.settings.docker_driver.as_deref() == Some("journald") {
                        serde_json::json!({"MESSAGE": message, "CONTAINER_NAME": container, "CONTAINER_ID": args.settings.docker_container_id.chars().take(12).collect::<String>(), "CONTAINER_ID_FULL": args.settings.docker_container_id, "SYSLOG_IDENTIFIER": container, "__REALTIME_TIMESTAMP": start.timestamp_micros().to_string()}).to_string()
                    } else if args.settings.docker_driver.as_deref() == Some("json-file")
                        || args.settings.docker_format.as_deref().unwrap_or("json") == "json"
                    {
                        #[derive(serde::Serialize)]
                        struct DockerRecord<'a> {
                            log: String,
                            stream: &'a str,
                            time: String,
                        }
                        let stream = if rng.gen_bool(0.7) {
                            "stdout"
                        } else {
                            "stderr"
                        };
                        serde_json::to_string(&DockerRecord {
                            log: format!("{message}\n"),
                            stream,
                            time: iso.to_string(),
                        })
                        .map_err(io::Error::other)?
                    } else {
                        format!(
                            "{} [{}] {}: {}",
                            iso,
                            level.to_uppercase(),
                            container,
                            message
                        )
                    }
                }
                LogType::KubernetesPod => {
                    let level = compat::choose_weighted(
                        &args.settings.kubernetes_levels,
                        &[
                            ("INFO", 0.50),
                            ("WARN", 0.25),
                            ("ERROR", 0.15),
                            ("DEBUG", 0.08),
                            ("FATAL", 0.02),
                        ],
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    let namespace = args
                        .settings
                        .kubernetes_namespace
                        .as_deref()
                        .unwrap_or("default");
                    let pod = args
                        .settings
                        .kubernetes_pod_name
                        .as_deref()
                        .unwrap_or("web-app-123");
                    let group = if level == "ERROR" || level == "FATAL" {
                        "KubernetesPodLogGenerator.ERROR_PATTERNS"
                    } else {
                        "KubernetesPodLogGenerator.APP_LOG_PATTERNS"
                    };
                    let template = crate::patterns::choose(group, &mut rng);
                    let values = [
                        (
                            "service",
                            ["api", "database", "cache", "auth"][rng.gen_range(0..4)].to_owned(),
                        ),
                        ("port", rng.gen_range(3000..=9000).to_string()),
                        (
                            "probe_type",
                            ["liveness", "readiness", "startup"][rng.gen_range(0..3)].to_owned(),
                        ),
                        (
                            "endpoint",
                            format!("http://service{}-service:8080", rng.gen_range(1..=1000)),
                        ),
                        ("request_id", format!("{:08x}", rng.r#gen::<u32>())),
                        ("namespace", namespace.to_owned()),
                        (
                            "configmap",
                            format!("app{}-config", rng.gen_range(1..=1000)),
                        ),
                        ("secret", format!("app{}-secret", rng.gen_range(1..=1000))),
                        ("pv", format!("pv-{:08x}", rng.r#gen::<u32>())),
                        (
                            "mount_path",
                            format!("/mnt/data{}", rng.gen_range(1..=1000)),
                        ),
                        ("count", rng.gen_range(1..=10).to_string()),
                        (
                            "host",
                            format!("service{}.example.test", rng.gen_range(1..=1000)),
                        ),
                        ("pod", pod.to_owned()),
                        ("image", format!("app{}:latest", rng.gen_range(1..=1000))),
                        (
                            "container",
                            args.settings
                                .kubernetes_container_name
                                .as_deref()
                                .unwrap_or("app")
                                .to_owned(),
                        ),
                        (
                            "exit_code",
                            [1, 2, 125, 126, 127, 130, 137, 143][rng.gen_range(0..8)].to_string(),
                        ),
                        ("source", format!("app{}-pod", rng.gen_range(1..=1000))),
                        (
                            "destination",
                            format!("app{}-service", rng.gen_range(1..=1000)),
                        ),
                        (
                            "cert",
                            format!("service{}.example.test-tls", rng.gen_range(1..=1000)),
                        ),
                    ];
                    let message =
                        crate::patterns::render(template, &values).map_err(io::Error::other)?;
                    format!("{iso} {level} {message}")
                }
                LogType::KubernetesEvent => {
                    if !args.settings.kubernetes_event_patterns.is_empty()
                        || !args.settings.kubernetes_pod_patterns.is_empty()
                    {
                        let pod_template = compat::choose(
                            &args.settings.kubernetes_pod_patterns,
                            "web-app-{replica_id}",
                            &mut rng,
                        );
                        let pod = crate::patterns::render(
                            pod_template,
                            &[
                                ("deployment", "app".into()),
                                (
                                    "replica_id",
                                    format!("{:05x}", rng.gen_range(0..=0xfffff_u32)),
                                ),
                            ],
                        )
                        .map_err(io::Error::other)?;
                        let template = compat::choose(
                            &args.settings.kubernetes_event_patterns,
                            "Pod {pod_name} scheduled to node {node_name}",
                            &mut rng,
                        );
                        let namespace = compat::choose(
                            &args.settings.kubernetes_namespaces,
                            "default",
                            &mut rng,
                        );
                        let node = format!("node-{}", rng.gen_range(1..=5));
                        let message = crate::patterns::render(
                            template,
                            &[
                                ("pod_name", pod.clone()),
                                ("node_name", node),
                                ("container_name", "app".into()),
                            ],
                        )
                        .map_err(io::Error::other)?;
                        let lower = message.to_ascii_lowercase();
                        let event = if lower.contains("failed") || lower.contains("evicted") {
                            "Warning"
                        } else {
                            "Normal"
                        };
                        let reason = if lower.contains("evicted") {
                            "Evicted"
                        } else if lower.contains("failed") {
                            "Unhealthy"
                        } else if lower.contains("scheduled") {
                            "Scheduled"
                        } else if lower.contains("started") {
                            "Started"
                        } else {
                            "Custom"
                        };
                        format!("{iso} {event} {reason} Pod/{namespace}/{pod} {message}")
                    } else {
                        let event = compat::choose_weighted(
                            &args.settings.kubernetes_event_types,
                            &[("Normal", 0.7), ("Warning", 0.3)],
                            &mut rng,
                        )
                        .map_err(io::Error::other)?;
                        let reasons = if event == "Warning" {
                            &[
                                "Failed",
                                "FailedScheduling",
                                "FailedMount",
                                "FailedAttachVolume",
                                "Unhealthy",
                                "BackOff",
                                "FailedCreatePodSandBox",
                                "NetworkNotReady",
                                "FreeDiskSpaceFailed",
                            ][..]
                        } else {
                            &[
                                "Scheduled",
                                "Pulled",
                                "Created",
                                "Started",
                                "Killing",
                                "SuccessfulMountVolume",
                                "SuccessfulAttachVolume",
                                "NodeReady",
                                "NodeNotReady",
                                "Sync",
                            ][..]
                        };
                        let reason = reasons[rng.gen_range(0..reasons.len())];
                        let kinds = [
                            "Pod",
                            "Node",
                            "Service",
                            "Deployment",
                            "ReplicaSet",
                            "ConfigMap",
                            "Secret",
                            "PersistentVolume",
                            "PersistentVolumeClaim",
                        ];
                        let kind = kinds[rng.gen_range(0..kinds.len())];
                        let name = match kind {
                            "Pod" => format!(
                                "{}-{:010x}",
                                ["web-app", "api-server", "database"][rng.gen_range(0..3)],
                                rng.gen_range(0..(1_u64 << 40))
                            ),
                            "Node" => format!("node-{}", rng.gen_range(1..=10)),
                            "Service" | "Deployment" => format!(
                                "app{}-{}",
                                rng.gen_range(1..=1000),
                                kind.to_ascii_lowercase()
                            ),
                            _ => {
                                format!("app{}-{:08x}", rng.gen_range(1..=1000), rng.r#gen::<u32>())
                            }
                        };
                        let message = match reason {
                            "Scheduled" => format!(
                                "Successfully assigned {name} to node-{}",
                                rng.gen_range(1..=10)
                            ),
                            "Pulled" => format!(
                                "Container image \"app{}:latest\" already present on machine",
                                rng.gen_range(1..=1000)
                            ),
                            "Created" => {
                                format!("Created container app{}", rng.gen_range(1..=1000))
                            }
                            "Started" => {
                                format!("Started container app{}", rng.gen_range(1..=1000))
                            }
                            "Killing" => {
                                format!("Stopping container app{}", rng.gen_range(1..=1000))
                            }
                            "Failed" => format!(
                                "Error: {}",
                                [
                                    "ImagePullBackOff",
                                    "CrashLoopBackOff",
                                    "CreateContainerConfigError"
                                ][rng.gen_range(0..3)]
                            ),
                            "FailedScheduling" => format!(
                                "0/3 nodes are available: 3 Insufficient {}",
                                ["cpu", "memory"][rng.gen_range(0..2)]
                            ),
                            "FailedMount" => format!(
                                "Unable to mount volumes for pod \"{name}\": timeout expired waiting for volumes to attach or mount"
                            ),
                            "Unhealthy" => format!(
                                "{} probe failed: {}",
                                ["Liveness", "Readiness"][rng.gen_range(0..2)],
                                ["HTTP probe failed", "Get timeout"][rng.gen_range(0..2)]
                            ),
                            _ if event == "Warning" => {
                                format!("{reason}: operation failed with error")
                            }
                            _ => format!("{reason} operation completed successfully"),
                        };
                        format!("{iso} {event} {reason} {kind}/{name} {message}")
                    }
                }
                LogType::MysqlQuery => {
                    let connection = rng.gen_range(1..=10000);
                    if rng.gen_bool(0.8) {
                        let query_type = compat::choose_weighted(
                            &args.settings.database_queries,
                            &[
                                ("SELECT", 0.60),
                                ("INSERT", 0.15),
                                ("UPDATE", 0.12),
                                ("DELETE", 0.08),
                                ("SHOW", 0.03),
                                ("DESCRIBE", 0.02),
                            ],
                            &mut rng,
                        )
                        .map_err(io::Error::other)?;
                        let table =
                            database_table(&args.settings.database_tables, MYSQL_TABLES, &mut rng);
                        let value = rng.gen_range(1..=1000);
                        let query = database_sql(
                            query_type,
                            table,
                            value,
                            &args.settings.database_tables,
                            &mut rng,
                        );
                        format!(
                            "{} {connection} Query {query}",
                            start.format("%y%m%d %H:%M:%S")
                        )
                    } else {
                        let (command, detail) = match rng.gen_range(0..7) {
                            0 => ("Connect", "app@localhost"),
                            1 => ("Quit", "connection closed"),
                            2 => (
                                "Init DB",
                                ["test", "production", "development"][rng.gen_range(0..3)],
                            ),
                            3 => (
                                "Field List",
                                database_table(
                                    &args.settings.database_tables,
                                    MYSQL_TABLES,
                                    &mut rng,
                                ),
                            ),
                            4 => ("Refresh", "server refreshed"),
                            5 => ("Statistics", "server statistics"),
                            _ => ("Ping", "server alive"),
                        };
                        format!(
                            "{} {connection} {command} {detail}",
                            start.format("%y%m%d %H:%M:%S")
                        )
                    }
                }
                LogType::MysqlError => {
                    let level = compat::choose_weighted(
                        &args.settings.database_levels,
                        &[("Note", 0.5), ("Warning", 0.3), ("Error", 0.2)],
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    let group = match level {
                        "Error" => "MySQLErrorLogGenerator.ERROR_PATTERNS",
                        "Warning" => "MySQLErrorLogGenerator.WARNING_PATTERNS",
                        _ => "MySQLErrorLogGenerator.NOTE_PATTERNS",
                    };
                    let template = crate::patterns::choose(group, &mut rng);
                    let values = [
                        ("user", format!("user{}", rng.gen_range(1..=10000))),
                        ("host", format!("{}.{}.{}.{}", ip.0, ip.1, ip.2, ip.3)),
                        ("password", ["YES", "NO"][rng.gen_range(0..2)].to_owned()),
                        (
                            "database",
                            ["test", "production", "app"][rng.gen_range(0..3)].to_owned(),
                        ),
                        (
                            "table",
                            ["users", "orders", "products"][rng.gen_range(0..3)].to_owned(),
                        ),
                        (
                            "column",
                            ["id", "name", "email", "status"][rng.gen_range(0..4)].to_owned(),
                        ),
                        ("value", rng.gen_range(1..=1000).to_string()),
                        ("key", "PRIMARY".to_owned()),
                        ("row", rng.gen_range(1..=100).to_string()),
                        (
                            "type",
                            ["integer", "datetime", "varchar"][rng.gen_range(0..3)].to_owned(),
                        ),
                        ("count", rng.gen_range(1..=100).to_string()),
                        (
                            "hostname",
                            format!("db{}.example.test", rng.gen_range(1..=1000)),
                        ),
                        ("ip", format!("{}.{}.{}.{}", ip.0, ip.1, ip.2, ip.3)),
                        (
                            "log",
                            format!("mysql-bin.{}", rng.gen_range(100000..=999999)),
                        ),
                        ("new_value", rng.gen_range(1000..=5000).to_string()),
                        ("old_value", rng.gen_range(100..=1000).to_string()),
                        ("pid_file", "/var/run/mysqld/mysqld.pid".to_owned()),
                        ("position", rng.gen_range(1000..=99999).to_string()),
                        ("timestamp", start.format("%Y-%m-%d %H:%M:%S").to_string()),
                        ("lsn", rng.gen_range(1000000..=9999999).to_string()),
                        ("pages", rng.gen_range(1000..=50000).to_string()),
                        ("port", [3306, 3307, 3308][rng.gen_range(0..3)].to_string()),
                        ("socket", "/var/run/mysqld/mysqld.sock".to_owned()),
                        ("version", format!("8.0.{}", rng.gen_range(20..=35))),
                    ];
                    let message =
                        crate::patterns::render(template, &values).map_err(io::Error::other)?;
                    format!("{iso} [{level}] {message}")
                }
                LogType::MysqlSlow => {
                    let duration = slow_query_duration(&mut rng);
                    let lock = rng.gen_range(0.0..1.0);
                    let rows_sent = rng.gen_range(0..=10000_u64);
                    let rows_examined = rng.gen_range(rows_sent..=rows_sent * 10);
                    let table =
                        database_table(&args.settings.database_tables, MYSQL_SLOW_TABLES, &mut rng);
                    let other =
                        database_table(&args.settings.database_tables, MYSQL_SLOW_TABLES, &mut rng);
                    let query_type = compat::choose_weighted(
                        &args.settings.database_queries,
                        &[
                            ("SELECT", 0.70),
                            ("UPDATE", 0.15),
                            ("DELETE", 0.10),
                            ("INSERT", 0.05),
                        ],
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    let query = slow_query(query_type, table, other, false, &mut rng);
                    let schema = ["test", "production", "app"][rng.gen_range(0..3)];
                    let user = format!("user{}", rng.gen_range(1..=1000));
                    format!(
                        "# Time: {iso}\n# User@Host: {user}[{user}] @ localhost []\n# Thread_id: {} Schema: {schema}\n# Query_time: {duration:.6} Lock_time: {lock:.6} Rows_sent: {rows_sent} Rows_examined: {rows_examined}\nSET timestamp={};\n{query};",
                        rng.gen_range(1..=1000),
                        start.timestamp()
                    )
                }
                LogType::Postgresql => {
                    let level = compat::choose_weighted(
                        &args.settings.database_levels,
                        &[
                            ("LOG", 0.40),
                            ("INFO", 0.25),
                            ("WARNING", 0.20),
                            ("ERROR", 0.10),
                            ("FATAL", 0.03),
                            ("PANIC", 0.02),
                        ],
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    let table =
                        database_table(&args.settings.database_tables, POSTGRES_TABLES, &mut rng);
                    let message = if ["ERROR", "FATAL", "PANIC"].contains(&level) {
                        postgres_error(level, table, &mut rng)
                    } else if rng.gen_bool(0.6) {
                        let query_type = compat::choose_weighted(
                            &args.settings.database_queries,
                            &[
                                ("SELECT", 0.55),
                                ("INSERT", 0.18),
                                ("UPDATE", 0.15),
                                ("DELETE", 0.07),
                                ("CREATE", 0.02),
                                ("DROP", 0.01),
                                ("ALTER", 0.02),
                            ],
                            &mut rng,
                        )
                        .map_err(io::Error::other)?;
                        let alternatives: Vec<&str> = if args.settings.database_tables.is_empty() {
                            POSTGRES_TABLES
                                .iter()
                                .copied()
                                .filter(|other| *other != table)
                                .collect()
                        } else {
                            args.settings
                                .database_tables
                                .iter()
                                .map(String::as_str)
                                .filter(|other| *other != table)
                                .collect()
                        };
                        let other = if alternatives.is_empty() {
                            table
                        } else {
                            alternatives[rng.gen_range(0..alternatives.len())]
                        };
                        postgres_statement(query_type, table, other, &mut rng)
                    } else {
                        postgres_admin(level, table, ip, &mut rng)
                    };
                    format!(
                        "{time} UTC [{}]: [{level}] {message}",
                        rng.gen_range(1000..=99999)
                    )
                }
                LogType::PostgresqlSlow => {
                    let table = database_table(
                        &args.settings.database_tables,
                        POSTGRES_SLOW_TABLES,
                        &mut rng,
                    );
                    let other = database_table(
                        &args.settings.database_tables,
                        POSTGRES_SLOW_TABLES,
                        &mut rng,
                    );
                    let duration = slow_query_duration(&mut rng) * 1000.0;
                    let query_type = compat::choose_weighted(
                        &args.settings.database_queries,
                        &[("SELECT", 0.75), ("UPDATE", 0.125), ("DELETE", 0.125)],
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    let query = slow_query(query_type, table, other, true, &mut rng);
                    format!(
                        "{} UTC [{}]: [LOG] duration: {:.3} ms statement: {query}",
                        time,
                        rng.gen_range(1000..=99999),
                        duration
                    )
                }
                LogType::Syslog => {
                    let facility = if args.settings.facilities.is_empty() {
                        compat::choose_weighted(
                            &[],
                            &[
                                ("kern", 0.15),
                                ("daemon", 0.20),
                                ("auth", 0.15),
                                ("mail", 0.05),
                                ("cron", 0.10),
                                ("syslog", 0.05),
                                ("user", 0.10),
                                ("local0", 0.05),
                                ("local1", 0.05),
                                ("local2", 0.05),
                                ("local3", 0.05),
                            ],
                            &mut rng,
                        )
                        .map_err(io::Error::other)?
                    } else {
                        compat::choose_weighted(&args.settings.facilities, &[], &mut rng)
                            .map_err(io::Error::other)?
                    };
                    let severity = if args.settings.severities.is_empty() {
                        compat::choose_weighted(
                            &[],
                            &[
                                ("info", 0.40),
                                ("notice", 0.25),
                                ("warning", 0.15),
                                ("err", 0.10),
                                ("debug", 0.05),
                                ("crit", 0.03),
                                ("alert", 0.01),
                                ("emerg", 0.01),
                            ],
                            &mut rng,
                        )
                        .map_err(io::Error::other)?
                    } else {
                        compat::choose_weighted(&args.settings.severities, &[], &mut rng)
                            .map_err(io::Error::other)?
                    };
                    let priority =
                        compat::syslog_priority(facility, severity).map_err(io::Error::other)?;
                    *generated_level = Some(
                        crate::json_output::syslog_level(u64::from(priority))
                            .ok_or_else(|| io::Error::other("invalid syslog priority"))?
                            .to_owned(),
                    );
                    let prefix = match args
                        .settings
                        .syslog_priority_style
                        .as_deref()
                        .unwrap_or("pri")
                    {
                        "pri" => format!("<{priority}>"),
                        "bracket" => format!("[{facility}.{severity}]"),
                        "plain" => format!("{facility}.{severity} "),
                        "none" => String::new(),
                        _ => return Err(io::Error::other("invalid syslog priority_style")),
                    };
                    let hostname = args.settings.syslog_hostname.as_deref().unwrap_or("server");
                    let client = format!("{}.{}.{}.{}", ip.0, ip.1, ip.2, ip.3);
                    let message = crate::patterns::syslog_message(
                        facility,
                        start,
                        &client,
                        args.settings
                            .syslog_message_patterns
                            .get(facility)
                            .map(Vec::as_slice),
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    let tags: &[&str] = match facility {
                        "kern" => &["kernel"],
                        "daemon" => &[
                            "systemd",
                            "NetworkManager",
                            "dbus",
                            "avahi-daemon",
                            "chronyd",
                        ],
                        "auth" => &["sshd", "sudo", "su", "login"],
                        "mail" => &[
                            "postfix/smtpd",
                            "postfix/cleanup",
                            "postfix/qmgr",
                            "dovecot",
                        ],
                        "cron" => &["cron", "CRON"],
                        "syslog" => &["rsyslogd"],
                        "user" => &["user_process"],
                        "local0" => &["local_app"],
                        "local1" => &["custom_daemon"],
                        "local2" => &["monitoring"],
                        "local3" => &["backup"],
                        _ => &[facility],
                    };
                    let tag = tags[rng.gen_range(0..tags.len())];
                    let process = if rng.gen_bool(0.7) {
                        Some(rng.gen_range(1000..=99999))
                    } else {
                        None
                    };
                    if args.settings.syslog_rfc.as_deref() == Some("5424") {
                        let ids: &[&str] = match facility {
                            "auth" => &["LOGIN", "LOGOUT", "FAILED_LOGIN", "SUDO"],
                            "mail" => &["CONNECT", "DISCONNECT", "DELIVERY", "BOUNCE"],
                            "cron" => &["JOB_START", "JOB_END", "JOB_FAILED"],
                            "daemon" => &["START", "STOP", "RELOAD", "ERROR"],
                            _ => &["INFO", "ERROR", "WARNING"],
                        };
                        let msgid = if rng.gen_bool(0.5) {
                            ids[rng.gen_range(0..ids.len())]
                        } else {
                            "-"
                        };
                        let process = process.map_or_else(|| "-".to_owned(), |pid| pid.to_string());
                        format!("{prefix}1 {iso} {hostname} {tag} {process} {msgid} - {message}")
                    } else {
                        let process = process.map_or_else(String::new, |pid| format!("[{pid}]"));
                        format!(
                            "{prefix}{} {hostname} {tag}{process}: {message}",
                            start.format("%b %d %H:%M:%S")
                        )
                    }
                }
                LogType::Fastapi => {
                    let level = compat::choose_weighted(
                        &args.settings.log_levels,
                        &[
                            ("INFO", 0.60),
                            ("DEBUG", 0.20),
                            ("WARNING", 0.15),
                            ("ERROR", 0.05),
                        ],
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    let user_number = rng.gen_range(1000..=99999);
                    let user_id = user_number.to_string();
                    let defaults = [
                        "/api/v1/users",
                        "/api/v1/users/{user_id}",
                        "/api/v1/auth/login",
                        "/api/v1/auth/logout",
                        "/api/v1/auth/refresh",
                        "/api/v1/products",
                        "/api/v1/products/{product_id}",
                        "/api/v1/orders",
                        "/api/v1/orders/{order_id}",
                        "/api/v1/health",
                        "/api/v1/metrics",
                        "/docs",
                        "/openapi.json",
                        "/api/v1/search",
                        "/api/v1/categories",
                        "/api/v1/upload",
                        "/api/v1/download/{file_id}",
                    ];
                    let dynamic_endpoint;
                    let selected_endpoint: &str = if !args.settings.endpoints.is_empty() {
                        &args.settings.endpoints[rng.gen_range(0..args.settings.endpoints.len())]
                    } else if rng.gen_bool(0.8) {
                        defaults[rng.gen_range(0..defaults.len())]
                    } else {
                        dynamic_endpoint = match rng.gen_range(0..4) {
                            0 => format!("/api/v1/users/{user_id}/orders"),
                            1 => format!("/api/v1/search?q=product{}", rng.gen_range(1..=10000)),
                            2 => {
                                format!("/api/v1/categories/category-{}", rng.gen_range(1..=10000))
                            }
                            _ => {
                                let date = start
                                    .date_naive()
                                    .checked_sub_days(chrono::Days::new(rng.gen_range(0..=365)))
                                    .ok_or_else(|| io::Error::other("report date overflow"))?;
                                format!("/api/v1/reports/{date}")
                            }
                        };
                        &dynamic_endpoint
                    };
                    let endpoint = selected_endpoint
                        .replace("{user_id}", &user_id)
                        .replace("{product_id}", &rng.gen_range(1..=10000).to_string())
                        .replace(
                            "{order_id}",
                            &format!("ORD-{}", rng.gen_range(100000..=999999)),
                        )
                        .replace("{file_id}", &format!("{:08x}", rng.r#gen::<u32>()));
                    let status = if args.settings.fastapi_status_codes.is_empty() {
                        let statuses = [
                            (200, 0.65),
                            (201, 0.10),
                            (400, 0.08),
                            (401, 0.05),
                            (404, 0.05),
                            (422, 0.03),
                            (500, 0.02),
                            (403, 0.01),
                            (429, 0.01),
                        ];
                        let weights =
                            crate::weights::checked(statuses.iter().map(|(_, weight)| *weight))
                                .map_err(io::Error::other)?;
                        statuses[weights.sample(&mut rng)].0
                    } else {
                        let weights = crate::weights::checked(
                            args.settings
                                .fastapi_status_codes
                                .iter()
                                .map(|(_, weight)| *weight),
                        )
                        .map_err(io::Error::other)?;
                        args.settings.fastapi_status_codes[weights.sample(&mut rng)].0
                    };
                    let response_time =
                        compat::fastapi_response_time(&args.settings, status, &mut rng)
                            .map_err(io::Error::other)?;
                    let message = if level == "INFO" && rng.gen_bool(0.7) {
                        format!(
                            "Request completed: {method} {endpoint} - {status} - {response_time}ms"
                        )
                    } else if ["INFO", "DEBUG", "WARNING", "ERROR"].contains(&level) {
                        let group = format!("FastAPILogGenerator.{level}");
                        let template = crate::patterns::choose(&group, &mut rng);
                        let deprecation_date = if template.contains("{date}") {
                            start
                                .date_naive()
                                .checked_add_days(chrono::Days::new(rng.gen_range(1..=365)))
                                .ok_or_else(|| io::Error::other("deprecation date overflow"))?
                                .to_string()
                        } else {
                            String::new()
                        };
                        let slow_query = if template.contains("{query}") {
                            let date = start
                                .date_naive()
                                .checked_sub_days(chrono::Days::new(rng.gen_range(0..=365)))
                                .ok_or_else(|| io::Error::other("query date overflow"))?;
                            format!("SELECT * FROM orders WHERE created_at > '{date}'")
                        } else {
                            String::new()
                        };
                        let errors = [
                            "Connection timeout",
                            "Invalid JSON format",
                            "Missing required field",
                            "Unauthorized access",
                            "Resource not found",
                            "Duplicate entry",
                            "Validation failed",
                            "Service unavailable",
                            "Rate limit exceeded",
                            "Internal processing error",
                        ];
                        let fields = ["email", "password", "username", "phone"];
                        let tasks = [
                            "send_email",
                            "process_payment",
                            "generate_report",
                            "backup_data",
                        ];
                        let values = [
                            ("method", method.to_owned()),
                            ("endpoint", endpoint.clone()),
                            ("status_code", status.to_string()),
                            ("response_time", response_time.to_string()),
                            ("user_id", user_id.clone()),
                            (
                                "query_type",
                                ["SELECT", "INSERT", "UPDATE", "DELETE"][rng.gen_range(0..4)]
                                    .to_owned(),
                            ),
                            ("duration", rng.gen_range(1..=1000).to_string()),
                            ("cache_key", format!("user:{user_id}:profile")),
                            (
                                "filename",
                                format!("upload-{}.txt", rng.gen_range(1..=10000)),
                            ),
                            ("file_size", rng.gen_range(1024..=10485760).to_string()),
                            ("email", format!("user{user_id}@example.test")),
                            ("task_name", tasks[rng.gen_range(0..tasks.len())].to_owned()),
                            ("ip", format!("{}.{}.{}.{}", ip.0, ip.1, ip.2, ip.3)),
                            ("metric_count", rng.gen_range(100..=10000).to_string()),
                            (
                                "headers",
                                r#"{"User-Agent":"FastAPI-Client/1.0","Content-Type":"application/json"}"#.to_owned(),
                            ),
                            ("request_body", serde_json::json!({"user_id": user_number, "action": "update"}).to_string()),
                            ("response_body", serde_json::json!({"status": "success", "data": {"id": rng.gen_range(1..=1000)}}).to_string()),
                            (
                                "sql_query",
                                format!(
                                    "SELECT * FROM users WHERE id = {}",
                                    rng.gen_range(1..=1000)
                                ),
                            ),
                            (
                                "field_name",
                                fields[rng.gen_range(0..fields.len())].to_owned(),
                            ),
                            ("field", fields[rng.gen_range(0..fields.len())].to_owned()),
                            (
                                "middleware_name",
                                ["AuthMiddleware", "CORSMiddleware", "RateLimitMiddleware"]
                                    [rng.gen_range(0..3)]
                                .to_owned(),
                            ),
                            (
                                "template_name",
                                ["user_profile.html", "dashboard.html", "email_template.html"]
                                    [rng.gen_range(0..3)]
                                .to_owned(),
                            ),
                            ("query", slow_query),
                            ("memory_usage", rng.gen_range(100..=2048).to_string()),
                            ("current_requests", rng.gen_range(80..=95).to_string()),
                            ("limit", "100".to_owned()),
                            ("size", format!("{:.2}", rng.gen_range(5.0..50.0))),
                            ("evicted_keys", rng.gen_range(10..=100).to_string()),
                            ("active", rng.gen_range(18..=20).to_string()),
                            ("max_connections", "20".to_owned()),
                            ("attempt", rng.gen_range(1..=3).to_string()),
                            ("max_attempts", "3".to_owned()),
                            (
                                "operation",
                                ["database_query", "api_call", "file_upload"][rng.gen_range(0..3)]
                                    .to_owned(),
                            ),
                            ("version", format!("v{}", rng.gen_range(1..=3))),
                            ("date", deprecation_date),
                            ("error", errors[rng.gen_range(0..errors.len())].to_owned()),
                            (
                                "reason",
                                ["invalid_credentials", "account_locked", "token_expired"]
                                    [rng.gen_range(0..3)]
                                .to_owned(),
                            ),
                            (
                                "error_message",
                                errors[rng.gen_range(0..errors.len())].to_owned(),
                            ),
                            ("api_url", "https://example.test/api/v1/data".to_owned()),
                            (
                                "order_id",
                                format!("ORD-{}", rng.gen_range(100000..=999999)),
                            ),
                        ];
                        crate::patterns::render(template, &values).map_err(io::Error::other)?
                    } else {
                        // The original generator uses a generic message for unlisted levels.
                        format!("Generic {} message", level.to_lowercase())
                    };
                    let request_id = if !args.settings.fastapi_no_request_id {
                        format!("{:08x}: ", rng.r#gen::<u32>())
                    } else {
                        String::new()
                    };
                    let user = if args.settings.fastapi_user_id {
                        format!(" user_id={user_id}")
                    } else {
                        String::new()
                    };
                    let correlation = if args.settings.fastapi_correlation_id {
                        format!(" correlation_id={:032x}", rng.r#gen::<u128>())
                    } else {
                        String::new()
                    };
                    format!(
                        "{} [{}] {request_id}{message}{user}{correlation}",
                        start.format("%Y-%m-%d %H:%M:%S,%3f"),
                        level
                    )
                }
                LogType::Goloop => {
                    let level = compat::choose_weighted(
                        &args.settings.goloop_levels,
                        &[
                            ("D", 0.60),
                            ("I", 0.25),
                            ("W", 0.10),
                            ("E", 0.04),
                            ("F", 0.01),
                        ],
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    let module = compat::choose_weighted(
                        &args.settings.goloop_modules,
                        &[
                            ("CS", 1.0),
                            ("BM", 1.0),
                            ("SV", 1.0),
                            ("NM", 1.0),
                            ("fastsync", 1.0),
                            ("TP", 1.0),
                            ("EE", 1.0),
                            ("PL", 1.0),
                            ("DB", 1.0),
                            ("IC", 1.0),
                            ("NT", 1.0),
                            ("ST", 1.0),
                            ("TX", 1.0),
                            ("SM", 1.0),
                            ("CH", 1.0),
                            ("RP", 1.0),
                            ("P2P", 1.0),
                        ],
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    let [minimum, maximum] =
                        args.settings.goloop_height_range.unwrap_or([1000, 999999]);
                    let height = rng.gen_range(minimum..=maximum);
                    let [revision_min, revision_max] =
                        args.settings.goloop_revision_range.unwrap_or([10, 25]);
                    let revision = rng.gen_range(revision_min..=revision_max);
                    let record = crate::goloop::generate(
                        module,
                        height,
                        &args.settings.goloop_network,
                        args.settings.goloop_network_id.as_deref().unwrap_or("0x1"),
                        revision,
                        &mut rng,
                    );
                    let timestamp = start.format("%Y%m%d-%H:%M:%S%.6f").to_string();
                    if args.settings.goloop_format.as_deref() == Some("json") {
                        serde_json::json!({"level": level, "timestamp": timestamp, "thread_id": "6df7", "sequence": record.sequence, "module": module, "sub_module": record.submodule, "source": record.source, "msg": record.message}).to_string()
                    } else {
                        let submodule = if record.submodule.is_empty() {
                            String::new()
                        } else {
                            format!("|{}", record.submodule)
                        };
                        let sequence = record.sequence.as_str().unwrap_or("1");
                        format!(
                            "{level}|{timestamp}|6df7|{sequence}|{module}{submodule}|{} {}",
                            record.source, record.message
                        )
                    }
                }
                LogType::Haproxy => {
                    let request_time = rng.gen_range(0..=20_u32);
                    let queue_time = rng.gen_range(0..=10_u32);
                    let connect_time = rng.gen_range(0..=25_u32);
                    let response_time = rng.gen_range(1..=150_u32);
                    let active_time = request_time + queue_time + connect_time + response_time;
                    let status = [200, 200, 200, 201, 301, 404, 500, 503][rng.gen_range(0..8)];
                    let termination = if status >= 500 { "SC--" } else { "----" };
                    format!(
                        "haproxy[{}]: {}.{}.{}.{}:{} [{}] http-in app/srv1 {request_time}/{queue_time}/{connect_time}/{response_time}/{active_time} {status} {} - - {termination} 1/1/1/1/0 0/0 \"{method} {path} HTTP/1.1\"",
                        rng.gen_range(1000..=99999),
                        ip.0,
                        ip.1,
                        ip.2,
                        ip.3,
                        rng.gen_range(1024..=65535),
                        start.format("%d/%b/%Y:%H:%M:%S%.3f"),
                        rng.gen_range(128..=65535),
                    )
                }
                LogType::KafkaBroker => {
                    let broker = if args.settings.kafka_broker_ids.is_empty() {
                        rng.gen_range(1..=3)
                    } else {
                        args.settings.kafka_broker_ids
                            [rng.gen_range(0..args.settings.kafka_broker_ids.len())]
                    };
                    let partition =
                        rng.gen_range(0..args.settings.kafka_partition_count.unwrap_or(12));
                    let defaults = ["orders", "payments", "users", "inventory"];
                    let topic = if args.settings.kafka_topics.is_empty() {
                        defaults[rng.gen_range(0..defaults.len())]
                    } else {
                        &args.settings.kafka_topics
                            [rng.gen_range(0..args.settings.kafka_topics.len())]
                    };
                    let level = compat::choose_weighted(
                        &args.settings.kafka_levels,
                        &[("INFO", 5.0), ("WARN", 2.0), ("ERROR", 1.0)],
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    let (message, logger) = match level {
                        "ERROR" => (
                            format!(
                                "[LogManager broker={broker}] Failed to write log segment for {topic}-{partition}"
                            ),
                            "kafka.log.LogManager",
                        ),
                        "WARN" if rng.gen_bool(0.5) => (
                            format!(
                                "[ReplicaFetcher brokerId={broker}] Fetch request for {topic}-{partition} timed out"
                            ),
                            "kafka.server.ReplicaFetcherThread",
                        ),
                        "WARN" => (
                            format!(
                                "[Partition {topic}-{partition} broker={broker}] Replica is out of sync"
                            ),
                            "kafka.cluster.Partition",
                        ),
                        _ => match rng.gen_range(0..5) {
                            0 => (
                                format!("[KafkaServer id={broker}] started"),
                                "kafka.server.KafkaServer",
                            ),
                            1 => (
                                format!(
                                    "[GroupCoordinator {broker}] Preparing rebalance of group consumer-{partition}"
                                ),
                                "kafka.coordinator.group.GroupCoordinator",
                            ),
                            2 => (
                                format!(
                                    "[Partition {topic}-{partition} broker={broker}] Leader epoch advanced"
                                ),
                                "kafka.cluster.Partition",
                            ),
                            3 => (
                                format!(
                                    "[Controller id={broker}] Completed leader election for {topic}-{partition}"
                                ),
                                "kafka.controller.KafkaController",
                            ),
                            _ => (
                                format!(
                                    "[SocketServer brokerId={broker}] Accepted connection from {}.{}.{}.{}",
                                    ip.0, ip.1, ip.2, ip.3
                                ),
                                "kafka.network.SocketServer",
                            ),
                        },
                    };
                    format!(
                        "[{},{:03}] {level} {message} ({logger})",
                        start.format("%Y-%m-%d %H:%M:%S"),
                        start.timestamp_subsec_millis()
                    )
                }
                LogType::Redis => {
                    let pid = rng.gen_range(1000..=99999);
                    let (severity, message) = if args.settings.redis_levels.is_empty() {
                        match rng.gen_range(0..8) {
                            0 => ("*", "Ready to accept connections".to_owned()),
                            1 => (
                                "*",
                                format!(
                                    "Background saving started by pid {}",
                                    rng.gen_range(1000..=99999)
                                ),
                            ),
                            2 => ("*", "Background saving terminated with success".to_owned()),
                            3 => ("*", "DB saved on disk".to_owned()),
                            4 => (
                                "*",
                                format!(
                                    "{} changes in 300 seconds. Saving...",
                                    rng.gen_range(1..=1000)
                                ),
                            ),
                            5 => ("#", "Connection with replica lost".to_owned()),
                            6 => (
                                "-",
                                format!(
                                    "{} clients connected ({} replicas), {} bytes in use",
                                    rng.gen_range(1..=100),
                                    rng.gen_range(0..=3),
                                    rng.gen_range(100_000..=9_000_000)
                                ),
                            ),
                            _ => (
                                "*",
                                format!(
                                    "Replica {}.{}.{}.{}:{} asks for synchronization",
                                    ip.0,
                                    ip.1,
                                    ip.2,
                                    ip.3,
                                    rng.gen_range(1024..=65535)
                                ),
                            ),
                        }
                    } else {
                        match compat::choose_weighted(&args.settings.redis_levels, &[], &mut rng)
                            .map_err(io::Error::other)?
                        {
                            "DEBUG" => (
                                ".",
                                [
                                    "Command processed",
                                    "Client connection accepted",
                                    "Key expiration cycle completed",
                                ][rng.gen_range(0..3)]
                                .to_owned(),
                            ),
                            "VERBOSE" => (
                                "-",
                                [
                                    "Client connected",
                                    "Client disconnected",
                                    "Replication backlog updated",
                                ][rng.gen_range(0..3)]
                                .to_owned(),
                            ),
                            "NOTICE" => (
                                "*",
                                [
                                    "Ready to accept connections",
                                    "Background saving terminated with success",
                                    "DB saved on disk",
                                ][rng.gen_range(0..3)]
                                .to_owned(),
                            ),
                            "WARNING" => (
                                "#",
                                [
                                    "Connection with replica lost",
                                    "Background saving failed",
                                    "Failed to persist append only file",
                                ][rng.gen_range(0..3)]
                                .to_owned(),
                            ),
                            _ => return Err(io::Error::other("invalid Redis log level")),
                        }
                    };
                    *generated_level = Some(
                        match severity {
                            "." => "DEBUG",
                            "-" => "VERBOSE",
                            "*" => "NOTICE",
                            "#" => "WARNING",
                            _ => return Err(io::Error::other("invalid Redis severity")),
                        }
                        .to_owned(),
                    );
                    format!(
                        "{pid}:M {} {severity} {message}",
                        start.format("%d %b %Y %H:%M:%S%.3f")
                    )
                }
                LogType::SshAuth => {
                    let pid = rng.gen_range(1000..=99999);
                    let users = ["admin", "deploy", "ubuntu", "service", "root"];
                    let user = users[rng.gen_range(0..users.len())];
                    let client = format!("{}.{}.{}.{}", ip.0, ip.1, ip.2, ip.3);
                    let port = rng.gen_range(1024..=65535);
                    let event = match rng.gen_range(0..6) {
                        0 => format!(
                            "Failed password for invalid user {user} from {client} port {port} ssh2"
                        ),
                        1 => format!("Failed password for {user} from {client} port {port} ssh2"),
                        2 => format!("Accepted password for {user} from {client} port {port} ssh2"),
                        3 => format!(
                            "Accepted publickey for {user} from {client} port {port} ssh2: ED25519 SHA256:syntheticFingerprint"
                        ),
                        4 => format!(
                            "Connection closed by authenticating user {user} {client} port {port} [preauth]"
                        ),
                        _ => format!("Invalid user {user} from {client} port {port}"),
                    };
                    format!(
                        "{} server sshd[{pid}]: {event}",
                        start.format("%b %e %H:%M:%S")
                    )
                }
                LogType::Custom => {
                    compat::render_custom(&args.settings, start, &mut rng, args.record_index)
                        .map_err(io::Error::other)?
                }
                LogType::ErrorPattern => {
                    let (category, severity, message) =
                        compat::library_message(&args.settings, &mut rng)
                            .map_err(io::Error::other)?;
                    format!("{iso} [{severity}] category={category} {message}")
                }
                LogType::Scenario | LogType::ErrorLibrary | LogType::Process => {
                    return Err(io::Error::other(
                        "stateful generator requires configuration",
                    ));
                }
                LogType::Microservice => {
                    let service =
                        args.settings.service_name.as_deref().ok_or_else(|| {
                            io::Error::other("microservice requires configuration")
                        })?;
                    let level = compat::choose_weighted(
                        &args.settings.service_levels,
                        &[
                            ("INFO", 0.70),
                            ("DEBUG", 0.15),
                            ("WARNING", 0.10),
                            ("ERROR", 0.05),
                        ],
                        &mut rng,
                    )
                    .map_err(io::Error::other)?;
                    let use_gateway = !args.settings.gateway_upstreams.is_empty()
                        && (args.settings.service_events.is_empty() || rng.gen_bool(0.5));
                    let (operation, message) = if use_gateway {
                        (
                            "request",
                            compat::gateway_message(&args.settings, &mut rng)
                                .map_err(io::Error::other)?,
                        )
                    } else {
                        let event = &args.settings.service_events
                            [rng.gen_range(0..args.settings.service_events.len())];
                        let operation = event.operation(
                            &args.settings.service_operations,
                            args.settings.service_events.len() == 1,
                        );
                        let message = crate::microservice::render_event(
                            event.template(),
                            service,
                            &args.settings.service_databases,
                            &args.settings.service_containers,
                            &mut rng,
                        )
                        .map_err(io::Error::other)?;
                        (operation, message)
                    };
                    let trace = args
                        .trace
                        .as_ref()
                        .map(|context| {
                            format!(
                                " trace_id={} span_id={} parent_span_id={} {}={} {}={}",
                                context.trace_id,
                                context.span_id,
                                context.parent_span_id.as_deref().unwrap_or("-"),
                                context.trace_header,
                                context.trace_id,
                                context.span_header,
                                context.span_id
                            )
                        })
                        .unwrap_or_default();
                    format!(
                        "{iso} [{level}] service={service} operation={operation}{trace} {message}"
                    )
                }
                LogType::NginxAccess | LogType::ApacheAccess => unreachable!(),
            };
            writeln!(output, "{line}")?;
            continue;
        }
        if matches!(args.log_type, LogType::ApacheAccess) && !args.settings.virtual_hosts.is_empty()
        {
            let host = compat::choose(&args.settings.virtual_hosts, "", &mut rng);
            write!(output, "{host} ")?;
        }
        let remote_user = if matches!(args.log_type, LogType::ApacheAccess) && rng.gen_bool(0.05) {
            format!("user{}", rng.gen_range(1..=10000))
        } else {
            "-".to_owned()
        };
        write!(
            output,
            "{}.{}.{}.{} - {} [{}] \"{} {} HTTP/1.1\" {} {}",
            ip.0, ip.1, ip.2, ip.3, remote_user, timestamp, method, path, status, size
        )?;
        if matches!(args.format, Format::Combined) {
            let agent = if args.nginx.user_agents.is_empty()
                || !matches!(args.log_type, LogType::NginxAccess | LogType::ApacheAccess)
            {
                if matches!(args.log_type, LogType::NginxAccess) {
                    let agents = [
                        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36",
                        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36",
                        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36",
                        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:89.0) Gecko/20100101 Firefox/89.0",
                        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:89.0) Gecko/20100101 Firefox/89.0",
                        "Mozilla/5.0 (iPhone; CPU iPhone OS 14_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/14.0 Mobile/15E148 Safari/604.1",
                        "Mozilla/5.0 (iPad; CPU OS 14_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/14.0 Mobile/15E148 Safari/604.1",
                        "curl/7.68.0",
                        "Googlebot/2.1 (+http://www.google.com/bot.html)",
                        "facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)",
                    ];
                    agents[rng.gen_range(0..agents.len())]
                } else {
                    let agents = [
                        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36",
                        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36",
                        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36",
                        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:89.0) Gecko/20100101 Firefox/89.0",
                        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:89.0) Gecko/20100101 Firefox/89.0",
                        "Apache-HttpClient/4.5.13 (Java/11.0.11)",
                        "curl/7.68.0",
                        "Wget/1.20.3 (linux-gnu)",
                        "Googlebot/2.1 (+http://www.google.com/bot.html)",
                        "facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)",
                    ];
                    agents[rng.gen_range(0..agents.len())]
                }
            } else {
                &args.nginx.user_agents[rng.gen_range(0..args.nginx.user_agents.len())]
            };
            let referer = if matches!(args.log_type, LogType::NginxAccess) {
                match rng.gen_range(0..7) {
                    0 => "-".to_owned(),
                    1 => format!(
                        "https://www.google.com/?q=search{}",
                        rng.gen_range(1..=10000)
                    ),
                    2 => "https://www.bing.com/".to_owned(),
                    3 => "https://github.com/".to_owned(),
                    4 => "https://stackoverflow.com/".to_owned(),
                    5 => format!("https://ref{}.example.test/", rng.gen_range(1..=10000)),
                    _ => format!("https://ref{}.example.test/page", rng.gen_range(1..=10000)),
                }
            } else {
                match rng.gen_range(0..8) {
                    0 => "-".to_owned(),
                    1 => "https://www.google.com/".to_owned(),
                    2 => "https://www.bing.com/".to_owned(),
                    3 => "https://duckduckgo.com/".to_owned(),
                    4 => "https://github.com/".to_owned(),
                    5 => "https://stackoverflow.com/".to_owned(),
                    6 => format!("https://ref{}.example.test/", rng.gen_range(1..=10000)),
                    _ => format!(
                        "https://ref{}.example.test/page.html",
                        rng.gen_range(1..=10000)
                    ),
                }
            };
            write!(output, " \"{}\" \"{}\"", referer, agent)?;
        }
        writeln!(output)?;
    }
    output.flush()
}

pub(crate) fn run(
    args: Vec<String>,
    registry: &crate::GeneratorRegistry,
) -> Result<(), Box<dyn std::error::Error>> {
    let executable = if args
        .first()
        .and_then(|arg| std::path::Path::new(arg).file_stem())
        .is_some_and(|name| name == "log-generator")
    {
        "log-generator"
    } else {
        "log-forge"
    };
    if args.len() <= 1 || matches!(args.get(1).map(String::as_str), Some("--help" | "-h")) {
        root_command(executable).print_help()?;
        println!();
        return Ok(());
    }
    if matches!(args.get(1).map(String::as_str), Some("--version" | "-V")) {
        println!("{executable} {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("help") {
        return if let Some(command) = args.get(2) {
            run(
                vec![args[0].clone(), command.clone(), "--help".to_owned()],
                registry,
            )
        } else {
            root_command(executable).print_help()?;
            println!();
            Ok(())
        };
    }
    #[cfg(any(unix, windows))]
    if matches!(
        args.get(1).map(String::as_str),
        Some("status" | "stop" | "pause" | "resume" | "monitor")
    ) {
        return crate::control::command(args);
    }
    if args.get(1).map(String::as_str) == Some("init") {
        return crate::init_cli::run(args, registry);
    }
    if args.get(1).map(String::as_str) == Some("validate") {
        return crate::validate_cli::run(args);
    }
    if args.get(1).map(String::as_str) == Some("analyze") {
        return crate::analysis_cli::run(args);
    }
    if args.get(1).map(String::as_str) == Some("sample") {
        let sample = SampleArgs::try_parse_from(
            std::iter::once(crate::app::command_name(&args[0], "sample"))
                .chain(args.into_iter().skip(2)),
        )?;
        if sample.count == 0 {
            return Err("sample count must be greater than zero".into());
        }
        let kind = match sample.kind.as_str() {
            "nginx" => "nginx_access",
            "apache" => "apache_access",
            "mysql" => "mysql_query",
            _ => &sample.kind,
        };
        let mut args = vec![
            "log-forge".into(),
            "start".into(),
            "--output".into(),
            "console".into(),
            "--types".into(),
            kind.to_owned(),
            "--total".into(),
            sample.count.to_string(),
            "--seed".into(),
            sample.seed.to_string(),
            "--no-monitor".into(),
        ];
        if let Some(time) = sample.start_time {
            args.extend(["--start-time".into(), time]);
        }
        if let Some(path) = sample.config {
            args.extend([
                "--config".into(),
                path.into_os_string()
                    .into_string()
                    .map_err(|_| "configuration path is not UTF-8")?,
            ]);
        }
        return compat::run(args, registry);
    }
    if matches!(
        args.get(1).map(String::as_str),
        Some("start" | "list-types" | "config")
    ) {
        return compat::run(args, registry);
    }
    // Direct mode takes only options, so a leading word is a mistyped command.
    if args.get(1).is_some_and(|arg| !arg.starts_with('-')) {
        root_command(executable).try_get_matches_from(&args[..2])?;
    }
    let args = Args::try_parse_from(args)?;
    let start = args.start.unwrap_or_else(Utc::now);
    let file = File::create(&args.output)?;
    generate(BufWriter::new(file), &args, start)?;
    Ok(())
}
