use chrono::{DateTime, Duration, TimeZone, Utc};
use log_forge::models::LogEntry;
use log_forge::{GeneratorError, GeneratorRegistry, LogGenerator, write_records};

struct ExampleGenerator {
    message: String,
    count: u64,
}

impl LogGenerator for ExampleGenerator {
    fn generate(&mut self, timestamp: DateTime<Utc>) -> Result<String, GeneratorError> {
        self.count += 1;
        Ok(LogEntry::new(
            timestamp,
            "example",
            "INFO",
            &format!("{} #{}", self.message, self.count),
        )
        .to_string())
    }

    fn pattern(&self) -> &str {
        r"^\S+ \[INFO\] .+ #\d+$"
    }

    fn validate(&self, record: &str) -> bool {
        record.contains(" [INFO] ")
    }
}

fn main() -> Result<(), GeneratorError> {
    let mut registry = GeneratorRegistry::new();
    registry.register("example", |config| {
        let message = config.as_str().ok_or("message must be a string")?;
        Ok(Box::new(ExampleGenerator {
            message: message.to_owned(),
            count: 0,
        }))
    })?;
    let mut generator = registry.create(
        "example",
        &serde_yaml::Value::String("Request completed".into()),
    )?;
    let start = Utc
        .with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
        .single()
        .ok_or("invalid time")?;
    let timestamps = (0..3).map(|seconds| start + Duration::seconds(seconds));
    write_records(
        generator.as_mut(),
        &mut std::io::stdout().lock(),
        timestamps,
    )?;
    generator.finish()?;
    Ok(())
}
