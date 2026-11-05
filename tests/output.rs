use chrono::{TimeZone, Utc};
use log_forge::output::{OutputHandler, RotationOptions};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, UdpSocket},
    time::Duration,
};

#[test]
fn network_connection_test_sends_a_record_and_updates_counts() {
    let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut output = OutputHandler::network(
        "127.0.0.1",
        receiver.local_addr().unwrap().port(),
        "udp",
        Duration::from_secs(1),
        0,
        Duration::ZERO,
    )
    .unwrap();
    output.test_connection().unwrap();
    let mut bytes = [0; 32];
    let length = receiver.recv(&mut bytes).unwrap();
    assert_eq!(&bytes[..length], b"test\n");
    assert_eq!(output.statistics().accepted_records, 1);
    assert!(OutputHandler::console().test_connection().is_err());
}

#[test]
fn scenario_json_uses_header_level() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("scenario.json");
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .json(BTreeMap::new(), false, false);
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    output
        .write(
            "scenario",
            time,
            "2024-01-01T00:00:00+00:00 [trace] scenario=example pattern=sample ERROR in message",
        )
        .unwrap();
    output.close().unwrap();
    let record: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(record["level"], "TRACE");
}

#[test]
fn network_connection_test_uses_json_format_and_reports_http_failure() {
    let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut json_output = OutputHandler::network(
        "127.0.0.1",
        receiver.local_addr().unwrap().port(),
        "udp",
        Duration::from_secs(1),
        0,
        Duration::ZERO,
    )
    .unwrap()
    .json(BTreeMap::new(), false, false);
    json_output.test_connection().unwrap();
    let mut bytes = [0; 4096];
    let length = receiver.recv(&mut bytes).unwrap();
    let record: serde_json::Value = serde_json::from_slice(&bytes[..length]).unwrap();
    assert_eq!(record["message"], "test");
    assert_eq!(record["log_type"], "test");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 4096];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let length: usize = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        while request.len() - header_end < length {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    let mut http = OutputHandler::http("127.0.0.1", port, "/test", Duration::from_secs(3)).unwrap();
    assert!(
        http.test_connection()
            .unwrap_err()
            .to_string()
            .contains("503")
    );
    server.join().unwrap();
    assert_eq!(http.statistics().accepted_records, 0);
    assert_eq!(http.statistics().failed_records, 1);
}

#[test]
fn json_field_mappings_can_be_inspected_and_updated() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.json");
    let mut plain = OutputHandler::file(&path, RotationOptions::default()).unwrap();
    assert!(plain.field_mapping().is_err());
    assert!(plain.set_field_mapping(BTreeMap::new()).is_err());
    let mut output = plain.json(BTreeMap::new(), false, false);
    let mut snapshot = output.field_mapping().unwrap();
    assert_eq!(snapshot["message"], "message");
    assert_eq!(snapshot["source_ip"], "source.ip");
    snapshot.insert("message".into(), "ignored".into());
    assert_eq!(output.field_mapping().unwrap()["message"], "message");
    output
        .set_field_mapping(BTreeMap::from([("message".into(), "event.detail".into())]))
        .unwrap();
    assert_eq!(output.field_mapping().unwrap()["message"], "event.detail");
    assert!(
        output
            .set_field_mapping(BTreeMap::from([("message".into(), "@timestamp".into())]))
            .is_err()
    );
    assert_eq!(output.field_mapping().unwrap()["message"], "event.detail");
    output
        .write(
            "redis",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            "record",
        )
        .unwrap();
    output.close().unwrap();
    let record: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(record["event"]["detail"], "record");
}

#[test]
fn json_mapping_updates_reject_nested_and_reserved_collisions_atomically() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.json");
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .json(BTreeMap::new(), true, false);
    for changes in [
        BTreeMap::from([("message".to_owned(), "http.response".to_owned())]),
        BTreeMap::from([("message".to_owned(), "@version".to_owned())]),
        BTreeMap::from([("message".to_owned(), "raw_message".to_owned())]),
        BTreeMap::from([("message".to_owned(), "event..detail".to_owned())]),
        BTreeMap::from([("mesage".to_owned(), "event.message".to_owned())]),
    ] {
        assert!(output.set_field_mapping(changes).is_err());
        assert_eq!(output.field_mapping().unwrap()["message"], "message");
    }
    output
        .set_field_mapping(BTreeMap::from([
            ("message".to_owned(), "event.message".to_owned()),
            ("level".to_owned(), "event.level".to_owned()),
        ]))
        .unwrap();
    output
        .write(
            "redis",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            "record",
        )
        .unwrap();
    output.close().unwrap();
    let record: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(record["event"]["message"], "record");
    assert!(record["event"]["level"].is_string());
}

