use chrono::{DateTime, Duration, TimeZone, Utc};
use log_forge::{
    GeneratorError, LogGenerator,
    engine::GenerationEngine,
    output::{OutputHandler, OutputSink, RotationOptions},
};

struct MemorySink {
    records: Arc<std::sync::Mutex<Vec<String>>>,
    closed: Arc<AtomicUsize>,
    fail: bool,
}

impl OutputSink for MemorySink {
    fn write(
        &mut self,
        kind: &str,
        _time: DateTime<Utc>,
        record: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.fail {
            return Err("sink failed".into());
        }
        self.records
            .lock()
            .unwrap()
            .push(format!("{kind}:{record}"));
        Ok(())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    fn close(&mut self) -> std::io::Result<()> {
        self.closed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn supports_custom_sinks_and_closes_them_after_write_failure() {
    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let closed = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let mut engine = GenerationEngine::new(0);
    let errors = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = Arc::clone(&errors);
    engine.add_error_handler(move |error| observed.lock().unwrap().push(error.to_string()));
    engine
        .add_generator(
            "one",
            1.0,
            Box::new(Generator {
                name: "one",
                finished: Arc::clone(&finished),
                fail: false,
            }),
        )
        .unwrap();
    for fail in [false, true] {
        engine.add_output(MemorySink {
            records: Arc::clone(&records),
            closed: Arc::clone(&closed),
            fail,
        });
    }
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    assert!(engine.run([time]).is_err());
    assert_eq!(engine.statistics().errors_count, 1);
    assert_eq!(*errors.lock().unwrap(), ["sink failed"]);
    assert_eq!(engine.statistics().total_logs_generated, 0);
    assert_eq!(
        records.lock().unwrap().as_slice(),
        ["one:one 2024-01-01T00:00:00+00:00"]
    );
    engine.close().unwrap();
    assert_eq!(closed.load(Ordering::SeqCst), 2);
    assert_eq!(finished.load(Ordering::SeqCst), 1);
}
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Generator {
    name: &'static str,
    finished: Arc<AtomicUsize>,
    fail: bool,
}

struct FailCloseSink;

struct FailFirstGenerator(bool);

impl LogGenerator for FailFirstGenerator {
    fn generate(&mut self, _timestamp: DateTime<Utc>) -> Result<String, GeneratorError> {
        if !self.0 {
            self.0 = true;
            return Err("first record failed".into());
        }
        Ok("record".into())
    }
    fn pattern(&self) -> &str {
        ".*"
    }
    fn validate(&self, _record: &str) -> bool {
        true
    }
}

#[test]
fn background_engine_can_continue_after_record_errors() {
    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let closed = Arc::new(AtomicUsize::new(0));
    let errors = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut engine = GenerationEngine::new(42);
    engine.set_continue_on_error(true);
    engine
        .add_generator("event", 1.0, Box::new(FailFirstGenerator(false)))
        .unwrap();
    for fail in [true, false] {
        engine.add_output(MemorySink {
            records: Arc::clone(&records),
            closed: Arc::clone(&closed),
            fail,
        });
    }
    let observed = Arc::clone(&errors);
    engine.add_error_handler(move |error| observed.lock().unwrap().push(error.to_string()));
    let stats = engine
        .run_background([Utc::now(); 3])
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(stats.total_logs_generated, 2);
    assert_eq!(stats.errors_count, 3);
    assert_eq!(records.lock().unwrap().len(), 2);
    assert_eq!(
        *errors.lock().unwrap(),
        ["first record failed", "sink failed", "sink failed"]
    );
    assert_eq!(closed.load(Ordering::SeqCst), 2);
}

#[test]
fn failed_error_handler_does_not_skip_other_handlers_or_cleanup() {
    let errors = Arc::new(std::sync::Mutex::new(Vec::new()));
    let finished = Arc::new(AtomicUsize::new(0));
    let mut engine = GenerationEngine::new(42);
    engine
        .add_generator(
            "event",
            1.0,
            Box::new(Generator {
                name: "event",
                finished: Arc::clone(&finished),
                fail: true,
            }),
        )
        .unwrap();
    engine.add_output(FailCloseSink);
    engine.add_fallible_error_handler(|_| Err(std::io::Error::other("callback failed")));
    let observed = Arc::clone(&errors);
    engine.add_error_handler(move |error| observed.lock().unwrap().push(error.to_string()));
    let error = engine
        .run_background([Utc::now()])
        .unwrap()
        .wait()
        .unwrap_err()
        .to_string();
    assert!(error.contains("generation failed"));
    assert!(error.contains("close failure marker"));
    assert_eq!(
        *errors.lock().unwrap(),
        ["generation failed", "close failure marker"]
    );
    assert_eq!(finished.load(Ordering::SeqCst), 1);
}

#[test]
fn background_engine_notifies_error_handlers_for_generation_and_close() {
    let errors = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut engine = GenerationEngine::new(42);
    engine
        .add_generator(
            "event",
            1.0,
            Box::new(Generator {
                name: "event",
                finished: Arc::new(AtomicUsize::new(0)),
                fail: true,
            }),
        )
        .unwrap();
    engine.add_output(FailCloseSink);
    let observed = Arc::clone(&errors);
    engine.add_error_handler(move |error| observed.lock().unwrap().push(error.to_string()));
    assert!(engine.run_background([Utc::now()]).unwrap().wait().is_err());
    assert_eq!(
        *errors.lock().unwrap(),
        ["generation failed", "close failure marker"]
    );
}

impl OutputSink for FailCloseSink {
    fn write(
        &mut self,
        _kind: &str,
        _timestamp: DateTime<Utc>,
        _record: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    fn close(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::other("close failure marker"))
    }
}

#[test]
fn background_close_failure_is_recorded_in_final_snapshot() {
    let finished = Arc::new(AtomicUsize::new(0));
    let mut engine = GenerationEngine::new(42);
    engine
        .add_generator(
            "event",
            1.0,
            Box::new(Generator {
                name: "event",
                finished,
                fail: false,
            }),
        )
        .unwrap();
    engine.add_output(FailCloseSink);
    let background = engine.run_background([Utc::now()]).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while background.status().unwrap().running && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!background.status().unwrap().running);
    let stats = background.statistics().unwrap();
    assert_eq!(stats.total_logs_generated, 1);
    assert_eq!(stats.errors_count, 1);
    assert!(stats.end_time.is_some());
    let (status, snapshot) = background.snapshot().unwrap();
    assert!(!status.running);
    assert_eq!(status.generated, snapshot.total_logs_generated);
    assert_eq!(snapshot.errors_count, 1);
    assert!(snapshot.end_time.is_some());
    assert!(
        background
            .wait()
            .unwrap_err()
            .to_string()
            .contains("close failure marker")
    );
}

#[test]
fn background_generation_and_close_errors_are_both_preserved() {
    let finished = Arc::new(AtomicUsize::new(0));
    let mut engine = GenerationEngine::new(42);
    engine
        .add_generator(
            "broken",
            1.0,
            Box::new(Generator {
                name: "broken",
                finished: Arc::clone(&finished),
                fail: true,
            }),
        )
        .unwrap();
    engine.add_output(FailCloseSink);
    let background = engine.run_background([Utc::now()]).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while background.status().unwrap().running && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!background.status().unwrap().running);
    let stats = background.statistics().unwrap();
    assert_eq!(stats.total_logs_generated, 0);
    assert_eq!(stats.errors_count, 2);
    assert!(stats.end_time.is_some());
    let (status, snapshot) = background.snapshot().unwrap();
    assert!(!status.running);
    assert_eq!(status.generated, snapshot.total_logs_generated);
    assert_eq!(snapshot.errors_count, 2);
    assert!(snapshot.end_time.is_some());
    let error = background.wait().unwrap_err().to_string();
    assert!(error.contains("generation failed"), "{error}");
    assert!(error.contains("close failure marker"), "{error}");
    assert_eq!(finished.load(Ordering::SeqCst), 1);
}

#[test]
fn engine_rejects_overflowing_total_weight_before_output() {
    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let closed = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let mut engine = GenerationEngine::new(42);
    for name in ["first", "second"] {
        engine
            .add_generator(
                name,
                1.0e308,
                Box::new(Generator {
                    name,
                    finished: Arc::clone(&finished),
                    fail: false,
                }),
            )
            .unwrap();
    }
    engine.add_output(MemorySink {
        records: Arc::clone(&records),
        closed,
        fail: false,
    });
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let error = engine.run([time]).unwrap_err();
    assert!(error.to_string().contains("weight total must be finite"));
    assert!(records.lock().unwrap().is_empty());
    engine.close().unwrap();
}

#[test]
fn engine_statistics_dictionary_includes_original_aliases() {
    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let closed = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let mut engine = GenerationEngine::new(42);
    engine
        .add_generator(
            "event",
            1.0,
            Box::new(Generator {
                name: "event",
                finished,
                fail: false,
            }),
        )
        .unwrap();
    engine.add_output(MemorySink {
        records,
        closed,
        fail: false,
    });
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    engine.run([time, time]).unwrap();
    let stats = engine.statistics_dict();
    assert_eq!(stats["total_logs_generated"], 2);
    assert!(stats["generation_rate"].as_f64().unwrap() > 0.0);
    assert!(stats["end_time"].is_null());
    assert_eq!(stats["log_type_counts"], stats["logs_by_type"]);
    assert_eq!(stats["error_count"], stats["errors_count"]);
    engine.close().unwrap();
}

struct GatedGenerator {
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
    first: bool,
}

impl LogGenerator for GatedGenerator {
    fn generate(&mut self, _time: DateTime<Utc>) -> Result<String, GeneratorError> {
        if self.first {
            self.first = false;
            self.entered.send(())?;
            self.release.recv()?;
        } else {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Ok("event".into())
    }
    fn pattern(&self) -> &str {
        "^event$"
    }
    fn validate(&self, record: &str) -> bool {
        record == "event"
    }
}

#[test]
fn background_engine_pauses_resumes_and_stops() {
    use std::{
        sync::mpsc,
        time::{Duration as WallDuration, Instant},
    };
    let (entered, ready) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let closed = Arc::new(AtomicUsize::new(0));
    let mut engine = GenerationEngine::new(42);
    engine
        .add_generator(
            "event",
            1.0,
            Box::new(GatedGenerator {
                entered,
                release: gate,
                first: true,
            }),
        )
        .unwrap();
    engine.add_output(MemorySink {
        records: Arc::clone(&records),
        closed: Arc::clone(&closed),
        fail: false,
    });
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let background = engine
        .run_background(std::iter::repeat_n(time, 10000))
        .unwrap();
    ready.recv_timeout(WallDuration::from_secs(2)).unwrap();
    background.pause().unwrap();
    release.send(()).unwrap();
    let deadline = Instant::now() + WallDuration::from_secs(2);
    while background.status().unwrap().generated < 1 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let paused = background.status().unwrap();
    assert!(paused.paused);
    assert_eq!(paused.generated, 1);
    let stats = background.statistics().unwrap();
    assert_eq!(stats.total_logs_generated, 1);
    assert_eq!(stats.logs_by_type["event"], 1);
    assert_eq!(stats.errors_count, 0);
    assert!(stats.generation_rate > 0.0);
    assert!(stats.end_time.is_none());
    let (status, snapshot) = background.snapshot().unwrap();
    assert!(status.paused);
    assert_eq!(status.generated, snapshot.total_logs_generated);
    std::thread::sleep(WallDuration::from_millis(20));
    assert_eq!(background.status().unwrap().generated, 1);
    background.resume().unwrap();
    for _ in 0..100 {
        let (status, stats) = background.snapshot().unwrap();
        assert_eq!(status.generated, stats.total_logs_generated);
        assert_eq!(
            stats.logs_by_type.values().sum::<u64>(),
            stats.total_logs_generated
        );
    }
    while background.status().unwrap().generated < 2 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(background.status().unwrap().generated >= 2);
    background.stop().unwrap();
    let stats = background.wait().unwrap();
    assert!(stats.total_logs_generated >= 2 && stats.total_logs_generated < 10000);
    assert_eq!(
        records.lock().unwrap().len(),
        stats.total_logs_generated as usize
    );
    assert_eq!(closed.load(Ordering::SeqCst), 1);
}

#[test]
fn background_snapshots_remain_consistent_during_fast_generation() {
    let finished = Arc::new(AtomicUsize::new(0));
    let mut engine = GenerationEngine::new(42);
    engine
        .add_generator(
            "event",
            1.0,
            Box::new(Generator {
                name: "event",
                finished,
                fail: false,
            }),
        )
        .unwrap();
    engine.add_output(MemorySink {
        records: Arc::new(std::sync::Mutex::new(Vec::new())),
        closed: Arc::new(AtomicUsize::new(0)),
        fail: false,
    });
    let background = engine
        .run_background(std::iter::repeat_n(Utc::now(), 10_000))
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut previous = 0;
    loop {
        let (status, stats) = background.snapshot().unwrap();
        assert_eq!(status.generated, stats.total_logs_generated);
        assert_eq!(status.running, stats.end_time.is_none());
        assert_eq!(
            stats.logs_by_type.values().sum::<u64>(),
            stats.total_logs_generated
        );
        assert!(stats.total_logs_generated >= previous);
        previous = stats.total_logs_generated;
        if !status.running {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "generation did not finish"
        );
        std::thread::yield_now();
    }
    assert_eq!(background.wait().unwrap().total_logs_generated, 10_000);
}

#[test]
fn background_engine_reports_failure_and_closes_output() {
    let finished = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicUsize::new(0));
    let mut engine = GenerationEngine::new(0);
    engine
        .add_generator(
            "broken",
            1.0,
            Box::new(Generator {
                name: "broken",
                finished: Arc::clone(&finished),
                fail: true,
            }),
        )
        .unwrap();
    engine.add_output(MemorySink {
        records: Arc::new(std::sync::Mutex::new(Vec::new())),
        closed: Arc::clone(&closed),
        fail: false,
    });
    let background = engine.run_background([Utc::now()]).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while background.status().unwrap().running && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(!background.status().unwrap().running);
    let snapshot = background.statistics().unwrap();
    assert_eq!(snapshot.errors_count, 1);
    assert_eq!(snapshot.total_logs_generated, 0);
    assert!(snapshot.end_time.is_some());
    let error = background.wait().unwrap_err();
    assert!(error.to_string().contains("generation failed"));
    assert_eq!(closed.load(Ordering::SeqCst), 1);
    assert_eq!(finished.load(Ordering::SeqCst), 1);
}
impl LogGenerator for Generator {
    fn generate(&mut self, timestamp: DateTime<Utc>) -> Result<String, GeneratorError> {
        if self.fail {
            return Err("generation failed".into());
        }
        Ok(format!("{} {}", self.name, timestamp.to_rfc3339()))
    }
    fn pattern(&self) -> &str {
        ".*"
    }
    fn validate(&self, _record: &str) -> bool {
        true
    }
    fn finish(&mut self) -> Result<(), GeneratorError> {
        self.finished.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn runs_weighted_generators_to_multiple_outputs_reproducibly() {
    let temp = tempfile::tempdir().unwrap();
    let finish_count = Arc::new(AtomicUsize::new(0));
    let run = |prefix: &str| {
        let mut engine = GenerationEngine::new(42);
        for name in ["one", "two"] {
            engine
                .add_generator(
                    name,
                    1.0,
                    Box::new(Generator {
                        name,
                        finished: Arc::clone(&finish_count),
                        fail: false,
                    }),
                )
                .unwrap();
        }
        assert!(
            engine
                .add_generator(
                    "one",
                    1.0,
                    Box::new(Generator {
                        name: "one",
                        finished: Arc::clone(&finish_count),
                        fail: false
                    })
                )
                .is_err()
        );
        let first = temp.path().join(format!("{prefix}-first.log"));
        let second = temp.path().join(format!("{prefix}-second.log"));
        engine.add_output(OutputHandler::file(&first, RotationOptions::default()).unwrap());
        engine.add_output(OutputHandler::file(&second, RotationOptions::default()).unwrap());
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(
            engine
                .run((0..50).map(|i| start + Duration::seconds(i)))
                .unwrap(),
            50
        );
        assert_eq!(engine.statistics().total_logs_generated, 50);
        let stats = engine.close().unwrap();
        assert_eq!(stats.logs_by_type.values().sum::<u64>(), 50);
        assert_eq!(stats.logs_by_type.len(), 2);
        let contents = fs::read_to_string(first).unwrap();
        assert_eq!(contents, fs::read_to_string(second).unwrap());
        contents
    };
    assert_eq!(run("a"), run("b"));
    assert_eq!(finish_count.load(Ordering::SeqCst), 4);
}

#[test]
fn engine_reports_generator_failures_and_still_closes() {
    let temp = tempfile::tempdir().unwrap();
    let finish_count = Arc::new(AtomicUsize::new(0));
    let mut engine = GenerationEngine::new(0);
    engine
        .add_generator(
            "broken",
            1.0,
            Box::new(Generator {
                name: "broken",
                finished: Arc::clone(&finish_count),
                fail: true,
            }),
        )
        .unwrap();
    engine.add_output(
        OutputHandler::file(&temp.path().join("out.log"), RotationOptions::default()).unwrap(),
    );
    assert!(engine.run([Utc::now()]).is_err());
    assert_eq!(engine.statistics().errors_count, 1);
    assert_eq!(engine.close().unwrap().total_logs_generated, 0);
    assert_eq!(finish_count.load(Ordering::SeqCst), 1);
}

#[test]
fn engine_preserves_structured_generator_fields_in_json_output() {
    struct Structured;
    impl LogGenerator for Structured {
        fn generate(&mut self, _: DateTime<Utc>) -> Result<String, GeneratorError> {
            Err("text generation unavailable".into())
        }
        fn generate_entry(
            &mut self,
            time: DateTime<Utc>,
        ) -> Result<Option<log_forge::models::LogEntry>, GeneratorError> {
            let mut entry = log_forge::models::LogEntry::new(time, "custom", "WARN", "event");
            entry
                .custom_fields
                .insert("request_id".into(), serde_json::json!("req-42"));
            Ok(Some(entry))
        }
        fn pattern(&self) -> &str {
            ".*"
        }
        fn validate(&self, _: &str) -> bool {
            true
        }
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("entries.jsonl");
    let output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .json(Default::default(), false, false);
    let mut engine = GenerationEngine::new(42);
    engine
        .add_generator("custom", 1.0, Box::new(Structured))
        .unwrap();
    engine.add_output(output);
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    assert_eq!(engine.run([time]).unwrap(), 1);
    engine.close().unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(value["request_id"], "req-42");
    assert_eq!(value["level"], "WARN");

    let mut engine = GenerationEngine::new(42);
    engine
        .add_generator("custom", 1.0, Box::new(Structured))
        .unwrap();
    engine.add_output(MemorySink {
        records: Arc::new(std::sync::Mutex::new(Vec::new())),
        closed: Arc::new(AtomicUsize::new(0)),
        fail: false,
    });
    let error = engine.run([time]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("structured entries are not supported")
    );
    assert_eq!(engine.statistics().errors_count, 1);
    engine.close().unwrap();
}
