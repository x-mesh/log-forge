use std::{
    io::{self, Write},
    net::{SocketAddr, TcpStream},
    thread,
    time::Duration,
};

pub(crate) struct TcpOutput {
    address: SocketAddr,
    timeout: Duration,
    max_retries: usize,
    retry_delay: Duration,
    stream: TcpStream,
    reconnects: u64,
}
impl TcpOutput {
    pub fn connect(
        address: SocketAddr,
        timeout: Duration,
        max_retries: usize,
        retry_delay: Duration,
    ) -> io::Result<Self> {
        let stream = Self::connect_stream(address, timeout)?;
        Ok(Self {
            address,
            timeout,
            max_retries,
            retry_delay,
            stream,
            reconnects: 0,
        })
    }
    fn connect_stream(address: SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
        let stream = TcpStream::connect_timeout(&address, timeout)?;
        stream.set_write_timeout(Some(timeout))?;
        Ok(stream)
    }
    pub fn write_record(&mut self, data: &[u8]) -> io::Result<()> {
        let mut written = 0;
        let mut retries = 0;
        while written < data.len() {
            match self.stream.write(&data[written..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "TCP connection wrote zero bytes",
                    ));
                }
                Ok(count) => written += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error)
                    if written == 0
                        && retries < self.max_retries
                        && matches!(
                            error.kind(),
                            io::ErrorKind::BrokenPipe
                                | io::ErrorKind::ConnectionReset
                                | io::ErrorKind::ConnectionAborted
                                | io::ErrorKind::NotConnected
                        ) =>
                {
                    let mut last_error = error;
                    let mut connected = false;
                    while retries < self.max_retries {
                        retries += 1;
                        eprintln!(
                            "TCP connection failed: {last_error}; reconnecting ({retries}/{})",
                            self.max_retries
                        );
                        let multiplier = u32::try_from(retries).unwrap_or(u32::MAX);
                        thread::sleep(self.retry_delay.saturating_mul(multiplier));
                        match Self::connect_stream(self.address, self.timeout) {
                            Ok(stream) => {
                                self.stream = stream;
                                self.reconnects += 1;
                                connected = true;
                                break;
                            }
                            Err(error) => last_error = error,
                        }
                    }
                    if !connected {
                        return Err(last_error);
                    }
                }
                Err(error) if written > 0 => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!(
                            "TCP write failed after {written} bytes; record was not retried: {error}"
                        ),
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    pub fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }

    pub fn reconnect_count(&self) -> u64 {
        self.reconnects
    }

    pub fn target(&self) -> SocketAddr {
        self.address
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, sync::mpsc};
    #[test]
    fn reconnects_after_reset_before_record_write() {
        use std::io::Read;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (ready, wait) = mpsc::channel();
        let (connected, wait_connected) = mpsc::channel();
        let server = thread::spawn(move || {
            let (first, _) = listener.accept().unwrap();
            wait_connected.recv().unwrap();
            socket2::SockRef::from(&first)
                .set_linger(Some(Duration::ZERO))
                .unwrap();
            drop(first);
            ready.send(()).unwrap();
            let (mut second, _) = listener.accept().unwrap();
            second
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut record = [0; 7];
            second.read_exact(&mut record).unwrap();
            record
        });
        let mut output = TcpOutput::connect(
            address,
            Duration::from_secs(2),
            2,
            Duration::from_millis(10),
        )
        .unwrap();
        connected.send(()).unwrap();
        wait.recv().unwrap();
        thread::sleep(Duration::from_millis(50));
        output.write_record(b"record\n").unwrap();
        assert_eq!(output.reconnect_count(), 1);
        assert_eq!(output.target(), address);
        assert_eq!(server.join().unwrap(), *b"record\n");
    }
    #[test]
    fn zero_retries_reports_the_reset() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (ready, wait) = mpsc::channel();
        let (connected, wait_connected) = mpsc::channel();
        let server = thread::spawn(move || {
            let (first, _) = listener.accept().unwrap();
            wait_connected.recv().unwrap();
            socket2::SockRef::from(&first)
                .set_linger(Some(Duration::ZERO))
                .unwrap();
            drop(first);
            ready.send(()).unwrap();
        });
        let mut output =
            TcpOutput::connect(address, Duration::from_secs(2), 0, Duration::ZERO).unwrap();
        connected.send(()).unwrap();
        wait.recv().unwrap();
        thread::sleep(Duration::from_millis(50));
        assert!(output.write_record(b"record\n").is_err());
        assert_eq!(output.reconnect_count(), 0);
        server.join().unwrap();
    }
}