#[test]
fn network_info_reports_protocol_and_target_without_claiming_delivery() {
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = udp.local_addr().unwrap();
    let output = OutputHandler::network(
        "127.0.0.1",
        address.port(),
        "udp",
        Duration::from_secs(1),
        0,
        Duration::ZERO,
    )
    .unwrap()
    .json(BTreeMap::new(), false, false);
    let info = output.network_info().unwrap().unwrap();
    assert_eq!(info.protocol, "udp");
    assert_eq!(info.target, address.to_string());
    let http = OutputHandler::http("127.0.0.1", 1234, "/ingest", Duration::from_secs(1)).unwrap();
    let info = http.network_info().unwrap().unwrap();
    assert_eq!(info.protocol, "http");
    assert_eq!(info.target, "127.0.0.1:1234/ingest");
    assert!(OutputHandler::console().network_info().unwrap().is_none());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let mut tcp = OutputHandler::network(
        "127.0.0.1",
        address.port(),
        "tcp",
        Duration::from_secs(1),
        0,
        Duration::ZERO,
    )
    .unwrap();
    let (mut receiver, _) = listener.accept().unwrap();
    let info = tcp.network_info().unwrap().unwrap();
    assert_eq!(info.protocol, "tcp");
    assert_eq!(info.target, address.to_string());
    tcp.write(
        "redis",
        Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        "record",
    )
    .unwrap();
    let mut received = [0; 7];
    receiver.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"record\n");
    assert_eq!(
        tcp.network_info().unwrap().unwrap().target,
        address.to_string()
    );
}

#[test]
fn writes_files_multi_file_and_json_fanout() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first.log");
    let second = temp.path().join("second.log");
    let file = OutputHandler::file(&first, RotationOptions::default()).unwrap();
    let json = OutputHandler::file(&second, RotationOptions::default())
        .unwrap()
        .json(BTreeMap::new(), true, false);
    let mut fanout = OutputHandler::fanout(vec![file, json]).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
    fanout.write("redis", time, "message").unwrap();
    fanout.close().unwrap();
    assert_eq!(fs::read_to_string(first).unwrap(), "message\n");
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(second).unwrap()).unwrap();
    assert_eq!(value["message"], "message");
    assert_eq!(value["log_type"], "redis");
    let mut multi = OutputHandler::multi_file(
        temp.path(),
        "{log_type}_{date}.log",
        "%Y%m%d",
        RotationOptions::default(),
    )
    .unwrap();
    multi.write("redis", time, "event").unwrap();
    multi.close().unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("redis_20240102.log")).unwrap(),
        "event\n"
    );
    assert!(OutputHandler::multi_file(temp.path(), "x", "%Q", RotationOptions::default()).is_err());
    assert!(OutputHandler::fanout(vec![]).is_err());
}

#[test]
fn fanout_reports_partial_delivery_and_flushes_earlier_outputs() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first.log");
    let last = root.path().join("last.log");
    let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
    let udp = OutputHandler::network(
        "127.0.0.1",
        receiver.local_addr().unwrap().port(),
        "udp",
        Duration::from_secs(1),
        0,
        Duration::ZERO,
    )
    .unwrap();
    let mut output = OutputHandler::fanout(vec![
        OutputHandler::file(&first, RotationOptions::default()).unwrap(),
        udp,
        OutputHandler::file(&last, RotationOptions::default()).unwrap(),
    ])
    .unwrap();
    let record = "x".repeat(70_000);
    assert!(
        output
            .write(
                "redis",
                Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                &record
            )
            .is_err()
    );
    assert_eq!(output.statistics().accepted_records, 0);
    assert_eq!(output.statistics().failed_records, 1);
    output.close().unwrap();
    assert_eq!(fs::read_to_string(first).unwrap(), format!("{record}\n"));
    assert!(fs::read(last).unwrap().is_empty());
}

