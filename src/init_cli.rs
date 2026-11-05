use crate::{GeneratorRegistry, app::compat};
use chrono::NaiveDateTime;
use clap::Parser;
use serde_json::{Map, json};
use std::{
    error::Error,
    fs::{self, OpenOptions},
    io::{self, BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
};

type WeightedTypes = Vec<(String, Option<f64>)>;

const DEFAULT_TYPES: &str = "nginx_access";
const DEFAULT_OUTPUT: &str = "multi_file";
const DEFAULT_TOTAL: u64 = 10_000;
const START_TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";
// These types need extra settings, such as a template or a scenario file.
const TYPES_WITH_SETTINGS: &[&str] = &[
    "custom",
    "scenario",
    "microservice",
    "error_pattern",
    "error_library",
    "process",
];

#[derive(Parser)]
#[command(
    about = "Create a configuration by answering a few questions",
    after_help = "In a terminal, init asks for each value that an option does not set.\n\nExamples:\n  log-forge init\n  log-forge init --types nginx_access=0.7,syslog=0.3 --total 5000 --yes"
)]
struct Args {
    #[arg(
        short,
        long,
        default_value = "config.yaml",
        help = "Set the configuration file to create (.yaml, .yml, or .json)"
    )]
    file: PathBuf,
    #[arg(
        long,
        help = "Select comma-separated log types, with optional weights such as nginx_access=0.7"
    )]
    types: Option<String>,
    #[arg(
        short,
        long,
        value_parser = ["multi_file", "file", "json", "console"],
        help = "Select multi_file, file, json, or console"
    )]
    output: Option<String>,
    #[arg(short, long, help = "Set the output file or directory")]
    path: Option<String>,
    #[arg(short, long, help = "Set the number of records; zero has no limit")]
    total: Option<u64>,
    #[arg(
        long,
        help = "Set the virtual start time, such as '2024-01-01 00:00:00'"
    )]
    start_time: Option<String>,
    #[arg(
        short,
        long,
        help = "Skip the questions and use the options and defaults"
    )]
    yes: bool,
}

#[derive(Debug, PartialEq)]
struct Answers {
    types: WeightedTypes,
    output: String,
    path: Option<String>,
    total: u64,
    start_time: Option<String>,
}

pub(crate) fn run(args: Vec<String>, registry: &GeneratorRegistry) -> Result<(), Box<dyn Error>> {
    let executable = Path::new(&args[0])
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("log-forge")
        .to_owned();
    let cli = Args::try_parse_from(
        std::iter::once(format!("{} init", args[0])).chain(args.into_iter().skip(2)),
    )?;
    if cli.file.exists() {
        return Err(format!(
            "{} already exists\n  try: {executable} init --file <new path>",
            cli.file.display()
        )
        .into());
    }
    let interactive = !cli.yes && io::stdin().is_terminal() && io::stderr().is_terminal();
    if !cli.yes && !interactive {
        return Err(Box::new(clap::Error::raw(
            clap::error::ErrorKind::MissingRequiredArgument,
            format!(
                "init asks questions only in a terminal\n  try: {executable} init --yes (use the options and defaults)\n"
            ),
        )));
    }
    let answers = if interactive {
        ask_all(&cli, &mut io::stdin().lock(), &mut io::stderr(), registry)?
    } else {
        from_options(&cli)?
    };
    let source = render(&answers, &cli.file)?;
    compat::check_config_source(&source, &cli.file, registry)?;
    if let Some(parent) = cli
        .file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&cli.file)?;
    file.write_all(source.as_bytes())?;
    println!("Configuration created: {}", cli.file.display());
    if interactive {
        eprintln!(
            "same as: {}",
            equivalent_command(&executable, &cli.file, &answers)
        );
    }
    eprintln!(
        "next: {executable} start --config {}",
        quote(&cli.file.display().to_string())
    );
    Ok(())
}

fn from_options(cli: &Args) -> Result<Answers, Box<dyn Error>> {
    let output = cli.output.clone().unwrap_or(DEFAULT_OUTPUT.to_owned());
    Ok(Answers {
        types: parse_types(cli.types.as_deref().unwrap_or(DEFAULT_TYPES))?,
        path: cli.path.clone().or_else(|| default_path(&output)),
        output,
        total: cli.total.unwrap_or(DEFAULT_TOTAL),
        start_time: cli
            .start_time
            .as_deref()
            .map(parse_start_time)
            .transpose()?
            .flatten(),
    })
}

