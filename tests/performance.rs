use log_forge::performance::{
    BackgroundPerformanceMonitor, BatchProcessor, MemoryTracker, PerformanceMonitor,
    PerformanceProfiler,
};

#[test]
fn memory_tracker_reports_baseline_growth_and_limits() {
    let mut tracker = MemoryTracker::new().unwrap();
    let snapshot = tracker.snapshot().unwrap();
    assert!(snapshot.baseline_bytes > 0);
    assert!(snapshot.current_bytes > 0);
    assert_eq!(
        snapshot.growth_bytes,
        i128::from(snapshot.current_bytes) - i128::from(snapshot.baseline_bytes)
    );
    assert_eq!(
        snapshot.system_memory_bytes.is_some(),
        snapshot.system_percent.is_some()
    );
    if let Some(percent) = snapshot.system_percent {
        assert!(percent.is_finite() && percent >= 0.0);
    }
    assert!(tracker.check_limit(0).is_err());
    assert!(
        tracker
            .check_limit(1)
            .unwrap_err()
            .to_string()
            .contains("memory limit exceeded")
    );
    assert!(tracker.check_limit(u64::MAX).is_ok());
}

#[test]
fn batches_records_and_retries_only_the_failed_processor() {
    use std::{cell::RefCell, rc::Rc, time::Duration};
    assert!(BatchProcessor::new(0, Duration::from_secs(1)).is_err());
    let first = Rc::new(RefCell::new(Vec::new()));
    let second = Rc::new(RefCell::new(Vec::new()));
    let attempts = Rc::new(RefCell::new(0));
    let mut batch = BatchProcessor::new(2, Duration::from_secs(10)).unwrap();
    let output = Rc::clone(&first);
    batch
        .add_processor(move |items| {
            output.borrow_mut().push(items.to_vec());
            Ok(())
        })
        .unwrap();
    let output = Rc::clone(&second);
    let counter = Rc::clone(&attempts);
    batch
        .add_processor(move |items| {
            *counter.borrow_mut() += 1;
            if *counter.borrow() == 1 {
                return Err(std::io::Error::other("temporary failure"));
            }
            output.borrow_mut().push(items.to_vec());
            Ok(())
        })
        .unwrap();
    batch.add_item("one".into()).unwrap();
    assert!(batch.add_item("two".into()).is_err());
    assert_eq!(batch.pending_count(), 2);
    assert!(batch.add_item("three".into()).is_err());
    batch.flush().unwrap();
    assert_eq!(*first.borrow(), vec![vec!["one", "two"]]);
    assert_eq!(*second.borrow(), vec![vec!["one", "two"]]);
    assert_eq!(*attempts.borrow(), 2);
    batch.add_item("three".into()).unwrap();
    batch.flush().unwrap();
    assert_eq!(batch.pending_count(), 0);
    let mut no_processors = BatchProcessor::new(1, Duration::from_secs(1)).unwrap();
    assert!(no_processors.add_item("retained".into()).is_err());
    no_processors.add_processor(|_| Ok(())).unwrap();
    no_processors.flush().unwrap();
    assert_eq!(no_processors.pending_count(), 0);
}

#[test]
fn background_monitor_samples_notifies_and_stops_promptly() {
    use std::{sync::mpsc, time::Duration};
    let monitor = PerformanceMonitor::new(5).unwrap();
    assert!(BackgroundPerformanceMonitor::start(monitor, Duration::ZERO, |_| {}).is_err());
    let mut monitor = PerformanceMonitor::new(5).unwrap();
    monitor.record_metrics(42, 3.0, 1, 2).unwrap();
    let (sender, receiver) = mpsc::channel();
    let mut background =
        BackgroundPerformanceMonitor::start(monitor, Duration::from_secs(60), move |metrics| {
            sender.send(metrics.clone()).unwrap();
        })
        .unwrap();
    let sample = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(sample.logs_generated, 42);
    assert_eq!(sample.queue_size, 2);
    let shared = background.monitor();
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .current_metrics()
            .unwrap()
            .logs_generated,
        42
    );
    let start = std::time::Instant::now();
    background.stop().unwrap();
    assert!(start.elapsed() < Duration::from_secs(5));
    background.stop().unwrap();
}