#[test]
fn writes_udp_and_json_http_records() {
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut output = OutputHandler::network(
        "127.0.0.1",
        udp.local_addr().unwrap().port(),
        "udp",
        Duration::from_secs(2),
        0,
        Duration::ZERO,
    )
    .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
    output.write("redis", time, "event").unwrap();
    assert_eq!(output.statistics().accepted_records, 1);
    assert_eq!(output.statistics().failed_records, 0);
    assert_eq!(output.statistics().reconnect_count, 0);
    let mut buffer = [0; 128];
    let (length, _) = udp.recv_from(&mut buffer).unwrap();
    assert_eq!(&buffer[..length], b"event\n");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 4096];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let length: usize = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        while request.len() - header_end < length {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        request
    });
    let mut http = OutputHandler::http("127.0.0.1", port, "/events", Duration::from_secs(2))
        .unwrap()
        .json(BTreeMap::new(), false, false);
    http.write("redis", time, "event").unwrap();
    let request = server.join().unwrap();
    assert!(String::from_utf8_lossy(&request).contains("Content-Type: application/json"));
}

#[test]
fn output_statistics_count_rejected_records() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("events.json");
    let mapping = BTreeMap::from([("message".to_owned(), "@timestamp".to_owned())]);
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .json(mapping, false, false);
    assert!(output.write("redis", Utc::now(), "event").is_err());
    let stats = output.statistics();
    assert_eq!(stats.accepted_records, 0);
    assert_eq!(stats.failed_records, 1);
    output.close().unwrap();
    assert!(fs::read(&path).unwrap().is_empty());
}

#[test]
fn elk_and_splunk_presets_map_fields_and_units() {
    let root = tempfile::tempdir().unwrap();
    let elk = root.path().join("elk.json");
    let splunk = root.path().join("splunk.json");
    let mut elk_output = OutputHandler::file(&elk, RotationOptions::default())
        .unwrap()
        .elk_json(BTreeMap::new(), true, false);
    let mut splunk_output = OutputHandler::file(&splunk, RotationOptions::default())
        .unwrap()
        .splunk_json(BTreeMap::new(), false, false);
    let timestamp = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
    let record = "10.0.0.1 - - [x] \"GET / HTTP/1.1\" 200 125 \"-\" \"Mozilla/5.0\" elapsed=125 ms";
    elk_output.write("nginx_access", timestamp, record).unwrap();
    splunk_output
        .write("nginx_access", timestamp, record)
        .unwrap();
    elk_output.close().unwrap();
    splunk_output.close().unwrap();
    let elk: serde_json::Value = serde_json::from_slice(&fs::read(elk).unwrap()).unwrap();
    let splunk: serde_json::Value = serde_json::from_slice(&fs::read(splunk).unwrap()).unwrap();
    assert_eq!(elk["source"]["ip"], "10.0.0.1");
    assert_eq!(elk["http"]["response"]["status_code"], 200);
    assert_eq!(elk["http"]["response"]["time_ms"], 125.0);
    assert_eq!(elk["raw_message"], record);
    assert_eq!(splunk["sourcetype"], "nginx_access");
    assert_eq!(splunk["src_ip"], "10.0.0.1");
    assert_eq!(splunk["status"], 200);
    assert_eq!(splunk["response_time"], 0.125);
    assert_eq!(splunk["_raw"], record);
    assert!(splunk.get("raw_message").is_none());
}

#[test]
fn force_rotation_preserves_full_backups_and_reports_file_state() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.log");
    let mut output = OutputHandler::file(
        &path,
        RotationOptions {
            max_files: 1,
            ..RotationOptions::default()
        },
    )
    .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    output.write("redis", time, "first").unwrap();
    let info = output.file_info().unwrap();
    assert_eq!(info.path, path);
    assert_eq!(info.size_bytes, 6);
    output.force_rotation().unwrap();
    assert_eq!(output.file_info().unwrap().size_bytes, 0);
    output.write("redis", time, "second").unwrap();
    assert!(output.force_rotation().is_err());
    output.close().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "second\n");
    assert_eq!(
        fs::read_to_string(root.path().join("events.log.1")).unwrap(),
        "first\n"
    );
    assert!(OutputHandler::console().force_rotation().is_err());
}

