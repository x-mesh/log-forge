use log_forge::{analysis::LogAnalyzer, validation::LogValidator};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut validator = LogValidator::with_pattern("events", r"^event=\w+")?;
    validator.add_required_field("trace_id", r"\btrace_id=[0-9a-f]{4}\b")?;
    let mut analyzer = LogAnalyzer::with_validator(validator);
    analyzer.observe("event=login trace_id=abcd");
    analyzer.observe("event=logout");
    analyzer.observe("invalid record");
    println!("{}", serde_json::to_string_pretty(&analyzer.statistics())?);
    Ok(())
}
