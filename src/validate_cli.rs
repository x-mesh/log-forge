use crate::validation::LogValidator;
use clap::Parser;
use std::{
    error::Error,
    fs::{self, File},
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    sync::LazyLock,
};

static DJANGO_START: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} ")
        .expect("valid Django record pattern")
});

const TYPES: &[&str] = &[
    "kubernetes",
    "kubernetes_event",
    "kubernetes_pod",
    "postgresql_slow",
    "postgresql",
    "nginx_access",
    "nginx_error",
    "apache_access",
    "apache_error",
    "django",
    "django_request",
    "django_sql",
    "mysql_query",
    "mysql_error",
    "mysql_slow",
    "ssh_auth",
    "kafka_broker",
    "haproxy",
    "error_library",
    "microservice",
    "scenario",
    "syslog",
    "fastapi",
    "docker",
    "goloop",
    "redis",
];

#[derive(Parser)]
#[command(
    about = "Validate log files",
    after_help = "Examples:\n  log-forge validate --input ./logs\n  log-forge validate --input access.log --type nginx_access"
)]
struct Args {
    #[arg(long, help = "Read a log file or a directory of .log and .json files")]
    input: PathBuf,
    #[arg(
        long = "type",
        help = "Set the log type; the default comes from the file name"
    )]
    kind: Option<String>,
    #[arg(long, help = "Accept records that match this regular expression")]
    pattern: Option<String>,
}

pub(crate) fn validator_for(kind: &str, pattern: Option<&str>) -> io::Result<LogValidator> {
    match LogValidator::new(kind) {
        Ok(mut validator) => {
            if let Some(pattern) = pattern {
                validator
                    .add_pattern("custom", pattern)
                    .map_err(io::Error::other)?;
            }
            Ok(validator)
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidInput && pattern.is_some() => {
            LogValidator::with_pattern(kind, pattern.unwrap())
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn infer_type(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?;
    TYPES.iter().copied().find(|kind| {
        name.strip_prefix(kind)
            .is_some_and(|rest| rest.starts_with('_') || rest.starts_with('.'))
    })
}

pub(crate) fn input_files(input: PathBuf) -> io::Result<Vec<PathBuf>> {
    if !input.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("input not found: {}", input.display()),
        ));
    }
    let paths = if input.is_dir() {
        let mut paths = Vec::new();
        for entry in fs::read_dir(input)? {
            let entry = entry?;
            if entry.file_type()?.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "log" || ext == "json")
            {
                paths.push(entry.path());
            }
        }
        paths.sort();
        paths
    } else {
        vec![input]
    };
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no log files found",
        ));
    }
    Ok(paths)
}

pub(crate) fn scan_records(
    path: &Path,
    kind: &str,
    mut visit: impl FnMut(&str, usize),
) -> io::Result<()> {
    let input = BufReader::new(File::open(path)?);
    let mut pending = String::new();
    let mut pending_line = 0;
    for (index, line) in input.lines().enumerate() {
        let line = line?;
        if matches!(kind, "mysql_slow" | "django_request" | "django") {
            let starts_record = if kind == "mysql_slow" {
                line.starts_with("# Time:")
            } else {
                DJANGO_START.is_match(&line)
            };
            if starts_record && !pending.is_empty() {
                visit(&pending, pending_line);
                pending.clear();
            }
            if pending.is_empty() {
                pending_line = index + 1;
            } else {
                pending.push('\n');
            }
            pending.push_str(&line);
        } else if !line.trim().is_empty() {
            visit(&line, index + 1);
        }
    }
    if !pending.is_empty() {
        visit(&pending, pending_line);
    }
    Ok(())
}

pub(crate) fn run(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let cli = Args::try_parse_from(
        std::iter::once(crate::app::command_name(&args[0], "validate"))
            .chain(args.into_iter().skip(2)),
    )?;
    let paths = input_files(cli.input)?;
    let mut total = 0_u64;
    let mut invalid = 0_u64;
    for path in paths {
        let kind = cli
            .kind
            .as_deref()
            .or_else(|| infer_type(&path))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("cannot infer log type for {}; use --type", path.display()),
                )
            })?;
        let validator = validator_for(kind, cli.pattern.as_deref())?;
        scan_records(&path, kind, |record, line| {
            let result = validator.validate(record);
            total += 1;
            if !result.is_valid {
                invalid += 1;
                eprintln!("{}:{}: {}", path.display(), line, result.errors.join(", "));
            }
        })?;
    }
    println!(
        "validated={total} valid={} invalid={invalid}",
        total - invalid
    );
    if invalid > 0 {
        return Err("log validation failed".into());
    }
    Ok(())
}
