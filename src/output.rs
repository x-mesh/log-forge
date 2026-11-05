use chrono::{DateTime, Utc};
use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    io::{self, BufRead},
    path::Path,
    time::Duration,
};

pub trait OutputSink: Send {
    fn write(
        &mut self,
        kind: &str,
        timestamp: DateTime<Utc>,
        record: &str,
    ) -> Result<(), Box<dyn Error>>;
    fn write_with_level(
        &mut self,
        kind: &str,
        timestamp: DateTime<Utc>,
        record: &str,
        _level: Option<&str>,
    ) -> Result<(), Box<dyn Error>> {
        self.write(kind, timestamp, record)
    }
    fn write_entry(&mut self, _entry: &crate::models::LogEntry) -> Result<(), Box<dyn Error>> {
        // Existing sinks can accept text without preserving structured fields.
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "structured entries are not supported by this sink",
        )
        .into())
    }
    fn flush(&mut self) -> io::Result<()>;
    fn close(&mut self) -> io::Result<()>;
}

#[derive(Clone, Debug)]
pub struct RotationOptions {
    pub size_bytes: Option<u64>,
    pub interval_seconds: Option<u64>,
    pub max_files: usize,
    pub buffer_size: usize,
    pub compress: bool,
}

impl Default for RotationOptions {
    fn default() -> Self {
        Self {
            size_bytes: None,
            interval_seconds: None,
            max_files: 10,
            buffer_size: 8192,
            compress: false,
        }
    }
}