#[test]
fn samples_process_metrics_with_bounded_history() {
    assert!(PerformanceMonitor::new(0).is_err());
    let mut monitor = PerformanceMonitor::new(2).unwrap();
    assert!(monitor.summary().is_none());
    let first = monitor.record_metrics(1, 2.0, 0, 0).unwrap();
    assert!(first.memory_usage_bytes > 0);
    assert_eq!(first.cpu_percent, None);
    assert!(monitor.record_metrics(1, f64::NAN, 0, 0).is_err());
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    let second = monitor.record_metrics(2, 4.0, 1, 3).unwrap();
    assert!(
        second
            .cpu_percent
            .is_some_and(|value| value.is_finite() && value >= 0.0)
    );
    monitor.record_metrics(3, 0.0, 1, 0).unwrap();
    assert_eq!(monitor.history().len(), 2);
    assert_eq!(monitor.current_metrics().unwrap().logs_generated, 3);
    let summary = monitor.summary().unwrap();
    assert_eq!(summary.total_measurements, 2);
    assert_eq!(summary.average_generation_rate, 4.0);
    assert!(summary.average_cpu_percent.is_some());
}

#[test]
fn collects_named_profiles_and_scoped_measurements() {
    let mut profiler = PerformanceProfiler::new();
    assert!(profiler.end_profile("missing").is_none());
    assert!(profiler.profile_stats("missing").is_none());
    profiler.start_profile("generate").unwrap();
    assert!(profiler.start_profile("generate").is_err());
    let first = profiler.end_profile("generate").unwrap();
    {
        let _span = profiler.profile("generate").unwrap();
        std::hint::black_box(1 + 1);
    }
    let stats = profiler.profile_stats("generate").unwrap();
    assert_eq!(stats.count, 2);
    assert!(stats.min <= first && stats.max >= first);
    assert_eq!(stats.average, stats.total / 2.0);
    assert_eq!(profiler.all_profiles().len(), 1);
    assert!(profiler.end_profile("generate").is_none());
    assert!(serde_json::to_string(profiler.all_profiles()).is_ok());
}

#[test]
fn work_pool_runs_tasks_and_reports_panics() {
    use log_forge::performance::WorkPool;
    assert!(WorkPool::new(0).is_err());
    let pool = WorkPool::new(2).unwrap();
    let first = pool.submit(|| 42).unwrap();
    let failed = pool.submit(|| panic!("expected test panic")).unwrap();
    assert_eq!(first.wait().unwrap(), 42);
    assert!(failed.wait().is_err());
    let stats = pool.wait_idle().unwrap();
    assert_eq!((stats.pending, stats.completed, stats.panicked), (0, 2, 1));
    assert_eq!(pool.submit(|| 7).unwrap().wait().unwrap(), 7);
    assert_eq!(pool.shutdown().unwrap().completed, 3);
}

#[test]
fn work_pool_runs_multiple_tasks_concurrently() {
    use log_forge::performance::WorkPool;
    use std::sync::{Arc, Condvar, Mutex, mpsc};
    use std::time::Duration;
    let pool = WorkPool::new(2).unwrap();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let (ready_tx, ready_rx) = mpsc::channel();
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let release = Arc::clone(&release);
            let ready_tx = ready_tx.clone();
            pool.submit(move || {
                ready_tx.send(()).unwrap();
                let guard = release.0.lock().unwrap();
                drop(release.1.wait_while(guard, |released| !*released).unwrap());
            })
            .unwrap()
        })
        .collect();
    let first = ready_rx.recv_timeout(Duration::from_secs(5));
    let second = ready_rx.recv_timeout(Duration::from_secs(5));
    *release.0.lock().unwrap() = true;
    release.1.notify_all();
    assert!(first.is_ok() && second.is_ok());
    for handle in handles {
        handle.wait().unwrap();
    }
    assert_eq!(pool.shutdown().unwrap().completed, 2);
}