#[test]
fn multi_file_type_info_and_rotation_work_after_file_eviction() {
    let root = tempfile::tempdir().unwrap();
    let mut output = OutputHandler::multi_file(
        root.path(),
        "{log_type}_{date}.log",
        "%Y%m%d",
        RotationOptions {
            max_files: 2,
            ..RotationOptions::default()
        },
    )
    .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    assert!(output.file_info_for_type("missing").unwrap().is_none());
    for index in 0..35 {
        output
            .write(&format!("type{index}"), time, &format!("event{index}"))
            .unwrap();
    }
    assert_eq!(output.active_file_types().len(), 35);
    let info = output.file_info_for_type("type0").unwrap().unwrap();
    assert_eq!(info.size_bytes, 7);
    let before = output.file_statistics().unwrap();
    let serialized = serde_json::to_value(&before["type0"]).unwrap();
    assert_eq!(
        serialized["file_path"],
        root.path().join("type0_20240101.log").to_str().unwrap()
    );
    assert_eq!(serialized["file_size"], 7);
    assert_eq!(serialized["recent_logs"], serde_json::json!(["event0"]));
    output.force_rotation_for_type("type0").unwrap();
    let after = output.file_statistics().unwrap();
    let serialized = serde_json::to_value(&after["type0"]).unwrap();
    assert_eq!(serialized["file_size"], 0);
    assert_eq!(serialized["line_count"], 0);
    assert_eq!(
        output
            .file_info_for_type("type0")
            .unwrap()
            .unwrap()
            .size_bytes,
        0
    );
    assert_eq!(
        fs::read_to_string(root.path().join("type0_20240101.log.1")).unwrap(),
        "event0\n"
    );
    assert!(output.force_rotation_for_type("missing").is_err());
    output.force_rotation().unwrap();
    output.close().unwrap();
    assert_eq!(
        fs::read_to_string(root.path().join("type34_20240101.log.1")).unwrap(),
        "event34\n"
    );
}

#[test]
fn multi_file_shared_path_rotates_once_for_all_types() {
    let root = tempfile::tempdir().unwrap();
    let mut output = OutputHandler::multi_file(
        root.path(),
        "shared.log",
        "%Y%m%d",
        RotationOptions::default(),
    )
    .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    output.write("redis", time, "first").unwrap();
    output.write("ssh_auth", time, "second").unwrap();
    output.force_rotation().unwrap();
    output.close().unwrap();
    assert_eq!(
        fs::read_to_string(root.path().join("shared.log.1")).unwrap(),
        "first\nsecond\n"
    );
    assert!(!root.path().join("shared.log.2").exists());
}

#[test]
fn multi_file_statistics_flush_and_report_recent_lines() {
    let root = tempfile::tempdir().unwrap();
    let mut output = OutputHandler::multi_file(
        root.path(),
        "{log_type}.log",
        "%Y%m%d",
        RotationOptions::default(),
    )
    .unwrap();
    assert!(output.file_statistics().unwrap().is_empty());
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    for record in ["first", "second", " ", "fourth"] {
        output.write("redis", time, record).unwrap();
    }
    output.write("ssh_auth", time, "login").unwrap();
    let stats = output.file_statistics().unwrap();
    assert_eq!(stats["redis"].line_count, 4);
    assert_eq!(stats["redis"].recent_logs, ["second", "fourth"]);
    assert_eq!(
        stats["redis"].size_bytes,
        fs::metadata(root.path().join("redis.log")).unwrap().len()
    );
    assert!(stats["redis"].active);
    let json = serde_json::to_value(&stats["redis"]).unwrap();
    assert_eq!(json["file_path"], json["path"]);
    assert_eq!(json["file_size"], json["size_bytes"]);
    assert_eq!(json["line_count"], 4);
    assert_eq!(stats["ssh_auth"].recent_logs, ["login"]);
    output.force_rotation_for_type("redis").unwrap();
    let after = output.file_statistics().unwrap();
    assert_eq!(after["redis"].line_count, 0);
    assert_eq!(after["ssh_auth"].line_count, 1);
    assert!(OutputHandler::console().file_statistics().is_err());
}

#[test]
fn rotates_files_and_writes_tcp_records() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("events.log");
    let mut file = OutputHandler::file(
        &path,
        RotationOptions {
            size_bytes: Some(4),
            max_files: 2,
            ..RotationOptions::default()
        },
    )
    .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
    file.write("redis", time, "one").unwrap();
    file.write("redis", time, "two").unwrap();
    file.close().unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("events.log.1")).unwrap(),
        "one\n"
    );
    assert_eq!(fs::read_to_string(path).unwrap(), "two\n");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let receiver = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut record = [0; 6];
        stream.read_exact(&mut record).unwrap();
        record
    });
    let mut output = OutputHandler::network(
        "127.0.0.1",
        port,
        "tcp",
        Duration::from_secs(2),
        0,
        Duration::ZERO,
    )
    .unwrap();
    output.write("redis", time, "event").unwrap();
    output.close().unwrap();
    assert_eq!(receiver.join().unwrap(), *b"event\n");
}

