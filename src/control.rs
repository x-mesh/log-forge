use crate::control_state::{ControlState, apply_command};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    error::Error,
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread::{self, JoinHandle},
    time::Duration,
};

pub(crate) struct ControlServer {
    pub state: Arc<ControlState>,
    stop: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<io::Result<()>>>,
    socket: Option<OwnedSocket>,
}

struct OwnedSocket {
    path: PathBuf,
    identity: (u64, u64),
}
impl OwnedSocket {
    fn remove(&self) -> io::Result<()> {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata)
                if metadata.file_type().is_socket()
                    && (metadata.dev(), metadata.ino()) == self.identity =>
            {
                fs::remove_file(&self.path)
            }
            Ok(_) => Err(io::Error::other("control socket path was replaced")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}
impl Drop for OwnedSocket {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

impl ControlServer {
    pub fn start(path: &Path, stopped: Arc<AtomicBool>) -> io::Result<Self> {
        let listener = UnixListener::bind(path)?;
        let metadata = fs::symlink_metadata(path)?;
        let socket = OwnedSocket {
            path: path.to_owned(),
            identity: (metadata.dev(), metadata.ino()),
        };
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let state = Arc::new(ControlState::new(stopped));
        let shared = Arc::clone(&state);
        let (stop, receive) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("log-forge-control".into())
            .spawn(move || {
                loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false)?;
                            let _ = handle(&mut stream, &shared);
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
            socket: Some(socket),
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
        if let Some(socket) = self.socket.take() {
            socket.remove()?;
        }
        Ok(())
    }
}
impl Drop for ControlServer {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn handle(stream: &mut UnixStream, state: &ControlState) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut command = String::new();
    BufReader::new(stream.try_clone()?)
        .take(256)
        .read_line(&mut command)?;
    let show_logs = apply_command(command.trim(), state)?;
    serde_json::to_writer(&mut *stream, &state.snapshot(show_logs)?)?;
    stream.write_all(b"\n")
}

#[derive(Parser)]
#[command(about = "Send a command to a running generator and print its JSON status")]
struct ControlArgs {
    #[arg(
        long,
        help = "Connect to the socket that start --control-socket created"
    )]
    control_socket: PathBuf,
}

#[derive(Parser)]
#[command(about = "Print the status of a running generator until it stops")]
struct MonitorArgs {
    #[arg(
        long,
        help = "Connect to the socket that start --control-socket created"
    )]
    control_socket: PathBuf,
    #[arg(
        short,
        long,
        default_value_t = 2.0,
        help = "Set the refresh interval in seconds"
    )]
    refresh: f64,
    #[arg(long, help = "Include recent records")]
    show_logs: bool,
}

fn request(path: &Path, action: &str) -> io::Result<Value> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(format!("{action}\n").as_bytes())?;
    let mut response = String::new();
    if BufReader::new(stream).read_line(&mut response)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "control socket closed without a response",
        ));
    }
    serde_json::from_str(&response).map_err(io::Error::other)
}

pub(crate) fn command(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let action = args[1].clone();
    if action == "monitor" {
        let cli = MonitorArgs::try_parse_from(
            std::iter::once(crate::app::command_name(&args[0], &action))
                .chain(args.into_iter().skip(2)),
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
                    writeln!(io::stdout(), "{}", serde_json::to_string(&response)?)
                        .map_err(crate::stdout_error)?;
                    io::stdout().flush().map_err(crate::stdout_error)?;
                    if response["state"] == "stopping" {
                        break;
                    }
                }
                // A stopping generator can accept a request and close before it replies.
                Err(error)
                    if connected
                        && matches!(
                            error.kind(),
                            io::ErrorKind::NotFound
                                | io::ErrorKind::ConnectionRefused
                                | io::ErrorKind::UnexpectedEof
                                | io::ErrorKind::NotConnected
                                | io::ErrorKind::ConnectionReset
                                | io::ErrorKind::BrokenPipe
                        ) =>
                {
                    writeln!(io::stdout(), "{}", json!({"state":"disconnected"}))
                        .map_err(crate::stdout_error)?;
                    break;
                }
                Err(error) => return Err(error.into()),
            }
            thread::sleep(delay);
        }
        return Ok(());
    }
    let cli = ControlArgs::try_parse_from(
        std::iter::once(crate::app::command_name(&args[0], &action))
            .chain(args.into_iter().skip(2)),
    )?;
    let response = request(&cli.control_socket, &action)?;
    writeln!(io::stdout(), "{}", serde_json::to_string(&response)?).map_err(crate::stdout_error)?;
    Ok(())
}
