use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct WorkPoolStatistics {
    pub pending: usize,
    pub completed: u64,
    pub panicked: u64,
}

pub struct WorkPool {
    pool: rayon::ThreadPool,
    state: Arc<(Mutex<WorkPoolStatistics>, Condvar)>,
}

pub struct WorkHandle<R> {
    receiver: mpsc::Receiver<Result<R, &'static str>>,
}

impl WorkPool {
    pub fn new(workers: usize) -> io::Result<Self> {
        if workers == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "worker count must be greater than zero",
            ));
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .map_err(io::Error::other)?;
        Ok(Self {
            pool,
            state: Arc::new((Mutex::new(WorkPoolStatistics::default()), Condvar::new())),
        })
    }

    pub fn submit<F, R>(&self, task: F) -> io::Result<WorkHandle<R>>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let state = Arc::clone(&self.state);
        {
            let mut stats = state
                .0
                .lock()
                .map_err(|_| io::Error::other("work pool state poisoned"))?;
            stats.pending = stats
                .pending
                .checked_add(1)
                .ok_or_else(|| io::Error::other("too many pending tasks"))?;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        self.pool.spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
            let panicked = result.is_err();
            drop(sender.send(result.map_err(|_| "work item panicked")));
            let mut stats = state.0.lock().unwrap_or_else(|error| error.into_inner());
            stats.pending -= 1;
            stats.completed += 1;
            stats.panicked += u64::from(panicked);
            state.1.notify_all();
        });
        Ok(WorkHandle { receiver })
    }

    pub fn statistics(&self) -> io::Result<WorkPoolStatistics> {
        self.state
            .0
            .lock()
            .map(|stats| *stats)
            .map_err(|_| io::Error::other("work pool state poisoned"))
    }

    pub fn wait_idle(&self) -> io::Result<WorkPoolStatistics> {
        if self.pool.current_thread_index().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "cannot wait for a work pool from its worker",
            ));
        }
        let stats = self
            .state
            .0
            .lock()
            .map_err(|_| io::Error::other("work pool state poisoned"))?;
        let stats = self
            .state
            .1
            .wait_while(stats, |stats| stats.pending > 0)
            .map_err(|_| io::Error::other("work pool state poisoned"))?;
        Ok(*stats)
    }

    pub fn shutdown(self) -> io::Result<WorkPoolStatistics> {
        self.wait_idle()
    }
}

impl<R> WorkHandle<R> {
    pub fn wait(self) -> io::Result<R> {
        self.receiver
            .recv()
            .map_err(|_| io::Error::other("work result unavailable"))?
            .map_err(io::Error::other)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MemorySnapshot {
    pub baseline_bytes: u64,
    pub current_bytes: u64,
    pub growth_bytes: i128,
    pub system_memory_bytes: Option<u64>,
    pub system_percent: Option<f64>,
}

pub struct MemoryTracker {
    baseline_bytes: u64,
    system: System,
}

impl MemoryTracker {
    pub fn new() -> io::Result<Self> {
        let baseline_bytes = memory_stats::memory_stats()
            .ok_or_else(|| io::Error::other("cannot read process memory usage"))?
            .physical_mem as u64;
        let system = System::new().map_err(io::Error::other)?;
        Ok(Self {
            baseline_bytes,
            system,
        })
    }

    pub fn snapshot(&mut self) -> io::Result<MemorySnapshot> {
        let current = memory_stats::memory_stats()
            .ok_or_else(|| io::Error::other("cannot read process memory usage"))?
            .physical_mem as u64;
        self.system
            .refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());
        let total = self.system.total_memory();
        Ok(MemorySnapshot {
            baseline_bytes: self.baseline_bytes,
            current_bytes: current,
            growth_bytes: i128::from(current) - i128::from(self.baseline_bytes),
            system_memory_bytes: (total > 0).then_some(total),
            system_percent: (total > 0).then(|| current as f64 * 100.0 / total as f64),
        })
    }

    pub fn check_limit(&mut self, limit_bytes: u64) -> io::Result<MemorySnapshot> {
        if limit_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "memory limit must be positive",
            ));
        }
        let snapshot = self.snapshot()?;
        if snapshot.current_bytes > limit_bytes {
            return Err(io::Error::other(format!(
                "memory limit exceeded: {} bytes used, limit {limit_bytes} bytes",
                snapshot.current_bytes
            )));
        }
        Ok(snapshot)
    }
}

type BatchHandler = dyn FnMut(&[String]) -> io::Result<()>;

pub struct BatchProcessor {
    batch_size: usize,
    flush_interval: Duration,
    last_flush: Instant,
    batch: Vec<String>,
    processors: Vec<Box<BatchHandler>>,
    next_processor: usize,
    failed: bool,
}