#[test]
fn structured_entry_preserves_metadata_and_custom_fields_in_json() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("entry.jsonl");
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut entry = log_forge::models::LogEntry::new(time, "nginx", "ERROR", "request failed");
    entry.source_ip = Some("192.0.2.42".into());
    entry.status_code = Some(503);
    entry.response_time = Some(0.125);
    entry
        .custom_fields
        .insert("user_id".into(), serde_json::json!(123));
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .elk_json(BTreeMap::new(), true, false);
    output.write_entry(&entry).unwrap();
    output.close().unwrap();
    let record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(record["source"]["ip"], "192.0.2.42");
    assert_eq!(record["http"]["response"]["status_code"], 503);
    assert_eq!(record["http"]["response"]["time_ms"], 125.0);
    assert_eq!(record["user_id"], 123);
    assert_eq!(record["level"], "ERROR");
    assert!(
        record["raw_message"]
            .as_str()
            .unwrap()
            .contains("request failed")
    );
}

#[test]
fn structured_entry_fanout_preserves_raw_and_splunk_fields() {
    let root = tempfile::tempdir().unwrap();
    let raw = root.path().join("raw.log");
    let structured = root.path().join("splunk.jsonl");
    let raw_output = OutputHandler::file(&raw, RotationOptions::default()).unwrap();
    let json_output = OutputHandler::file(&structured, RotationOptions::default())
        .unwrap()
        .splunk_json(BTreeMap::new(), false, false);
    let mut output = OutputHandler::fanout(vec![raw_output, json_output]).unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut entry = log_forge::models::LogEntry::new(time, "service", "WARN", "test event");
    entry
        .custom_fields
        .insert("event_id".into(), serde_json::json!("abc"));
    output.write_entry(&entry).unwrap();
    assert_eq!(output.statistics().accepted_records, 1);
    output.close().unwrap();
    assert!(
        fs::read_to_string(raw)
            .unwrap()
            .contains("[WARN] test event")
    );
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(structured).unwrap()).unwrap();
    assert_eq!(value["sourcetype"], "service");
    assert_eq!(value["log_level"], "WARN");
    assert_eq!(value["_raw"], "test event");
    assert_eq!(value["event_id"], "abc");
}

#[test]
fn structured_entry_rejects_unrepresentable_elk_duration() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("entry.jsonl");
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut entry = log_forge::models::LogEntry::new(time, "service", "INFO", "event");
    entry.response_time = Some(f64::MAX);
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .elk_json(BTreeMap::new(), false, false);
    assert!(output.write_entry(&entry).is_err());
    assert_eq!(output.statistics().accepted_records, 0);
    assert_eq!(output.statistics().failed_records, 1);
    output.close().unwrap();
    assert!(fs::read_to_string(path).unwrap().is_empty());
}

#[test]
fn structured_entry_uses_custom_field_mappings_and_omits_absent_values() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mapped.jsonl");
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut entry = log_forge::models::LogEntry::new(time, "service", "INFO", "event");
    entry.source_ip = Some("192.0.2.42".into());
    entry
        .custom_fields
        .insert("event_id".into(), serde_json::json!("id-1"));
    entry
        .custom_fields
        .insert("details".into(), serde_json::json!({"count": 3}));
    let mapping = BTreeMap::from([
        ("source_ip".into(), "client.ip".into()),
        ("event_id".into(), "event.id".into()),
    ]);
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .json(mapping, false, false);
    output.write_entry(&entry).unwrap();
    output.close().unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(value["client"]["ip"], "192.0.2.42");
    assert_eq!(value["event"]["id"], "id-1");
    assert_eq!(value["details"]["count"], 3);
    assert!(value.get("raw_message").is_none());
    assert!(value.get("user_agent").is_none());
    assert!(value.get("http").is_none());
}

