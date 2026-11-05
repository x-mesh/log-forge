use crate::{
    analysis::LogAnalyzer,
    validate_cli::{infer_type, input_files, scan_records, validator_for},
};
use clap::Parser;
use std::{
    collections::BTreeMap,
    error::Error,
    io::{self, Write},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    about = "Analyze log files and print statistics as JSON",
    after_help = "Example:\n  log-forge analyze --input access.log --type nginx_access --pretty"
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
    #[arg(long, help = "Indent the JSON output")]
    pretty: bool,
}

pub(crate) fn run(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let cli = Args::try_parse_from(
        std::iter::once(format!("{} analyze", args[0])).chain(args.into_iter().skip(2)),
    )?;
    let mut analyzers = BTreeMap::new();
    for path in input_files(cli.input)? {
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
        if !analyzers.contains_key(kind) {
            analyzers.insert(
                kind.to_owned(),
                LogAnalyzer::with_validator(validator_for(kind, cli.pattern.as_deref())?),
            );
        }
        let analyzer = analyzers.get_mut(kind).ok_or("missing analyzer")?;
        scan_records(&path, kind, |record, _| analyzer.observe(record))?;
    }
    let stats: BTreeMap<_, _> = analyzers
        .into_iter()
        .map(|(kind, analyzer)| (kind, analyzer.statistics()))
        .collect();
    let mut stdout = io::stdout().lock();
    if cli.pretty {
        serde_json::to_writer_pretty(&mut stdout, &stats)?;
    } else {
        serde_json::to_writer(&mut stdout, &stats)?;
    }
    writeln!(stdout)?;
    Ok(())
}
