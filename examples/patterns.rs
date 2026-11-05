use chrono::{TimeZone, Utc};
use log_forge::{pattern_file::PatternManager, pattern_validation};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("demo_patterns");
    let manager = PatternManager::open(path)?;
    let pattern = manager.get("custom_web_access").ok_or("missing pattern")?;
    let report = pattern_validation::validate(pattern);
    if !report.valid {
        return Err(format!("invalid pattern: {:?}", report.errors).into());
    }
    let time = Utc
        .with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
        .single()
        .ok_or("invalid time")?;
    for index in 0..3 {
        println!("{}", manager.generate(&pattern.name, time, 42, index)?);
    }
    Ok(())
}
