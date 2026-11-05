use crate::{LogGenerator, models::GenerationStatistics, output::OutputSink};
use chrono::{DateTime, Utc};
use rand::{SeedableRng, distributions::Distribution, rngs::StdRng};
use std::{
    error::Error,
    io,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
};

#[derive(Clone, Copy, Debug)]
pub struct BackgroundStatus {
    pub running: bool,
    pub paused: bool,
    pub stopped: bool,
    pub generated: u64,
}

struct BackgroundState {
    status: Mutex<BackgroundStatus>,
    statistics: Mutex<GenerationStatistics>,
    changed: Condvar,
}

pub struct BackgroundRun {
    state: Arc<BackgroundState>,
    worker: Option<JoinHandle<io::Result<GenerationStatistics>>>,
}

impl BackgroundRun {
    pub fn snapshot(&self) -> io::Result<(BackgroundStatus, GenerationStatistics)> {
        let guard = self
            .state
            .status
            .lock()
            .map_err(|_| io::Error::other("background state is poisoned"))?;
        let mut status = *guard;
        let stats = self.statistics()?;
        status.generated = stats.total_logs_generated;
        if self.worker.as_ref().is_none_or(JoinHandle::is_finished) {
            status.running = false;
        }
        Ok((status, stats))
    }

    pub fn statistics(&self) -> io::Result<GenerationStatistics> {
        let mut stats = self
            .state
            .statistics
            .lock()
            .map_err(|_| io::Error::other("background statistics are poisoned"))?
            .clone();
        if stats.start_time.is_some() && stats.end_time.is_none() {
            stats.end_time = Some(Utc::now());
            stats.calculate_rate();
            stats.end_time = None;
        }
        Ok(stats)
    }
    pub fn status(&self) -> io::Result<BackgroundStatus> {
        let mut status = *self
            .state
            .status
            .lock()
            .map_err(|_| io::Error::other("background state is poisoned"))?;
        if self.worker.as_ref().is_none_or(JoinHandle::is_finished) {
            status.running = false;
        }
        Ok(status)
    }

    pub fn pause(&self) -> io::Result<()> {
        let mut status = self
            .state
            .status
            .lock()
            .map_err(|_| io::Error::other("background state is poisoned"))?;
        if !status.running || status.stopped {
            return Err(io::Error::other("generation is not running"));
        }
        status.paused = true;
        Ok(())
    }

    pub fn resume(&self) -> io::Result<()> {
        let mut status = self
            .state
            .status
            .lock()
            .map_err(|_| io::Error::other("background state is poisoned"))?;
        if !status.running || status.stopped {
            return Err(io::Error::other("generation is not running"));
        }
        status.paused = false;
        self.state.changed.notify_all();
        Ok(())
    }

    pub fn stop(&self) -> io::Result<()> {
        let mut status = self
            .state
            .status
            .lock()
            .map_err(|_| io::Error::other("background state is poisoned"))?;
        status.stopped = true;
        status.paused = false;
        self.state.changed.notify_all();
        Ok(())
    }

    pub fn wait(mut self) -> io::Result<GenerationStatistics> {
        self.worker
            .take()
            .ok_or_else(|| io::Error::other("background worker already joined"))?
            .join()
            .map_err(|_| io::Error::other("background worker panicked"))?
    }
}

impl Drop for BackgroundRun {
    fn drop(&mut self) {
        if self.worker.is_some() {
            let _ = self.stop();
        }
    }
}

struct Generator {
    name: String,
    weight: f64,
    generator: Box<dyn LogGenerator>,
}

type ErrorHandler = Box<dyn Fn(&dyn Error) -> io::Result<()> + Send>;

fn notify_error_handlers(handlers: &[ErrorHandler], error: &dyn Error) {
    for handler in handlers {
        if let Err(handler_error) = handler(error) {
            eprintln!("error handler failed: {handler_error}");
        }
    }
}

pub struct GenerationEngine {
    generators: Vec<Generator>,
    outputs: Vec<Box<dyn OutputSink>>,
    error_handlers: Vec<ErrorHandler>,
    continue_on_error: bool,
    rng: StdRng,
    stats: GenerationStatistics,
}

