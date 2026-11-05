use chrono::{TimeZone, Utc};
use log_forge::{
    models::LogEntry,
    output::{OutputHandler, RotationOptions},
};
use std::{collections::BTreeMap, error::Error, fs};

fn main() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let text_path = directory.path().join("events.log");
    let json_path = directory.path().join("events.jsonl");
    let text = OutputHandler::file(&text_path, RotationOptions::default())?;
    let json = OutputHandler::file(&json_path, RotationOptions::default())?.elk_json(
        BTreeMap::new(),
        true,
        false,
    );
    let mut output = OutputHandler::fanout(vec![text, json])?;
    let timestamp = Utc
        .with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
        .single()
        .ok_or("invalid time")?;
    let mut entry = LogEntry::new(timestamp, "example", "INFO", "Request completed");
    entry.source_ip = Some("192.0.2.42".to_owned());
    entry.status_code = Some(200);
    entry
        .custom_fields
        .insert("request_id".to_owned(), serde_json::json!("example-1"));
    output.write_entry(&entry)?;
    output.flush()?;
    output.close()?;
    println!("Text: {}", fs::read_to_string(text_path)?.trim());
    println!("ELK JSON: {}", fs::read_to_string(json_path)?.trim());
    Ok(())
}
