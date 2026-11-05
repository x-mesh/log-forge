use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{IpAddr, TcpStream, ToSocketAddrs},
    time::Duration,
};

pub(crate) struct HttpOutput {
    host: String,
    port: u16,
    endpoint: String,
    timeout: Duration,
    content_type: &'static str,
}

pub(crate) fn validate_target(host: &str, endpoint: &str) -> io::Result<()> {
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid HTTP host",
        ));
    }
    if !endpoint.starts_with('/') || endpoint.chars().any(|c| !c.is_ascii_graphic() || c == '#') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid HTTP endpoint",
        ));
    }
    Ok(())
}

impl HttpOutput {
    pub(crate) fn target(&self) -> String {
        format!("{}:{}{}", self.host, self.port, self.endpoint)
    }
    pub(crate) fn set_json_content_type(&mut self) {
        self.content_type = "application/json";
    }

    pub fn new(
        host: &str,
        port: u16,
        endpoint: &str,
        timeout: Duration,
        json: bool,
    ) -> io::Result<Self> {
        validate_target(host, endpoint)?;
        Ok(Self {
            host: host.to_owned(),
            port,
            endpoint: endpoint.to_owned(),
            timeout,
            content_type: if json {
                "application/json"
            } else {
                "text/plain; charset=utf-8"
            },
        })
    }
    pub fn write(&self, body: &[u8]) -> io::Result<()> {
        let code = self.send(body, self.content_type, "")?;
        if !(200..300).contains(&code) {
            return Err(io::Error::other(format!("HTTP {code}")));
        }
        Ok(())
    }

    pub fn send(&self, body: &[u8], content_type: &str, headers: &str) -> io::Result<u16> {
        let address = (self.host.as_str(), self.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::other("HTTP host did not resolve"))?;
        let mut stream = TcpStream::connect_timeout(&address, self.timeout)?;
        stream.set_write_timeout(Some(self.timeout))?;
        stream.set_read_timeout(Some(self.timeout))?;
        let host = if matches!(self.host.parse::<IpAddr>(), Ok(IpAddr::V6(_))) {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        write!(
            stream,
            "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nUser-Agent: log-forge/{}\r\n{}\r\n",
            self.endpoint,
            host,
            self.port,
            content_type,
            body.len(),
            env!("CARGO_PKG_VERSION"),
            headers
        )?;
        stream.write_all(body)?;
        let mut reader = BufReader::new(stream.take(16_385));
        let mut status = String::new();
        reader.read_line(&mut status)?;
        let mut pieces = status.split_whitespace();
        let version = pieces
            .next()
            .ok_or_else(|| io::Error::other("missing HTTP response"))?;
        let code = pieces
            .next()
            .ok_or_else(|| io::Error::other("missing HTTP status"))?
            .parse::<u16>()
            .map_err(|_| io::Error::other("invalid HTTP status"))?;
        if !["HTTP/1.0", "HTTP/1.1"].contains(&version) {
            return Err(io::Error::other("unsupported HTTP response"));
        }
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Err(io::Error::other("incomplete HTTP headers"));
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        Ok(code)
    }
}
