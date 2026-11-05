use crate::http_output::HttpOutput;
use prost::Message;
use std::{
    collections::BTreeMap,
    io, thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, PartialEq, Message)]
struct WriteRequest {
    #[prost(message, repeated, tag = "1")]
    timeseries: Vec<TimeSeries>,
}
#[derive(Clone, PartialEq, Message)]
struct TimeSeries {
    #[prost(message, repeated, tag = "1")]
    labels: Vec<Label>,
    #[prost(message, repeated, tag = "2")]
    samples: Vec<Sample>,
}
#[derive(Clone, PartialEq, Message)]
struct Label {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(string, tag = "2")]
    value: String,
}
#[derive(Clone, PartialEq, Message)]
struct Sample {
    #[prost(double, tag = "1")]
    value: f64,
    #[prost(int64, tag = "2")]
    timestamp: i64,
}

pub(crate) struct PrometheusOutput {
    http: HttpOutput,
    counts: BTreeMap<String, u64>,
    dirty: bool,
    last_timestamp: Option<i64>,
    closed: bool,
}
impl PrometheusOutput {
    pub fn new(http: HttpOutput) -> Self {
        Self {
            http,
            counts: BTreeMap::new(),
            dirty: false,
            last_timestamp: None,
            closed: false,
        }
    }
    pub fn record(&mut self, kind: &str) -> io::Result<()> {
        if self.closed {
            return Err(io::Error::other("Prometheus output is closed"));
        }
        let count = self.counts.entry(kind.to_owned()).or_default();
        *count = count
            .checked_add(1)
            .filter(|value| *value <= (1_u64 << 53))
            .ok_or_else(|| io::Error::other("Prometheus counter exceeds exact f64 range"))?;
        self.dirty = true;
        Ok(())
    }
    pub fn flush(&mut self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        self.send_samples(false)
    }
    pub fn close(&mut self) -> io::Result<()> {
        if self.closed {
            return Ok(());
        }
        self.flush()?;
        if self.last_timestamp.is_some() {
            self.send_samples(true)?;
        }
        self.closed = true;
        Ok(())
    }
    fn send_samples(&mut self, stale: bool) -> io::Result<()> {
        let timestamp = loop {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_millis();
            let now = i64::try_from(now).map_err(io::Error::other)?;
            match self.last_timestamp {
                Some(last) if now < last => {
                    return Err(io::Error::other("system clock moved backward"));
                }
                Some(last) if now == last => thread::sleep(Duration::from_millis(1)),
                _ => break now,
            }
        };
        let timeseries = self
            .counts
            .iter()
            .map(|(kind, count)| TimeSeries {
                labels: vec![
                    Label {
                        name: "__name__".into(),
                        value: "log_forge_records_total".into(),
                    },
                    Label {
                        name: "log_type".into(),
                        value: kind.clone(),
                    },
                ],
                samples: vec![Sample {
                    value: if stale {
                        f64::from_bits(0x7ff0_0000_0000_0002)
                    } else {
                        *count as f64
                    },
                    timestamp,
                }],
            })
            .collect();
        let encoded = WriteRequest { timeseries }.encode_to_vec();
        let body = snap::raw::Encoder::new()
            .compress_vec(&encoded)
            .map_err(io::Error::other)?;
        for attempt in 0..4 {
            let code = self.http.send(
                &body,
                "application/x-protobuf",
                "Content-Encoding: snappy\r\nX-Prometheus-Remote-Write-Version: 0.1.0\r\n",
            )?;
            if (200..300).contains(&code) {
                self.last_timestamp = Some(timestamp);
                if !stale {
                    self.dirty = false;
                }
                return Ok(());
            }
            if !(500..600).contains(&code) || attempt == 3 {
                return Err(io::Error::other(format!("HTTP {code}")));
            }
            eprintln!("Prometheus endpoint returned HTTP {code}; retrying");
            thread::sleep(Duration::from_millis(100 * (1 << attempt)));
        }
        unreachable!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    #[test]
    fn sends_snappy_protobuf_and_retries_server_errors() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let mut last_timestamp = 0;
            for (index, code) in [500, 204, 204].into_iter().enumerate() {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let header = String::from_utf8(request).unwrap();
                assert!(header.contains("Content-Encoding: snappy\r\n"));
                assert!(header.contains("Content-Type: application/x-protobuf\r\n"));
                assert!(header.contains("X-Prometheus-Remote-Write-Version: 0.1.0\r\n"));
                let length: usize = header
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut body = vec![0; length];
                stream.read_exact(&mut body).unwrap();
                let bytes = snap::raw::Decoder::new().decompress_vec(&body).unwrap();
                let request = WriteRequest::decode(bytes.as_slice()).unwrap();
                assert_eq!(request.timeseries[0].labels[1].value, "redis");
                let sample = &request.timeseries[0].samples[0];
                if index == 2 {
                    assert_eq!(sample.value.to_bits(), 0x7ff0_0000_0000_0002);
                    assert!(sample.timestamp > last_timestamp);
                } else {
                    assert_eq!(sample.value, 2.0);
                    last_timestamp = sample.timestamp;
                }
                write!(
                    stream,
                    "HTTP/1.1 {code} status\r\nContent-Length: 0\r\n\r\n"
                )
                .unwrap();
            }
        });
        let http = HttpOutput::new(
            "127.0.0.1",
            port,
            "/api/v1/write",
            Duration::from_secs(3),
            false,
        )
        .unwrap();
        let mut output = PrometheusOutput::new(http);
        output.record("redis").unwrap();
        output.record("redis").unwrap();
        output.flush().unwrap();
        output.flush().unwrap();
        output.close().unwrap();
        output.close().unwrap();
        assert!(output.record("redis").is_err());
        server.join().unwrap();
    }
    #[test]
    fn stale_marker_failure_is_reported() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            for code in [204, 400] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut header = Vec::new();
                let mut byte = [0];
                while !header.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    header.push(byte[0]);
                }
                let header = String::from_utf8(header).unwrap();
                let length: usize = header
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut body = vec![0; length];
                stream.read_exact(&mut body).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 {code} status\r\nContent-Length: 0\r\n\r\n"
                )
                .unwrap();
            }
        });
        let http = HttpOutput::new(
            "127.0.0.1",
            port,
            "/api/v1/write",
            Duration::from_secs(3),
            false,
        )
        .unwrap();
        let mut output = PrometheusOutput::new(http);
        output.record("redis").unwrap();
        let error = output.close().unwrap_err();
        assert!(error.to_string().contains("HTTP 400"));
        server.join().unwrap();
    }
}