fn ask_all(
    cli: &Args,
    input: &mut impl BufRead,
    prompt: &mut impl Write,
    registry: &GeneratorRegistry,
) -> Result<Answers, Box<dyn Error>> {
    let mut answers = from_options(cli)?;
    writeln!(
        prompt,
        "Press Enter to accept the value in brackets.\n\nLog types:\n{}\n",
        wrap(
            &compat::log_type_names()
                .into_iter()
                .filter(|name| !TYPES_WITH_SETTINGS.contains(&name.as_str()))
                .collect::<Vec<_>>(),
            72
        )
    )?;
    if cli.types.is_none() {
        answers.types = ask(
            input,
            prompt,
            "Log types, comma-separated, weights optional (redis=0.7)",
            DEFAULT_TYPES,
            |text| {
                let types = parse_types(text)?;
                let trial = Answers {
                    types,
                    output: "console".to_owned(),
                    path: None,
                    total: DEFAULT_TOTAL,
                    start_time: None,
                };
                compat::check_config_source(
                    &render(&trial, Path::new("check.yaml"))?,
                    Path::new("check.yaml"),
                    registry,
                )?;
                Ok(trial.types)
            },
        )?;
    }
    if cli.output.is_none() {
        answers.output = ask(
            input,
            prompt,
            "Output: multi_file, file, json, or console",
            DEFAULT_OUTPUT,
            |text| match text {
                "multi_file" | "file" | "json" | "console" => Ok(text.to_owned()),
                _ => Err(format!("unknown output: {text}").into()),
            },
        )?;
        answers.path = cli.path.clone().or_else(|| default_path(&answers.output));
    }
    if cli.path.is_none() && answers.output != "console" {
        let default = default_path(&answers.output).unwrap_or_default();
        let label = if answers.output == "multi_file" {
            "Output directory"
        } else {
            "Output file"
        };
        answers.path = Some(ask(input, prompt, label, &default, |text| {
            Ok(text.to_owned())
        })?);
    }
    if cli.total.is_none() {
        answers.total = ask(
            input,
            prompt,
            "Records, 0 for no limit",
            &DEFAULT_TOTAL.to_string(),
            |text| {
                text.parse::<u64>()
                    .map_err(|_| format!("enter a whole number: {text}").into())
            },
        )?;
    }
    if cli.start_time.is_none() {
        answers.start_time = ask(
            input,
            prompt,
            "Start time (YYYY-MM-DD HH:MM:SS)",
            "now",
            parse_start_time,
        )?;
    }
    writeln!(prompt)?;
    Ok(answers)
}

fn ask<T>(
    input: &mut impl BufRead,
    prompt: &mut impl Write,
    label: &str,
    default: &str,
    parse: impl Fn(&str) -> Result<T, Box<dyn Error>>,
) -> Result<T, Box<dyn Error>> {
    loop {
        write!(prompt, "{label} [{default}]: ")?;
        prompt.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            writeln!(prompt)?;
            return Err("input ended before init finished; no file was created".into());
        }
        let answer = match line.trim() {
            "" => default,
            text => text,
        };
        match parse(answer) {
            Ok(value) => return Ok(value),
            Err(error) => {
                for line in error.to_string().lines() {
                    writeln!(prompt, "  {}", line.trim_start())?;
                }
            }
        }
    }
}

fn parse_types(text: &str) -> Result<WeightedTypes, Box<dyn Error>> {
    let mut types: WeightedTypes = Vec::new();
    for item in text
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let (name, weight) = match item.split_once('=') {
            Some((name, weight)) => {
                let value = weight
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .ok_or_else(|| format!("weight must be a positive number: {item}"))?;
                (name.trim(), Some(value))
            }
            None => (item, None),
        };
        if types.iter().any(|(existing, _)| existing == name) {
            return Err(format!("log type listed twice: {name}").into());
        }
        types.push((name.to_owned(), weight));
    }
    if types.is_empty() {
        return Err("enter at least one log type".into());
    }
    Ok(types)
}

fn parse_start_time(text: &str) -> Result<Option<String>, Box<dyn Error>> {
    if text.is_empty() || text == "now" {
        return Ok(None);
    }
    NaiveDateTime::parse_from_str(text, START_TIME_FORMAT)
        .map_err(|_| format!("use the format YYYY-MM-DD HH:MM:SS: {text}"))?;
    Ok(Some(text.to_owned()))
}

fn default_path(output: &str) -> Option<String> {
    match output {
        "multi_file" => Some("./logs".to_owned()),
        "file" => Some("./logs/generated.log".to_owned()),
        "json" => Some("./logs/generated.json".to_owned()),
        _ => None,
    }
}