#[test]
fn structured_entry_rejects_conflicting_mapped_paths_before_write() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mapped.jsonl");
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut entry = log_forge::models::LogEntry::new(time, "service", "INFO", "event");
    entry.source_ip = Some("192.0.2.42".into());
    entry
        .custom_fields
        .insert("event_id".into(), serde_json::json!("id-1"));
    let mapping = BTreeMap::from([
        ("source_ip".into(), "event".into()),
        ("event_id".into(), "event.id".into()),
    ]);
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .json(mapping, false, false);
    assert!(output.write_entry(&entry).is_err());
    assert_eq!(output.statistics().failed_records, 1);
    output.close().unwrap();
    assert!(fs::read_to_string(path).unwrap().is_empty());
}

#[test]
fn structured_entry_fanout_reports_failure_and_skips_later_outputs() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first.log");
    let failing = root.path().join("failed.jsonl");
    let last = root.path().join("last.log");
    let mapping = BTreeMap::from([
        ("source_ip".into(), "event".into()),
        ("event_id".into(), "event.id".into()),
    ]);
    let mut output = OutputHandler::fanout(vec![
        OutputHandler::file(&first, RotationOptions::default()).unwrap(),
        OutputHandler::file(&failing, RotationOptions::default())
            .unwrap()
            .json(mapping, false, false),
        OutputHandler::file(&last, RotationOptions::default()).unwrap(),
    ])
    .unwrap();
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut entry = log_forge::models::LogEntry::new(time, "service", "INFO", "event");
    entry.source_ip = Some("192.0.2.42".into());
    entry
        .custom_fields
        .insert("event_id".into(), serde_json::json!("id-1"));
    let error = output.write_entry(&entry).unwrap_err();
    assert!(error.to_string().contains("output handler 1 failed"));
    assert_eq!(output.statistics().accepted_records, 0);
    assert_eq!(output.statistics().failed_records, 1);
    output.close().unwrap();
    assert!(fs::read_to_string(first).unwrap().contains("event"));
    assert!(fs::read_to_string(failing).unwrap().is_empty());
    assert!(fs::read_to_string(last).unwrap().is_empty());
}

#[test]
fn structured_entry_http_output_preserves_json_fields() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 4096];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]);
        assert!(headers.contains("Content-Type: application/json"));
        let length: usize = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        while request.len() - header_end < length {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        request[header_end..header_end + length].to_vec()
    });
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut entry = log_forge::models::LogEntry::new(time, "service", "WARN", "event");
    entry.status_code = Some(429);
    entry
        .custom_fields
        .insert("request_id".into(), serde_json::json!("req-42"));
    let mut output = OutputHandler::http("127.0.0.1", port, "/events", Duration::from_secs(2))
        .unwrap()
        .json(BTreeMap::new(), false, false);
    output.write_entry(&entry).unwrap();
    output.close().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&server.join().unwrap()).unwrap();
    assert_eq!(value["level"], "WARN");
    assert_eq!(value["request_id"], "req-42");
    assert_eq!(value["http"]["response"]["status_code"], 429);
}

#[test]
fn structured_entries_route_by_type_and_survive_rotation() {
    let root = tempfile::tempdir().unwrap();
    let mut output = OutputHandler::multi_file(
        root.path(),
        "{log_type}_{date}.jsonl",
        "%Y%m%d",
        RotationOptions::default(),
    )
    .unwrap()
    .json(BTreeMap::new(), false, false);
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut first = log_forge::models::LogEntry::new(time, "service", "INFO", "first");
    first
        .custom_fields
        .insert("event_id".into(), serde_json::json!("one"));
    let mut other = log_forge::models::LogEntry::new(time, "worker", "WARN", "other");
    other
        .custom_fields
        .insert("event_id".into(), serde_json::json!("two"));
    output.write_entry(&first).unwrap();
    output.write_entry(&other).unwrap();
    output.force_rotation_for_type("service").unwrap();
    first.message = "next".into();
    first
        .custom_fields
        .insert("event_id".into(), serde_json::json!("three"));
    output.write_entry(&first).unwrap();
    output.close().unwrap();
    for (name, expected) in [
        ("service_20240101.jsonl.1", "one"),
        ("worker_20240101.jsonl", "two"),
        ("service_20240101.jsonl", "three"),
    ] {
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.path().join(name)).unwrap()).unwrap();
        assert_eq!(value["event_id"], expected);
    }
}