impl BatchProcessor {
    pub fn new(batch_size: usize, flush_interval: Duration) -> io::Result<Self> {
        if batch_size == 0 || flush_interval.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "batch size and flush interval must be positive",
            ));
        }
        Ok(Self {
            batch_size,
            flush_interval,
            last_flush: Instant::now(),
            batch: Vec::new(),
            processors: Vec::new(),
            next_processor: 0,
            failed: false,
        })
    }

    pub fn add_processor<F>(&mut self, processor: F) -> io::Result<()>
    where
        F: FnMut(&[String]) -> io::Result<()> + 'static,
    {
        if self.failed && !self.processors.is_empty() {
            return Err(io::Error::other(
                "flush the failed batch before adding a processor",
            ));
        }
        self.processors.push(Box::new(processor));
        Ok(())
    }

    pub fn add_item(&mut self, item: String) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::other(
                "flush the failed batch before adding an item",
            ));
        }
        self.batch.push(item);
        if self.batch.len() >= self.batch_size || self.last_flush.elapsed() >= self.flush_interval {
            self.flush()?;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        if self.batch.is_empty() {
            return Ok(());
        }
        if self.processors.is_empty() {
            self.failed = true;
            return Err(io::Error::other("no batch processors configured"));
        }
        while self.next_processor < self.processors.len() {
            if let Err(error) = (self.processors[self.next_processor])(&self.batch) {
                self.failed = true;
                return Err(error);
            }
            self.next_processor += 1;
        }
        self.batch.clear();
        self.next_processor = 0;
        self.failed = false;
        self.last_flush = Instant::now();
        Ok(())
    }

    pub fn pending_count(&self) -> usize {
        self.batch.len()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PerformanceMetrics {
    pub timestamp: String,
    pub logs_generated: u64,
    pub generation_rate: f64,
    pub memory_usage_bytes: u64,
    pub cpu_percent: Option<f32>,
    pub thread_count: Option<usize>,
    pub queue_size: usize,
    pub error_count: u64,
}

pub struct BackgroundPerformanceMonitor {
    monitor: Arc<Mutex<PerformanceMonitor>>,
    stop: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl BackgroundPerformanceMonitor {
    pub fn start<F>(
        monitor: PerformanceMonitor,
        interval: Duration,
        mut callback: F,
    ) -> io::Result<Self>
    where
        F: FnMut(&PerformanceMetrics) + Send + 'static,
    {
        if interval.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "monitor interval must be positive",
            ));
        }
        let monitor = Arc::new(Mutex::new(monitor));
        let worker_monitor = Arc::clone(&monitor);
        let (stop, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("log-forge-performance".into())
            .spawn(move || {
                loop {
                    let metrics = {
                        let mut monitor = worker_monitor
                            .lock()
                            .map_err(|_| io::Error::other("performance monitor lock poisoned"))?;
                        let previous = monitor.current_metrics().cloned();
                        monitor
                            .record_metrics(
                                previous.as_ref().map_or(0, |m| m.logs_generated),
                                previous.as_ref().map_or(0.0, |m| m.generation_rate),
                                previous.as_ref().map_or(0, |m| m.error_count),
                                previous.as_ref().map_or(0, |m| m.queue_size),
                            )?
                            .clone()
                    };
                    callback(&metrics);
                    match receiver.recv_timeout(interval) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                        Err(mpsc::RecvTimeoutError::Timeout) => (),
                    }
                }
            })?;
        Ok(Self {
            monitor,
            stop: Some(stop),
            worker: Some(worker),
        })
    }

    pub fn monitor(&self) -> Arc<Mutex<PerformanceMonitor>> {
        Arc::clone(&self.monitor)
    }

    pub fn stop(&mut self) -> io::Result<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        match self.worker.take() {
            Some(worker) => worker
                .join()
                .map_err(|_| io::Error::other("performance monitor thread failed"))?,
            None => Ok(()),
        }
    }
}

