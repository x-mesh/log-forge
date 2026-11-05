use crate::control_state::{ControlState, apply_command};
use clap::Parser;
use interprocess::{
    local_socket::{
        GenericNamespaced, ListenerNonblockingMode, ListenerOptions, Stream, prelude::*,
    },
    os::windows::{local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor},
};
use serde_json::{Value, json};
use std::{
    error::Error,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use widestring::U16CString;

fn pipe_name(path: &Path) -> io::Result<interprocess::local_socket::Name<'_>> {
    let name = path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "control pipe name must be UTF-8",
        )
    })?;
    if name.is_empty() || name.contains(['/', '\\', ':']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "control pipe name must be a simple name",
        ));
    }
    name.to_ns_name::<GenericNamespaced>()
}

fn read_deadline(stream: &mut Stream, max: usize) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    while bytes.len() < max {
        let mut chunk = [0; 4096];
        let capacity = (max - bytes.len()).min(chunk.len());
        match stream.read(&mut chunk[..capacity]) {
            // Wine can report an empty read before a pipe response arrives.
            Ok(0) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "control response timed out",
                ));
            }
            Ok(count) => {
                if let Some(end) = chunk[..count].iter().position(|byte| *byte == b'\n') {
                    bytes.extend_from_slice(&chunk[..end]);
                    return Ok(bytes);
                }
                bytes.extend_from_slice(&chunk[..count]);
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(5))
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "control message too long",
    ))
}

fn write_deadline(stream: &mut Stream, mut bytes: &[u8]) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !bytes.is_empty() {
        // Interprocess uses a 512-byte default pipe buffer on Windows.
        let chunk = &bytes[..bytes.len().min(512)];
        match stream.write(chunk) {
            // A full nonblocking pipe can accept zero bytes until the reader drains it.
            Ok(0) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "control write timed out",
                ));
            }
            Ok(count) => bytes = &bytes[count..],
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(5))
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(crate) struct ControlServer {
    pub state: Arc<ControlState>,
    stop: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<io::Result<()>>>,
}
impl ControlServer {
    pub fn start(path: &Path, stopped: Arc<AtomicBool>) -> io::Result<Self> {
        let sddl = U16CString::from_str("D:P(A;;GA;;;OW)(A;;GA;;;SY)").map_err(io::Error::other)?;
        let security = SecurityDescriptor::deserialize(&sddl)?;
        let listener = ListenerOptions::new()
            .name(pipe_name(path)?)
            .security_descriptor(security)
            .nonblocking(ListenerNonblockingMode::Both)
            .create_sync()?;
        let state = Arc::new(ControlState::new(stopped));
        let shared = Arc::clone(&state);
        let (stop, receive) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("log-forge-control".into())
            .spawn(move || {
                loop {
                    match listener.accept() {
                        Ok(mut stream) => {
                            let result = (|| -> io::Result<()> {
                                let bytes = read_deadline(&mut stream, 256)?;
                                let command =
                                    std::str::from_utf8(&bytes).map_err(io::Error::other)?;
                                let show = apply_command(command.trim(), &shared)?;
                                let mut response = serde_json::to_vec(&shared.snapshot(show)?)?;
                                response.push(b'\n');
                                write_deadline(&mut stream, &response)
                            })();
                            if let Err(error) = result {
                                eprintln!("control request failed: {error}");
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                        Err(error) => return Err(error),
                    }
                    if receive.recv_timeout(Duration::from_millis(20)).is_ok() {
                        return Ok(());
                    }
                }
            })?;
        Ok(Self {
            state,
            stop: Some(stop),
            worker: Some(worker),
        })
    }
    pub fn finish(&mut self) -> io::Result<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| io::Error::other("control thread failed"))??;
        }
        Ok(())
    }
}
impl Drop for ControlServer {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[derive(Parser)]
struct ControlArgs {
    #[arg(long)]
    control_socket: PathBuf,
}

#[derive(Parser)]
struct MonitorArgs {
    #[arg(long)]
    control_socket: PathBuf,
    #[arg(short, long, default_value_t = 2.0)]
    refresh: f64,
    #[arg(long)]
    show_logs: bool,
}

fn request(path: &Path, action: &str) -> io::Result<Value> {
    let mut stream = Stream::connect(pipe_name(path)?)?;
    stream.set_nonblocking(true)?;
    write_deadline(&mut stream, format!("{action}\n").as_bytes())?;
    let response = read_deadline(&mut stream, 1_000_000)?;
    serde_json::from_slice(&response).map_err(io::Error::other)
}

pub(crate) fn command(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let action = args[1].clone();
    if action == "monitor" {
        let cli = MonitorArgs::try_parse_from(
            std::iter::once(args[0].clone()).chain(args.into_iter().skip(2)),
        )?;
        if !cli.refresh.is_finite() || cli.refresh <= 0.0 {
            return Err("refresh must be a positive finite number".into());
        }
        let delay = Duration::try_from_secs_f64(cli.refresh)?;
        if delay.is_zero() {
            return Err("refresh interval is too small".into());
        }
        let mut connected = false;
        loop {
            match request(
                &cli.control_socket,
                if cli.show_logs {
                    "monitor_logs"
                } else {
                    "status"
                },
            ) {
                Ok(response) => {
                    connected = true;
                    println!("{}", serde_json::to_string(&response)?);
                    io::stdout().flush()?;
                    if response["state"] == "stopping" {
                        break;
                    }
                }
                Err(error)
                    if connected
                        && matches!(
                            error.kind(),
                            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                        ) =>
                {
                    println!("{}", json!({"state":"disconnected"}));
                    break;
                }
                Err(error) => return Err(error.into()),
            }
            thread::sleep(delay);
        }
        return Ok(());
    }
    let cli = ControlArgs::try_parse_from(
        std::iter::once(args[0].clone()).chain(args.into_iter().skip(2)),
    )?;
    let response = request(&cli.control_socket, &action)?;
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn named_pipe_controls_run_and_releases_name() {
        let name = PathBuf::from(format!("log-forge-test-{}", std::process::id()));
        let stopped = Arc::new(AtomicBool::new(false));
        let mut server = ControlServer::start(&name, Arc::clone(&stopped)).unwrap();
        server.state.record("redis", b"sample\n").unwrap();
        assert_eq!(request(&name, "status").unwrap()["generated"], 1);
        assert_eq!(request(&name, "pause").unwrap()["state"], "paused");
        assert_eq!(
            request(&name, "monitor_logs").unwrap()["recent_logs"][0]["message"],
            "sample"
        );
        let long_record = "x".repeat(4096);
        for _ in 0..20 {
            server
                .state
                .record("redis", long_record.as_bytes())
                .unwrap();
        }
        let snapshot = request(&name, "monitor_logs").unwrap();
        assert_eq!(snapshot["recent_logs"].as_array().unwrap().len(), 20);
        assert_eq!(snapshot["recent_logs"][0]["message"], long_record);
        assert_eq!(request(&name, "resume").unwrap()["state"], "running");
        assert_eq!(request(&name, "stop").unwrap()["state"], "stopping");
        assert!(stopped.load(Ordering::SeqCst));
        server.finish().unwrap();
        let mut next = ControlServer::start(&name, Arc::new(AtomicBool::new(false))).unwrap();
        next.finish().unwrap();
    }
}
