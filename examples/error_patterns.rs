use chrono::{Duration, TimeZone, Utc};
use log_forge::{
    GeneratorError,
    error_library::ErrorLibrary,
    scenario::{ScenarioGenerator, ScenarioManager},
};

fn main() -> Result<(), GeneratorError> {
    let start = Utc
        .with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
        .single()
        .ok_or("invalid time")?;
    let mut library = ErrorLibrary::from_sources(None, None, None, 42)?;
    println!("Categories: {:?}", library.categories());
    println!(
        "{}",
        library.generate_filtered(start, Some("web_server"), None)?
    );

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("error_scenarios/web_server_cascade.json");
    let mut manager = ScenarioManager::new();
    manager.register(ScenarioGenerator::from_sources(&path, None, 42)?)?;
    manager.start("web_server_cascade", 42)?;
    for minute in [0, 2, 5] {
        println!(
            "{}",
            manager.generate("web_server_cascade", start + Duration::minutes(minute))?
        );
    }
    let analytics = manager
        .pattern_analytics_with_library(&library, "nginx_timeout")?
        .ok_or("missing pattern")?;
    println!("{}", serde_json::to_string_pretty(&analytics.to_dict())?);
    Ok(())
}