pub struct OutputHandler {
    inner: crate::app::compat::OutputHandle,
    accepted_records: u64,
    failed_records: u64,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct OutputStatistics {
    pub accepted_records: u64,
    pub failed_records: u64,
    pub reconnect_count: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct NetworkInfo {
    pub protocol: String,
    pub target: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct FileInfo {
    pub path: std::path::PathBuf,
    pub size_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct FileStatistics {
    pub path: std::path::PathBuf,
    pub size_bytes: u64,
    pub line_count: u64,
    pub recent_logs: Vec<String>,
    pub active: bool,
}

impl serde::Serialize for FileStatistics {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("FileStatistics", 7)?;
        state.serialize_field("path", &self.path)?;
        state.serialize_field("size_bytes", &self.size_bytes)?;
        state.serialize_field("file_path", &self.path)?;
        state.serialize_field("file_size", &self.size_bytes)?;
        state.serialize_field("line_count", &self.line_count)?;
        state.serialize_field("recent_logs", &self.recent_logs)?;
        state.serialize_field("active", &self.active)?;
        state.end()
    }
}

impl OutputHandler {
    pub fn file(path: &Path, rotation: RotationOptions) -> io::Result<Self> {
        Ok(Self {
            inner: crate::app::compat::OutputHandle::file(path, rotation)?,
            accepted_records: 0,
            failed_records: 0,
        })
    }

    pub fn multi_file(
        base: &Path,
        pattern: &str,
        date_format: &str,
        rotation: RotationOptions,
    ) -> io::Result<Self> {
        Ok(Self {
            inner: crate::app::compat::OutputHandle::multi_file(
                base,
                pattern,
                date_format,
                rotation,
            )?,
            accepted_records: 0,
            failed_records: 0,
        })
    }

    pub fn console() -> Self {
        Self {
            inner: crate::app::compat::OutputHandle::console(),
            accepted_records: 0,
            failed_records: 0,
        }
    }

    pub fn network(
        host: &str,
        port: u16,
        protocol: &str,
        timeout: Duration,
        max_retries: usize,
        retry_delay: Duration,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            inner: crate::app::compat::OutputHandle::network(
                host,
                port,
                protocol,
                timeout,
                max_retries,
                retry_delay,
            )?,
            accepted_records: 0,
            failed_records: 0,
        })
    }

    pub fn http(host: &str, port: u16, endpoint: &str, timeout: Duration) -> io::Result<Self> {
        Ok(Self {
            inner: crate::app::compat::OutputHandle::http(host, port, endpoint, timeout)?,
            accepted_records: 0,
            failed_records: 0,
        })
    }

    pub fn json(self, mapping: BTreeMap<String, String>, include_raw: bool, pretty: bool) -> Self {
        Self {
            inner: self.inner.json(
                mapping,
                include_raw,
                pretty,
                crate::json_output::JsonFormat::Standard,
            ),
            accepted_records: self.accepted_records,
            failed_records: self.failed_records,
        }
    }

    pub fn elk_json(
        self,
        mapping: BTreeMap<String, String>,
        include_raw: bool,
        pretty: bool,
    ) -> Self {
        Self {
            inner: self.inner.json(
                mapping,
                include_raw,
                pretty,
                crate::json_output::JsonFormat::Elk,
            ),
            accepted_records: self.accepted_records,
            failed_records: self.failed_records,
        }
    }

    pub fn splunk_json(
        self,
        mapping: BTreeMap<String, String>,
        include_raw: bool,
        pretty: bool,
    ) -> Self {
        Self {
            inner: self.inner.json(
                mapping,
                include_raw,
                pretty,
                crate::json_output::JsonFormat::Splunk,
            ),
            accepted_records: self.accepted_records,
            failed_records: self.failed_records,
        }
    }

    pub fn fanout(outputs: Vec<Self>) -> io::Result<Self> {
        if outputs.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "outputs must not be empty",
            ));
        }
        Ok(Self {
            inner: crate::app::compat::OutputHandle::fanout(
                outputs.into_iter().map(|output| output.inner).collect(),
            ),
            accepted_records: 0,
            failed_records: 0,
        })
    }

    pub fn write(
        &mut self,
        kind: &str,
        timestamp: DateTime<Utc>,
        record: &str,
    ) -> Result<(), Box<dyn Error>> {
        self.write_with_level(kind, timestamp, record, None)
    }

    pub fn write_entry(&mut self, entry: &crate::models::LogEntry) -> Result<(), Box<dyn Error>> {
        match self.inner.write_entry(entry) {
            Ok(()) => {
                self.accepted_records += 1;
                Ok(())
            }
            Err(error) => {
                self.failed_records += 1;
                Err(error)
            }
        }
    }

    pub fn write_with_level(
        &mut self,
        kind: &str,
        timestamp: DateTime<Utc>,
        record: &str,
        level: Option<&str>,
    ) -> Result<(), Box<dyn Error>> {
        let mut data = record.as_bytes().to_vec();
        if !data.ends_with(b"\n") {
            data.push(b'\n');
        }
        match self.inner.write_with_level(kind, timestamp, &data, level) {
            Ok(()) => {
                self.accepted_records += 1;
                Ok(())
            }
            Err(error) => {
                self.failed_records += 1;
                Err(error)
            }
        }
    }

    pub fn statistics(&self) -> OutputStatistics {
        OutputStatistics {
            accepted_records: self.accepted_records,
            failed_records: self.failed_records,
            reconnect_count: self.inner.reconnect_count(),
        }
    }

    pub fn network_info(&self) -> io::Result<Option<NetworkInfo>> {
        self.inner.network_info()
    }

    pub fn test_connection(&mut self) -> Result<(), Box<dyn Error>> {
        if self.network_info()?.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "connection test requires a network output",
            )
            .into());
        }
        self.write("test", chrono::Utc::now(), "test")
    }

    pub fn field_mapping(&self) -> io::Result<BTreeMap<String, String>> {
        self.inner.field_mapping()
    }

    pub fn set_field_mapping(&mut self, mapping: BTreeMap<String, String>) -> io::Result<()> {
        self.inner.set_field_mapping(mapping)
    }

    pub fn file_info(&self) -> Option<FileInfo> {
        self.inner.file_info()
    }

    pub fn force_rotation(&mut self) -> io::Result<()> {
        self.inner.force_rotation()
    }

    pub fn file_info_for_type(&self, kind: &str) -> io::Result<Option<FileInfo>> {
        self.inner.file_info_for_type(kind)
    }

    pub fn force_rotation_for_type(&mut self, kind: &str) -> io::Result<()> {
        self.inner.force_rotation_for_type(kind)
    }

    pub fn active_file_types(&self) -> Vec<String> {
        self.inner.active_file_types()
    }

    pub fn file_statistics(&mut self) -> io::Result<BTreeMap<String, FileStatistics>> {
        if !self.inner.is_multi_file() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "file statistics require multi-file output",
            ));
        }
        self.flush()?;
        let mut result = BTreeMap::new();
        for kind in self.active_file_types() {
            let info = self
                .file_info_for_type(&kind)?
                .ok_or_else(|| io::Error::other("active file type has no file"))?;
            let reader = io::BufReader::new(std::fs::File::open(&info.path)?);
            let mut recent = VecDeque::new();
            let mut line_count = 0_u64;
            for line in reader.lines() {
                let line = line?;
                line_count = line_count
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("line count overflow"))?;
                if recent.len() == 3 {
                    recent.pop_front();
                }
                recent.push_back(line.trim().to_owned());
            }
            result.insert(
                kind,
                FileStatistics {
                    path: info.path,
                    size_bytes: info.size_bytes,
                    line_count,
                    recent_logs: recent.into_iter().filter(|line| !line.is_empty()).collect(),
                    active: true,
                },
            );
        }
        Ok(result)
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }

    pub fn close(mut self) -> io::Result<()> {
        self.inner.close()
    }
}

impl OutputSink for OutputHandler {
    fn write(
        &mut self,
        kind: &str,
        timestamp: DateTime<Utc>,
        record: &str,
    ) -> Result<(), Box<dyn Error>> {
        OutputHandler::write(self, kind, timestamp, record)
    }

    fn flush(&mut self) -> io::Result<()> {
        OutputHandler::flush(self)
    }

    fn write_with_level(
        &mut self,
        kind: &str,
        timestamp: DateTime<Utc>,
        record: &str,
        level: Option<&str>,
    ) -> Result<(), Box<dyn Error>> {
        OutputHandler::write_with_level(self, kind, timestamp, record, level)
    }

    fn write_entry(&mut self, entry: &crate::models::LogEntry) -> Result<(), Box<dyn Error>> {
        OutputHandler::write_entry(self, entry)
    }

    fn close(&mut self) -> io::Result<()> {
        self.inner.close()
    }
}