#[test]
fn structured_entry_custom_type_override_keeps_json_aliases_consistent() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("entry.jsonl");
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut entry = log_forge::models::LogEntry::new(time, "original", "INFO", "event");
    entry
        .custom_fields
        .insert("log_type".into(), serde_json::json!("overridden"));
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .json(BTreeMap::new(), false, false);
    output.write_entry(&entry).unwrap();
    output.close().unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(value["type"], "overridden");
    assert_eq!(value["log_type"], "overridden");
}

#[test]
fn structured_entry_null_override_omits_log_type() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("entry.jsonl");
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut entry = log_forge::models::LogEntry::new(time, "original", "INFO", "event");
    entry
        .custom_fields
        .insert("log_type".into(), serde_json::Value::Null);
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .json(BTreeMap::new(), false, false);
    output.write_entry(&entry).unwrap();
    output.close().unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(value.get("type").is_none());
    assert!(value.get("log_type").is_none());
}

#[test]
fn structured_entry_null_overrides_omit_other_fields() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("entry.jsonl");
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut entry = log_forge::models::LogEntry::new(time, "service", "INFO", "event");
    entry.source_ip = Some("192.0.2.42".into());
    entry.status_code = Some(200);
    for field in ["timestamp", "level", "message", "source_ip", "status_code"] {
        entry
            .custom_fields
            .insert(field.into(), serde_json::Value::Null);
    }
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .json(BTreeMap::new(), true, false);
    output.write_entry(&entry).unwrap();
    output.close().unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    for field in [
        "@timestamp",
        "timestamp",
        "level",
        "message",
        "source",
        "http",
    ] {
        assert!(value.get(field).is_none(), "{field}: {value}");
    }
    assert!(value["raw_message"].as_str().unwrap().contains("event"));
}

#[test]
fn structured_entry_valid_override_replaces_invalid_base_duration() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("entry.jsonl");
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut entry = log_forge::models::LogEntry::new(time, "service", "INFO", "event");
    entry.response_time = Some(f64::NAN);
    entry
        .custom_fields
        .insert("response_time".into(), serde_json::json!(0.125));
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .json(BTreeMap::new(), false, false);
    output.write_entry(&entry).unwrap();
    output.close().unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(value["http"]["response"]["time"], 0.125);
}

#[test]
fn structured_entry_overrides_timestamp_level_and_message_but_keeps_raw_entry() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("entry.jsonl");
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut entry = log_forge::models::LogEntry::new(time, "service", "INFO", "original");
    entry.custom_fields.insert(
        "timestamp".into(),
        serde_json::json!("2030-02-03T00:00:00Z"),
    );
    entry
        .custom_fields
        .insert("level".into(), serde_json::json!("ERROR"));
    entry
        .custom_fields
        .insert("message".into(), serde_json::json!("replacement"));
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .json(BTreeMap::new(), true, false);
    output.write_entry(&entry).unwrap();
    output.close().unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(value["@timestamp"], "2030-02-03T00:00:00Z");
    assert_eq!(value["timestamp"], "2030-02-03T00:00:00Z");
    assert_eq!(value["level"], "ERROR");
    assert_eq!(value["message"], "replacement");
    assert!(
        value["raw_message"]
            .as_str()
            .unwrap()
            .contains("[INFO] original")
    );
}

#[test]
fn structured_entry_survives_output_sink_trait_dispatch() {
    use log_forge::output::OutputSink;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("entry.jsonl");
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let mut entry = log_forge::models::LogEntry::new(time, "service", "INFO", "event");
    entry
        .custom_fields
        .insert("request_id".into(), serde_json::json!("req-42"));
    let mut output = OutputHandler::file(&path, RotationOptions::default())
        .unwrap()
        .json(BTreeMap::new(), false, false);
    let sink: &mut dyn OutputSink = &mut output;
    sink.write_entry(&entry).unwrap();
    sink.flush().unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(value["request_id"], "req-42");
}

#[test]
fn text_only_output_sink_rejects_structured_entries() {
    use log_forge::output::OutputSink;
    struct TextOnly;
    impl OutputSink for TextOnly {
        fn write(
            &mut self,
            _: &str,
            _: chrono::DateTime<Utc>,
            _: &str,
        ) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
        fn close(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let entry = log_forge::models::LogEntry::new(time, "service", "INFO", "event");
    let error = TextOnly.write_entry(&entry).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::Unsupported
    );
}