fn render(answers: &Answers, file: &Path) -> Result<String, Box<dyn Error>> {
    let share = (1000.0 / answers.types.len() as f64).round() / 1000.0;
    let mut log_types = Map::new();
    for (name, weight) in &answers.types {
        log_types.insert(
            name.clone(),
            json!({"enabled": true, "frequency": weight.unwrap_or(share)}),
        );
    }
    let mut global = Map::new();
    global.insert("output_format".into(), json!(answers.output));
    if answers.output != "console"
        && let Some(path) = &answers.path
    {
        global.insert("output_path".into(), json!(path));
    }
    global.insert("total_logs".into(), json!(answers.total));
    global.insert("generation_interval".into(), json!(0.1));
    global.insert("virtual_time".into(), json!(true));
    if let Some(start_time) = &answers.start_time {
        global.insert("start_time".into(), json!(start_time));
    }
    let document = json!({"log_generator": {"global": global, "log_types": log_types}});
    let is_json = file
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
    Ok(if is_json {
        format!("{}\n", serde_json::to_string_pretty(&document)?)
    } else {
        serde_yaml::to_string(&document)?
    })
}

fn equivalent_command(executable: &str, file: &Path, answers: &Answers) -> String {
    let types = answers
        .types
        .iter()
        .map(|(name, weight)| match weight {
            Some(weight) => format!("{name}={weight}"),
            None => name.clone(),
        })
        .collect::<Vec<_>>()
        .join(",");
    let mut command = format!(
        "{executable} init --file {} --types {types} --output {}",
        quote(&file.display().to_string()),
        answers.output
    );
    if answers.output != "console"
        && let Some(path) = &answers.path
    {
        command.push_str(&format!(" --path {}", quote(path)));
    }
    command.push_str(&format!(" --total {}", answers.total));
    if let Some(start_time) = &answers.start_time {
        command.push_str(&format!(" --start-time {}", quote(start_time)));
    }
    command.push_str(" --yes");
    command
}

fn quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "./_-=,:".contains(c))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

fn wrap(words: &[String], width: usize) -> String {
    let mut lines = vec![String::from(" ")];
    for word in words {
        let line = lines.last_mut().expect("lines starts with one entry");
        if line.len() > 1 && line.len() + 1 + word.len() > width {
            lines.push(format!("  {word}"));
        } else {
            line.push(' ');
            line.push_str(word);
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(extra: &[&str]) -> Args {
        Args::try_parse_from(std::iter::once("log-forge init").chain(extra.iter().copied()))
            .unwrap()
    }

    #[test]
    fn questions_repeat_until_the_answer_is_valid() {
        let mut input = io::Cursor::new(
            "rediss\nredis=0.7,ssh_auth=0.3\nfiles\nfile\n\nabc\n500\n2024-13-01 00:00:00\n2024-01-01 00:00:00\n",
        );
        let mut prompt = Vec::new();
        let answers = ask_all(
            &args(&[]),
            &mut input,
            &mut prompt,
            &GeneratorRegistry::new(),
        )
        .unwrap();
        assert_eq!(
            answers,
            Answers {
                types: vec![("redis".into(), Some(0.7)), ("ssh_auth".into(), Some(0.3))],
                output: "file".into(),
                path: Some("./logs/generated.log".into()),
                total: 500,
                start_time: Some("2024-01-01 00:00:00".into()),
            }
        );
        let prompt = String::from_utf8(prompt).unwrap();
        assert!(prompt.contains("tip: did you mean 'redis'?"), "{prompt}");
        assert!(prompt.contains("unknown output: files"), "{prompt}");
        assert!(prompt.contains("enter a whole number: abc"), "{prompt}");
        assert!(
            prompt.contains("use the format YYYY-MM-DD HH:MM:SS"),
            "{prompt}"
        );
        assert!(!prompt.contains("custom"), "{prompt}");
    }

    #[test]
    fn options_skip_their_questions() {
        let mut input = io::Cursor::new("\n");
        let mut prompt = Vec::new();
        let answers = ask_all(
            &args(&["--types", "syslog", "--output", "console", "--total", "3"]),
            &mut input,
            &mut prompt,
            &GeneratorRegistry::new(),
        )
        .unwrap();
        assert_eq!(answers.types, vec![("syslog".into(), None)]);
        assert_eq!(answers.path, None);
        let prompt = String::from_utf8(prompt).unwrap();
        assert!(!prompt.contains("Log types, comma-separated"), "{prompt}");
        assert!(prompt.contains("Start time"), "{prompt}");
    }

    #[test]
    fn closed_input_stops_without_a_file() {
        let mut input = io::Cursor::new("");
        let error = ask_all(
            &args(&[]),
            &mut input,
            &mut Vec::new(),
            &GeneratorRegistry::new(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("no file was created"));
    }

    #[test]
    fn equivalent_command_quotes_values_with_spaces() {
        let answers = Answers {
            types: vec![("redis".into(), None)],
            output: "multi_file".into(),
            path: Some("./my logs".into()),
            total: 0,
            start_time: Some("2024-01-01 00:00:00".into()),
        };
        assert_eq!(
            equivalent_command("log-forge", Path::new("config.yaml"), &answers),
            "log-forge init --file config.yaml --types redis --output multi_file --path './my logs' --total 0 --start-time '2024-01-01 00:00:00' --yes"
        );
    }
}
