use crate::{
    analysis::LogAnalyzer,
    validate_cli::{infer_type, input_files, scan_records, validator_for},
};
use clap::Parser;
use std::{collections::BTreeMap, error::Error, io, path::PathBuf};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long = "type")]
    kind: Option<String>,
    #[arg(long, help = "Accept records that match this regular expression")]
    pattern: Option<String>,
    #[arg(long)]
    pretty: bool,
}

pub(crate) fn run(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let cli =
        Args::try_parse_from(std::iter::once(args[0].clone()).chain(args.into_iter().skip(2)))?;
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
    if cli.pretty {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("{}", serde_json::to_string(&stats)?);
    }
    Ok(())
}