impl Drop for BackgroundPerformanceMonitor {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[derive(Debug, Serialize)]
pub struct PerformanceSummary {
    pub total_measurements: usize,
    pub average_memory_bytes: f64,
    pub peak_memory_bytes: u64,
    pub average_cpu_percent: Option<f64>,
    pub peak_cpu_percent: Option<f32>,
    pub average_generation_rate: f64,
    pub peak_generation_rate: f64,
}

pub struct PerformanceMonitor {
    system: System,
    pid: Pid,
    last_cpu_refresh: Instant,
    max_history: usize,
    history: VecDeque<PerformanceMetrics>,
}

impl PerformanceMonitor {
    pub fn new(max_history: usize) -> io::Result<Self> {
        if max_history == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "max_history must be positive",
            ));
        }
        let mut system = System::new().map_err(io::Error::other)?;
        let pid = sysinfo::get_current_pid().map_err(io::Error::other)?;
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_tasks(),
        );
        if system.process(pid).is_none() {
            return Err(io::Error::other("current process metrics are unavailable"));
        }
        Ok(Self {
            system,
            pid,
            last_cpu_refresh: Instant::now(),
            max_history,
            history: VecDeque::new(),
        })
    }

    pub fn record_metrics(
        &mut self,
        logs_generated: u64,
        generation_rate: f64,
        error_count: u64,
        queue_size: usize,
    ) -> io::Result<&PerformanceMetrics> {
        if !generation_rate.is_finite() || generation_rate < 0.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "generation rate must be finite and nonnegative",
            ));
        }
        let cpu_ready = self.last_cpu_refresh.elapsed() >= sysinfo::MINIMUM_CPU_UPDATE_INTERVAL;
        let mut refresh = ProcessRefreshKind::nothing().with_memory().with_tasks();
        if cpu_ready {
            refresh = refresh.with_cpu();
        }
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            refresh,
        );
        let process = self
            .system
            .process(self.pid)
            .ok_or_else(|| io::Error::other("current process metrics are unavailable"))?;
        if cpu_ready {
            self.last_cpu_refresh = Instant::now();
        }
        let metrics = PerformanceMetrics {
            timestamp: chrono::Utc::now().to_rfc3339(),
            logs_generated,
            generation_rate,
            memory_usage_bytes: process.memory(),
            cpu_percent: cpu_ready.then(|| process.cpu_usage()),
            thread_count: process.tasks().map(|tasks| tasks.len()),
            queue_size,
            error_count,
        };
        if self.history.len() == self.max_history {
            self.history.pop_front();
        }
        self.history.push_back(metrics);
        self.history
            .back()
            .ok_or_else(|| io::Error::other("metrics history is empty"))
    }

    pub fn history(&self) -> &VecDeque<PerformanceMetrics> {
        &self.history
    }

    pub fn current_metrics(&self) -> Option<&PerformanceMetrics> {
        self.history.back()
    }

    pub fn summary(&self) -> Option<PerformanceSummary> {
        if self.history.is_empty() {
            return None;
        }
        let count = self.history.len();
        let cpu: Vec<f32> = self.history.iter().filter_map(|m| m.cpu_percent).collect();
        let rates: Vec<f64> = self
            .history
            .iter()
            .map(|m| m.generation_rate)
            .filter(|rate| *rate > 0.0)
            .collect();
        Some(PerformanceSummary {
            total_measurements: count,
            average_memory_bytes: self
                .history
                .iter()
                .map(|m| m.memory_usage_bytes as f64)
                .sum::<f64>()
                / count as f64,
            peak_memory_bytes: self
                .history
                .iter()
                .map(|m| m.memory_usage_bytes)
                .max()
                .unwrap_or(0),
            average_cpu_percent: (!cpu.is_empty())
                .then(|| cpu.iter().map(|v| *v as f64).sum::<f64>() / cpu.len() as f64),
            peak_cpu_percent: cpu.into_iter().reduce(f32::max),
            average_generation_rate: if rates.is_empty() {
                0.0
            } else {
                rates.iter().sum::<f64>() / rates.len() as f64
            },
            peak_generation_rate: rates.into_iter().reduce(f64::max).unwrap_or(0.0),
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ProfileStatistics {
    pub count: u64,
    pub total: f64,
    pub average: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Default)]
pub struct PerformanceProfiler {
    active: BTreeMap<String, Instant>,
    profiles: BTreeMap<String, ProfileStatistics>,
}

impl PerformanceProfiler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start_profile(&mut self, name: &str) -> io::Result<()> {
        if self.active.contains_key(name) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "profile is already active",
            ));
        }
        self.active.insert(name.to_owned(), Instant::now());
        Ok(())
    }

    pub fn end_profile(&mut self, name: &str) -> Option<f64> {
        let duration = self.active.remove(name)?.elapsed().as_secs_f64();
        let stats = self
            .profiles
            .entry(name.to_owned())
            .or_insert(ProfileStatistics {
                count: 0,
                total: 0.0,
                average: 0.0,
                min: duration,
                max: duration,
            });
        stats.count += 1;
        stats.total += duration;
        stats.average = stats.total / stats.count as f64;
        stats.min = stats.min.min(duration);
        stats.max = stats.max.max(duration);
        Some(duration)
    }

    pub fn profile_stats(&self, name: &str) -> Option<&ProfileStatistics> {
        self.profiles.get(name)
    }

    pub fn all_profiles(&self) -> &BTreeMap<String, ProfileStatistics> {
        &self.profiles
    }

    pub fn profile(&mut self, name: &str) -> io::Result<ProfileSpan<'_>> {
        self.start_profile(name)?;
        Ok(ProfileSpan {
            profiler: self,
            name: name.to_owned(),
        })
    }
}

pub struct ProfileSpan<'a> {
    profiler: &'a mut PerformanceProfiler,
    name: String,
}

impl Drop for ProfileSpan<'_> {
    fn drop(&mut self) {
        self.profiler.end_profile(&self.name);
    }
}