impl GenerationEngine {
    pub fn new(seed: u64) -> Self {
        Self {
            generators: Vec::new(),
            outputs: Vec::new(),
            error_handlers: Vec::new(),
            continue_on_error: false,
            rng: StdRng::seed_from_u64(seed),
            stats: GenerationStatistics::default(),
        }
    }

    pub fn add_generator(
        &mut self,
        name: &str,
        weight: f64,
        generator: Box<dyn LogGenerator>,
    ) -> io::Result<()> {
        if name.is_empty()
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || self.generators.iter().any(|entry| entry.name == name)
            || !weight.is_finite()
            || weight < 0.0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid or duplicate generator name or weight",
            ));
        }
        self.generators.push(Generator {
            name: name.to_owned(),
            weight,
            generator,
        });
        Ok(())
    }

    pub fn add_output(&mut self, output: impl OutputSink + 'static) {
        self.outputs.push(Box::new(output));
    }

    pub fn set_continue_on_error(&mut self, enabled: bool) {
        self.continue_on_error = enabled;
    }

    pub fn add_error_handler(&mut self, handler: impl Fn(&dyn Error) + Send + 'static) {
        self.add_fallible_error_handler(move |error| {
            handler(error);
            Ok(())
        });
    }

    pub fn add_fallible_error_handler(
        &mut self,
        handler: impl Fn(&dyn Error) -> io::Result<()> + Send + 'static,
    ) {
        self.error_handlers.push(Box::new(handler));
    }

    pub fn statistics(&self) -> &GenerationStatistics {
        &self.stats
    }

    pub fn statistics_dict(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut snapshot = self.stats.clone();
        if snapshot.end_time.is_none() && snapshot.start_time.is_some() {
            snapshot.end_time = Some(Utc::now());
            snapshot.calculate_rate();
            snapshot.end_time = None;
        }
        let mut result = snapshot.to_dict();
        result.insert(
            "log_type_counts".to_owned(),
            serde_json::json!(self.stats.logs_by_type),
        );
        result.insert(
            "error_count".to_owned(),
            serde_json::json!(self.stats.errors_count),
        );
        result
    }

    pub fn run(
        &mut self,
        timestamps: impl IntoIterator<Item = DateTime<Utc>>,
    ) -> Result<u64, Box<dyn Error>> {
        self.run_controlled(timestamps, None)
    }

    fn run_controlled(
        &mut self,
        timestamps: impl IntoIterator<Item = DateTime<Utc>>,
        control: Option<&BackgroundState>,
    ) -> Result<u64, Box<dyn Error>> {
        if self.outputs.is_empty() {
            return Err("at least one output is required".into());
        }
        let weights = crate::weights::checked(self.generators.iter().map(|entry| entry.weight))?;
        self.stats.start_time.get_or_insert_with(Utc::now);
        if let Some(control) = control {
            control
                .statistics
                .lock()
                .map_err(|_| io::Error::other("background statistics are poisoned"))?
                .start_time = self.stats.start_time;
        }
        let mut written = 0;
        for timestamp in timestamps {
            if let Some(control) = control {
                let mut state = control
                    .status
                    .lock()
                    .map_err(|_| io::Error::other("background state is poisoned"))?;
                while state.paused && !state.stopped {
                    state = control
                        .changed
                        .wait(state)
                        .map_err(|_| io::Error::other("background state is poisoned"))?;
                }
                if state.stopped {
                    break;
                }
            }
            let entry = &mut self.generators[weights.sample(&mut self.rng)];
            let generated = entry
                .generator
                .generate_entry(timestamp)
                .and_then(|structured| {
                    if let Some(record) = structured {
                        Ok((Some(record), None))
                    } else {
                        entry
                            .generator
                            .generate_record(timestamp)
                            .map(|record| (None, Some(record)))
                    }
                });
            let (structured, record) = match generated {
                Ok(record) => record,
                Err(error) => {
                    self.stats.add_error();
                    notify_error_handlers(&self.error_handlers, error.as_ref());
                    if let Some(control) = control {
                        control
                            .statistics
                            .lock()
                            .map_err(|_| io::Error::other("background statistics are poisoned"))?
                            .add_error();
                    }
                    if self.continue_on_error {
                        continue;
                    }
                    return Err(error);
                }
            };
            for output in &mut self.outputs {
                let result = if let Some(structured) = &structured {
                    output.write_entry(structured)
                } else {
                    let record = record.as_ref().expect("text record is present");
                    output.write_with_level(
                        &entry.name,
                        timestamp,
                        &record.message,
                        record.level.as_deref(),
                    )
                };
                if let Err(error) = result {
                    self.stats.add_error();
                    notify_error_handlers(&self.error_handlers, error.as_ref());
                    if let Some(control) = control {
                        control
                            .statistics
                            .lock()
                            .map_err(|_| io::Error::other("background statistics are poisoned"))?
                            .add_error();
                    }
                    if !self.continue_on_error {
                        return Err(error);
                    }
                }
            }
            self.stats.add_log(&entry.name);
            if let Some(control) = control {
                control
                    .statistics
                    .lock()
                    .map_err(|_| io::Error::other("background statistics are poisoned"))?
                    .add_log(&entry.name);
                control
                    .status
                    .lock()
                    .map_err(|_| io::Error::other("background state is poisoned"))?
                    .generated = self.stats.total_logs_generated;
            }
            written += 1;
        }
        for output in &mut self.outputs {
            if let Err(error) = output.flush() {
                self.stats.add_error();
                notify_error_handlers(&self.error_handlers, &error);
                if let Some(control) = control {
                    control
                        .statistics
                        .lock()
                        .map_err(|_| io::Error::other("background statistics are poisoned"))?
                        .add_error();
                }
                return Err(error.into());
            }
        }
        Ok(written)
    }

    pub fn run_background<I>(mut self, timestamps: I) -> io::Result<BackgroundRun>
    where
        I: IntoIterator<Item = DateTime<Utc>>,
        I::IntoIter: Send + 'static,
    {
        if self.outputs.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "at least one output is required",
            ));
        }
        crate::weights::checked(self.generators.iter().map(|entry| entry.weight))
            .map_err(io::Error::other)?;
        let state = Arc::new(BackgroundState {
            statistics: Mutex::new(self.stats.clone()),
            status: Mutex::new(BackgroundStatus {
                running: true,
                paused: false,
                stopped: false,
                generated: self.stats.total_logs_generated,
            }),
            changed: Condvar::new(),
        });
        let worker_state = Arc::clone(&state);
        let timestamps = timestamps.into_iter();
        let worker = thread::Builder::new()
            .name("log-forge-generation".into())
            .spawn(move || {
                let result = self
                    .run_controlled(timestamps, Some(&worker_state))
                    .map_err(|error| error.to_string());
                let (stats, close_error) = self.close_collect();
                if let Ok(mut status) = worker_state.status.lock() {
                    if let Ok(mut snapshot) = worker_state.statistics.lock() {
                        *snapshot = stats.clone();
                    }
                    status.running = false;
                    status.paused = false;
                }
                match (result, close_error) {
                    (Ok(_), None) => Ok(stats),
                    (Err(error), None) => Err(io::Error::other(error)),
                    (Ok(_), Some(error)) => Err(io::Error::other(error.to_string())),
                    (Err(first), Some(second)) => {
                        Err(io::Error::other(format!("{first}; close failed: {second}")))
                    }
                }
            })?;
        Ok(BackgroundRun {
            state,
            worker: Some(worker),
        })
    }

    fn close_collect(mut self) -> (GenerationStatistics, Option<Box<dyn Error>>) {
        let mut first_error: Option<Box<dyn Error>> = None;
        for entry in &mut self.generators {
            if let Err(error) = entry.generator.finish() {
                self.stats.add_error();
                notify_error_handlers(&self.error_handlers, error.as_ref());
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        for mut output in self.outputs {
            if let Err(error) = output.close() {
                self.stats.add_error();
                notify_error_handlers(&self.error_handlers, &error);
                if first_error.is_none() {
                    first_error = Some(Box::new(error));
                }
            }
        }
        self.stats.end_time = Some(Utc::now());
        self.stats.calculate_rate();
        (self.stats, first_error)
    }

    pub fn close(self) -> Result<GenerationStatistics, Box<dyn Error>> {
        let (stats, error) = self.close_collect();
        if let Some(error) = error {
            Err(error)
        } else {
            Ok(stats)
        }
    }
}
