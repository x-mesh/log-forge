# Reference

This page describes CLI options, outputs, generators, and the Rust API.

Run the commands from the project directory.

## Configuration commands

Use `config create --file config.yaml` to create a configuration. Use `config validate --file config.yaml` to check it. Use `config show --file config.yaml` to print it. Add `--format json` to create or display JSON. The create command does not replace an existing file.

For `config create`, the file extension selects YAML or JSON. If you also set `--format`, it must match the extension.

Without a configuration file, `start`, `config show`, and `config create` use the original Python defaults. With a configuration file, `log-generator` merges these defaults. `log-forge` merges them only with `--original-defaults`. Use `--standalone-config` to use only the file. A merge retains default log types that the file does not mention.

## Output destinations

The `start` command supports `file`, `multi_file`, `console`, `json`, and `network` output. File output appends to existing files. Network output supports TCP, UDP, and HTTP. For `log-generator`, use `--real-time` to override virtual time, even if the file sets `virtual_time` to false. For `log-forge`, the file sets the time mode unless the command specifies it.

Set `global.multi_file.separate_by_type` to `false` to combine log types. The `{log_type}` placeholder then uses `all`. Date placeholders still create separate files.

Set `global.network` with `host`, `port`, and `protocol` for network output. This section takes precedence over flat network fields. CLI options take precedence over the section. The section defaults to UDP. Network output uses a five-second timeout by default. Use `--network-timeout` or set `network_timeout` under `global` to change it. A timeout ends the run with an error. TCP retries a detected connection reset before any bytes of the current record are written. Set `max_retries` and `retry_delay` in `global.network` or a TCP handler. The defaults are three retries and one second. Partial writes fail without a retry. TCP cannot prove that the receiver processed a prior record. HTTP output sends one POST per record and requires a 2xx response. It supports plain HTTP without TLS.

Use `output.handlers` to send each record to multiple destinations. Each handler accepts `type: file`, `console`, or `network`, with `format: raw` or `json`. HTTP network handlers also accept `format: prometheus`. Console handlers accept `format: human_readable` and `colored`. Color requires a terminal. `NO_COLOR` disables color. TCP and UDP handlers accept `format: syslog`. Network handlers accept TCP, UDP, or HTTP. HTTP handlers require an `endpoint` path. The default error behavior depends on the command name. A handler can receive a record before a later handler fails. `--output` overrides the configured handlers.

Prometheus output sends the cumulative `log_forge_records_total` counter for each `log_type`. It sends samples on each output flush. Sample timestamps use the system clock, not virtual log time. It retries HTTP 5xx responses three times. On normal shutdown, it sends a stale marker for each series. A crash can prevent the marker. Use a separate destination or external labels for each concurrent process to avoid duplicate series.

JSON output includes the timestamp, type, level, message, and fields that the parser finds in the record. Set `field_mapping` under `global` to rename fields. Set `include_raw_message` to `false` to omit the raw message. Set `pretty_print` to `true` for indented JSON.

Mapping keys are `timestamp`, `log_type`, `level`, `message`, `source_ip`, `user_agent`, `status_code`, and `response_time`. They also include `trace_id`, `span_id`, and `parent_span_id`. Unknown keys are errors.

Docker records from the built-in generator carry level metadata into structured output. Their raw JSON keeps the Docker fields. For external Docker JSON records without a level, structured output uses `UNKNOWN`.

Syslog and Redis records retain their generated severity in structured output. An external Syslog record without a priority uses `UNKNOWN`.

SSH authentication records have no explicit severity. Structured output uses `UNKNOWN` for those records.

For SSH authentication logs, `analyze` counts accepted and failed authentication, invalid users, and closed connections. The count covers recognized valid records.

Use handler format `elk_json` or `splunk_json` for their field mappings. The ELK `http.response.time_ms` value uses milliseconds. Splunk omits `raw_message` by default. Set `include_raw_message` on a JSON handler to change that choice.

## Monitor and control

Send SIGINT or SIGTERM to stop `start`. The command flushes buffered output before it exits.

The `start` command reports counts and rate on stderr. Use `--no-monitor` to disable this report. Use `--no-progress` to omit the target count.

Set a metrics or health port to `0` to use an available local port. The ready message reports the assigned port.

Set `--control-socket` to control a run from another process. On Unix, use a unique socket path for each run. On Windows, use a simple pipe name. The Windows control path needs runtime verification.

```sh
log-forge start --control-socket /tmp/log-forge.sock
log-forge status --control-socket /tmp/log-forge.sock
log-forge monitor --control-socket /tmp/log-forge.sock --refresh 2 --show-logs
log-forge pause --control-socket /tmp/log-forge.sock
log-forge resume --control-socket /tmp/log-forge.sock
log-forge stop --control-socket /tmp/log-forge.sock
```

The control commands print JSON. Pause stops generation after the current record. Queued output can continue after a pause. A normal exit removes the socket. If the process stops without cleanup, use a new socket path.

The monitor prints one JSON object per refresh. `--show-logs` includes up to 20 recent records. Records over 4096 bytes have `truncated: true`.

## Performance and error handling

Use `--fixed-interval` to disable a configured random interval. Set `buffer_size` under `global` to control the output buffer in bytes. Set `batch_size` to flush after that number of records.

The parser also accepts these values under a legacy `performance` section. Values under `global` take precedence. Set `threads` under `global` or `performance`, or use `--threads`. Virtual-time runs with built-in generators can use multiple threads. The output order stays reproducible for the same seed and start time. Real-time and stateful generator runs use one thread and report this choice on stderr.

Set `performance.memory_limit` to stop a run when the process exceeds its resident memory limit. The check runs at startup, at most every 100 ms during a run, and at exit. It does not impose a hard allocation cap.

Set `monitoring.enable_statistics` and `statistics_interval` to control console statistics. Enable `monitoring.metrics` to expose `log_forge_records_total` on a local Prometheus endpoint. Enable `monitoring.health_check` to expose a local health endpoint. Both endpoints bind to `127.0.0.1`. The metrics endpoint accepts static labels. `--no-monitor` disables console statistics but leaves configured HTTP endpoints active.

For gateway services, `monitoring.service_metrics` accepts `request_count`, `response_time`, and `error_rate`. These metrics use generated gateway requests. Response time is the mean in milliseconds. Error rate is the fraction of responses with status 500 or above.

User and order metrics match the event text in the microservices example. Active users counts distinct numeric IDs seen in the last five minutes of wall time. Registration rate is the total registrations divided by elapsed wall time. Order value sums amounts from payment processed events. Cancellation rate divides cancellation events by creation events. Other event text does not contribute.

`thread_pool_size` limits the generator thread count. If `threads` is absent, it sets that count. `io_buffer_size` takes precedence over `performance.buffer_size`. A `global.buffer_size` value takes precedence over both. Set `lazy_loading` to false to load built-in patterns before generation. The command reports `gc_threshold` as unsupported because Rust has no cyclic garbage collector.

The `logging` section controls lifecycle messages. It supports `level`, `format`, `file`, and per-logger levels. It records start and successful completion. Other diagnostics still go to standard error.

Set `async_io` to `true` or use `--async-io` to write records on a separate thread. Use `--sync-io` to override the setting. The output queue holds up to 64 records. The command drains it before exit. A blocked destination can delay shutdown.

`log-generator` reports record errors and continues by default. Use `--fail-fast` to stop on an error. `log-forge` stops by default. Use `--continue-on-error` to continue. In this mode, the command writes errors to stderr. It tries each output and counts each generated record even if an output fails. It also continues after batch flush errors. It reports close errors and still exits successfully. With asynchronous output, it waits for each output result. Real-time, stateful, and custom pattern runs use one generator thread. Persistent generation errors continue until you stop the command.

The control status reports these errors in `error_count`.

## Configure log types

Nginx and Apache error logs accept `error_levels` and `error_patterns`. Nginx and Apache access logs accept `ip_ranges`, `status_codes`, `user_agents`, and `url_patterns` under `custom_fields`. Apache access logs also accept `log_format`. Set `virtual_hosts` to prepend a selected host to each Apache access record. Set `message_patterns` under `syslog` to provide templates for specific facilities. Kubernetes accepts `pod_patterns` and `event_patterns` for generated events. Docker accepts `log_drivers` and `container_patterns`. It emits JSON file, syslog, or journal JSON records. It does not write to Docker or the system journal.

Use `rotation_size`, `rotation_time`, and `max_files` under `global` to rotate file or JSON output. The command preserves rotated files. If rotation reaches the backup limit, it returns an error. Error handling follows the command options. Move existing rotated files before you resume.

Nginx `crit` and `alert` levels use built-in critical messages. Apache also uses them for `emerg`. These levels ignore custom error patterns.

For web error logs, JSON output and analysis map `crit` to `CRITICAL` and `emerg` to `EMERGENCY`.

You can also set `log_rotation` under `global`. Set `enabled` to `true` and set `max_size` in bytes or units such as `100MB`. Set `rotation_time` to `hourly`, `daily`, `weekly`, or a number of seconds. Set `compress` to `true` to store rotated records as gzip files. Set `backup_count` to limit time-based rotation. Set `max_files` to limit size-based rotation. If both triggers fire, the lower limit applies. `--max-files` overrides both limits. Rotation requires a regular file without a symbolic link.

The `nginx_error` type accepts `error_levels` and `error_patterns`. The `syslog` type accepts `facilities`, `severities`, `hostname`, `rfc_format`, and `priority_style`. Set `rfc_format` to `3164` or `5424`. Set `priority_style` to `pri`, `bracket`, `plain`, or `none`.

Syslog messages use the original facility-specific patterns. A facility without a pattern uses a message that names that facility.

Nginx, Apache, and MySQL error logs use message patterns from the original generator. MySQL selects note, warning, and error levels by weight.

For Nginx and Apache error logs, `error_levels` accepts a list or a weighted map. Set it at the type level or under `custom_fields`.

The `django` type accepts request, database, and security components. Set `include_sql_queries` and `sql_query_patterns` to control SQL records.

The `fastapi` type accepts weighted `log_levels`, `endpoints`, and weighted `status_codes`. Put `status_codes` under `custom_fields`. Path parameters such as `{user_id}` get generated values. Set `response_time_distribution` to select weighted ranges in milliseconds. Use `include_request_id`, `include_user_id`, and `include_correlation_id` to control identifiers. The `custom` type accepts a `template` and named `fields`. Field values support string choices and built-in generators.

The `docker` type uses Docker JSON format by default. Set `format_type` to `text` for text output. Set `log_levels` and `container_names` to control the generated records.

The Docker JSON `log` field ends with a newline character. The Python generator uses a literal backslash and `n`.

Docker analysis uses an explicit level or journal priority. If neither exists, level statistics omit that record.

Set `container_name` or `container_id` to keep a fixed identity. These settings also work under `custom_fields`.

Set `image_name` or `images` to control the Docker generator metadata. The Rust `LogGenerator::metadata` method returns the selected image, container name, and ID. These fields do not change raw Docker JSON.

The `kubernetes_pod` type accepts `log_levels`, `namespaces`, and `pod_name`. The `kubernetes_event` type accepts `event_types`. These fields also work under `custom_fields`.

For Kubernetes events, JSON output and analysis map `Normal` to `INFO` and `Warning` to `WARNING`.

Set `namespace`, `pod_name`, `container_name`, and `labels` for a pod identity. The Rust `LogGenerator::structured_metadata` method returns that identity and its labels. A generator keeps one namespace across its records.

Docker and Kubernetes pod logs use the original application and error message patterns. Kubernetes events use normal and warning reasons from the original generator.

MySQL query, PostgreSQL, and slow query logs accept `query_types` and `table_names`. PostgreSQL and MySQL error logs accept weighted `log_levels`. These fields also work under `custom_fields`.

MySQL error analysis reports the `Note` level as `NOTE`. Sampling treats it as an informational level.

PostgreSQL analysis reads the level from the record header. Sampling treats `LOG` as an informational level.

Django request logs accept weighted `log_levels` and `status_codes`, plus `url_patterns`. Django SQL logs accept `models`. Put these fields under `custom_fields`.

Use `--types haproxy` with `start` to generate HAProxy HTTP records with IPv4 clients. The format follows the [HAProxy HTTP log format](https://docs.haproxy.org/2.1/configuration.html#8.2.3). Records include client addresses, timers, status codes, and termination states. The validator accepts optional syslog prefixes and captured headers. The analyzer counts HTTP status codes.

Kafka broker records use the [Apache Kafka Log4j layout](https://github.com/apache/kafka/blob/trunk/config/log4j2.yaml). They include synthetic broker, partition, rebalance, and error events.

Set `broker_ids`, `topics`, `partition_count`, and weighted `log_levels` under `kafka_broker` to control those records. Levels accept `INFO`, `WARN`, and `ERROR`.

Set weighted `log_levels` under `redis` to select `DEBUG`, `VERBOSE`, `NOTICE`, or `WARNING` records. You can also set the levels under `custom_fields`.

## Custom templates

Templates support `timestamp`, `iso_timestamp`, `epoch_timestamp`, `counter`, `random_int`, `random_float`, `uuid`, `ipv4`, `ip_address`, `port`, and `sentence`. Other fields cover network addresses, HTTP requests, identity, location, files, resource use, security tokens, and databases. Names and domains use a small synthetic vocabulary. Token fields contain synthetic values that cannot authenticate. Use `{response_time:min:1,max:100}` to set a range in milliseconds. A choice uses syntax such as `{status:200,404}`. A counter uses syntax such as `{counter:start:100,step:5}`. A numeric range uses syntax such as `{random_int:min_val:1,max_val:100}`.

Use `{{` and `}}` to write literal braces in a custom template. Unmatched braces are errors.

Set `pattern_file` under `custom` to load a JSON or YAML pattern file. Set `pattern_name` if the file contains multiple patterns. A relative pattern path starts at the configuration file directory. Pattern fields support `faker`, `choice`, `random_int`, `random_float`, `counter`, and `uuid`.

Set `frequency_maps` under `custom` to assign weights to field values. Set `correlation_rules` to select a target distribution when an earlier field matches a value. The condition field must appear before the target field in the template.

Use `pattern_validation` to check templates, suggest improvements, generate samples, test regexes, compare fields, and benchmark generation. The regex check uses Rust regex syntax.

Use `pattern_file::PatternManager` to load, list, add, update, remove, import, and export patterns from Rust. Its `generate` method accepts a timestamp, seed, and record index. Use `generate_with_values` to supply field values.

Add and update refresh the sample output and update time. An update retains the creation time unless the caller supplies another value.

Use `create_sample_patterns` to add the three original example patterns. The report lists created and existing patterns. Existing patterns stay unchanged.

Add and export do not replace existing files. Update and remove require a dedicated JSON file for that pattern. An import error can leave earlier imported patterns in place.

Use `{agent:file:agents.txt}` to select a nonempty line from a file. Relative paths start from the current directory. The command reads the file before generation starts.

## Goloop

The `goloop` type accepts `format_type`, `height_range`, `log_levels`, and `module_weights`. Set `format_type` to `text` or `json`. The level and module settings accept weighted maps. The generator selects from the original 17 module pattern groups. JSON and text records include a submodule and sequence value.

The Goloop `network` settings control the local IP address, peer port, and peer address source. If `use_real_ips` is true, records use the static peer address list from the original generator. The `blockchain` settings control the network ID, block height range, and revision range.

## Error scenarios

The `scenario` type reads a JSON scenario and its JSON pattern files. Set `scenario_file` to the scenario path. Set `patterns_dir` to a directory of custom patterns. If a custom pattern is absent, the generator uses a matching built-in pattern and reports its source on stderr. The generator uses virtual time to select the current escalation level. It stops with an error after the scenario duration.

Use `ScenarioGenerator::from_definition` to create a scenario from an in-memory JSON value. It applies the same checks as file loading.

Use `save_definition` to save the scenario to a new JSON file. It does not save referenced pattern files or replace an existing file.

Use fractional minutes in `duration_minutes` and `escalation_delay_minutes` for sub-minute scenarios.

Use `--type scenario` to validate or analyze scenario records.

The `error_scenarios` directory includes the four default scenarios from the original project. Their escalation levels follow the configured delays. The generator does not apply `escalation_probability`.

Use `ScenarioGenerator::pattern_usage` to inspect pattern counts and last-use times. Scenario timestamps must not move backward.

Use `scenario::ScenarioManager` to register and control multiple scenarios. Start each scenario with a seed. Stop returns the generator with its usage statistics.

The active scenario list follows start order. A restarted scenario moves to the end.

Use `start_at` to set the scenario start time. If the duration expires, the next generation call deactivates the scenario and returns an error. The `start` method uses the first generation time as the start time.

Call `pattern_analytics` to inspect total usage across scenario restarts. Recent usage counts entries in the retained event history.

Call `to_dict` on the result for the original pattern and usage field structure. The average response time is zero because the manager does not measure it. Call `pattern_analytics_with_library` to include the current weight and its change history from an `ErrorLibrary`.

The library analysis returns an error if a recorded pattern has a different definition. A weight change does not cause a conflict.

The `escalation_count` includes every scenario event for the pattern. It also includes events before an escalation.

Call `definition` on a scenario generator to inspect its source fields. The generator preserves additional fields but does not apply them to generation. Use `ScenarioManager::export_configuration` to save definitions and usage counts to a new JSON file. The export includes frequency settings if the manager has them.

Call `set_frequency` to configure error frequency. Then call `generate_if_due` to apply that frequency. A call to `generate` does not apply frequency rules.

Use `generate_first_active_if_due` to select the first active scenario by start order. It removes expired scenarios. If none remain, it returns no record.

Use `generate_realistic_if_due` with an `ErrorLibrary` to select a regular error when no scenario remains. The result identifies the selected path. Active scenario patterns must match the library. Both paths update the manager and library usage statistics.

The manager rejects a library pattern that differs from its recorded definition. Weight changes remain valid. Create a new manager after other pattern changes.

The manager rejects conflicting definitions that share a pattern ID. Identical definitions can share an ID.

Use `ScenarioManager::health_at` to summarize generated scenario errors in the prior hour. The result includes suggestions based on the synthetic events. The manager retains the latest 1,000 events. This summary describes synthetic logs, not the host system.

## Microservice logs

Add a named service under `log_types` with `service_name`, `operations`, and `custom_fields.events` to generate service logs. Event templates accept fields such as `{user_id}` and `{order_id}`. For an explicit operation, use an event with `operation` and `message` keys. Unmapped events use `operation=event`. The bundled microservices example has mappings for its event templates. You can also add templates under `db_operations`, `external_calls`, and `gateway_calls` in `custom_fields`. Service entries can define `databases` and `containers` for `{db_name}` and `{container_name}`. Database types do not change the event text. Define a top-level `error_patterns` library and enable the `error_pattern` log type to generate its entries. The generator uses each entry frequency as a selection weight. A service with `upstream_services` can generate gateway records. Set `endpoints`, `status_codes`, and weighted `response_times` for that service. Use `--type microservice` to validate these records.

Set `distributed_tracing.enabled` and define `service_dependencies` to generate related service spans. Related records share a trace ID. Each child record includes its parent span ID. Dependencies can select a service even if its frequency is zero. A record limit can stop a trace before all spans are emitted. Cycles and unknown services are rejected.

## Error pattern library

Enable `error_library` to generate weighted built-in error patterns. Set `patterns_dir` to load JSON patterns. Set `error_category` or `error_severity` to filter them. Custom patterns with the same ID replace built-in patterns.

Use `ErrorLibrary` to inspect patterns, categories, and severities. Update a weight in memory with `update_weight`. Use `export` to save the selected patterns to a new JSON file. Export does not replace a file.

Use `update_weight_with_reason` to retain a change record in memory. Call `weight_changes` to inspect those records. Use `export_analytics` to save patterns, usage counts, and change records to a new JSON file. The pattern export does not include change records.

Use `select_pattern` for a weighted choice with category or severity filters. Use `generate_scenario` to create related messages from one category.

Use `generate_scenario_with_context` to supply external values for those messages. This method does not update pattern usage counts.

Use `generate_filtered` to apply filters per record and update the selected pattern's usage statistics.

Use `generate_filtered_with_context` to supply external values for the selected pattern.

Use `adjusted_weights` to inspect weights that reflect recent use and peak hours. Use `generate_adjusted` to select a record with those weights. If every adjusted weight is zero, generation returns an error.

Use `generate_adjusted_with_context` to supply external values with adjusted weights.

Use `add_pattern` to add or replace an in-memory pattern. Use `generate_message` to select a pattern and override its context fields.

Use `{{` and `}}` in an error message template for literal braces.

If a context object has no range or default, the value is `unknown`.

Null context values render as `None`. Nested arrays and objects use a Python-style representation. Object keys use sorted order.

Use `add_pattern_with_external_context` for fields supplied only at generation time. Pass those fields to `generate_message`. Automatic generation returns an error if it selects a pattern with missing fields. Use `from_sources_with_external_context` to load those patterns from files. The default file loader requires all fields in the pattern.

Call `required_context_fields` to list the fields that require external values.

If a pattern replaces an existing ID, its usage and weight change history reset.

Use `statistics` for pattern counts and weights. Use `pattern_usage` to inspect generated record counts and last-use times. Direct calls to `generate_message` do not update usage.

## Field frequency

Use `frequency::FieldFrequencyController` to apply field distributions, correlations, UTC time rules, and timed bursts from Rust. A burst begins on a controlled value request. The time rule can use its active multiplier. Set `field_frequency` under `custom` to use these rules in the CLI. Set `field_inactive_value` to replace a field outside its time rule. The default replacement is `-`. This mode uses one generation thread.

Use `FieldFrequencyConfig::web()`, `FieldFrequencyConfig::security()`, or `FieldFrequencyConfig::database()` for preset field distributions and time rules. Pass the configuration to `FieldFrequencyController::new` with a seed. A correlation rule can set `condition_operator` to `greater_than` for an integer comparison. If the field is not an integer, generation returns an error.

Call `statistics` on the controller to inspect rule counts and configured fields.

```yaml
log_generator:
  log_types:
    custom:
      template: "{timestamp} status={status}"
      field_inactive_value: "-"
      field_frequency:
        distributions:
          status: {"200": 1}
        time_patterns:
          status: {business_hours_only: true}
        burst_patterns:
          status:
            burst_probability: 0.1
            burst_duration_seconds: 60
            burst_values: ["500"]
```

Set `error_frequency` under `scenario` to control error attempts. The base probability per attempt is `base_frequency / 60`. Peak hours and bursts multiply that probability. The probability cannot exceed one. Each attempt advances time by the configured generation interval.

## Extend from Rust

Run the examples from the project directory.

```sh
cargo run --example custom_generator
cargo run --example patterns
cargo run --example analyze
cargo run --example output_handlers
cargo run --example error_patterns
```

Implement the `LogGenerator` trait and register a factory with `GeneratorRegistry`. Call `write_records` to generate records at specified timestamps. Call `run_with_registry` from a Rust executable to use registered generators with the CLI engine. Set the generator options under `plugin_config` in the configuration file. The standard executable can run a process generator at runtime.

Implement `generate_entry` to emit a structured `LogEntry` from a generator. The library and CLI engines pass it to outputs and retain its fields. The default returns `None` and uses `generate_record` for existing text generators.

Use `models::LogEntry` to store a timestamp, type, level, message, optional metadata, and custom fields. Call `to_string` or `to_json` to format the entry.

Call `OutputHandler::write_entry` to send a `LogEntry` to an output. JSON outputs retain its metadata and custom fields. ELK output converts response time from seconds to milliseconds.

`OutputSink::write_entry` supports trait objects. A custom sink must implement this method to accept structured entries. The default returns an unsupported error.

Use `output::OutputHandler` to write records to files, multiple files, console, TCP, UDP, or HTTP. Wrap a handler with `json` for structured output. Use `fanout` for multiple destinations. Call `close` to flush output and report errors.

Use `OutputHandler::statistics` to inspect accepted records, failed writes, and TCP reconnects. An accepted write does not prove receiver delivery.

Call `test_connection` to send a test record to a network output. The output format applies to that record. A successful write does not prove receiver processing.

Use `network_info` to inspect the protocol and target of a single network output. TCP and UDP targets use the resolved address. HTTP targets include the endpoint path.

Use `field_mapping` to inspect the effective JSON field names. Use `set_field_mapping` to update names for subsequent records.

Use `file_info` or `force_rotation` on a single file handler. For multi-file output, use `file_info_for_type` and `force_rotation_for_type`. Use `force_rotation` to rotate the latest file for each active type. The rotation preserves existing backups.

Use `file_statistics` on a multi-file handler to inspect file sizes, line counts, and the last three lines for each type. It flushes the buffers first.

If one multi-file rotation fails, earlier files can already have rotated. Shared paths rotate once per call.

Use `engine::GenerationEngine` to combine weighted generators and output handlers in Rust. Pass record timestamps to `run`. Call `close` to finish the generators and outputs. An output error can leave a record in earlier outputs.

Register callbacks with `add_error_handler` to observe generator and output errors. Callbacks also receive errors from generator cleanup and output closure. The engine still returns the errors to the caller.

Call `set_continue_on_error(true)` to continue after a record generation or write error. The engine calls error handlers and increments the error count. A failed generation consumes its input timestamp. A successful generation counts as one record even if an output fails. Flush and close errors still return an error.

Use `add_fallible_error_handler` for a callback that can return an I/O error. The engine reports that error on standard error and calls the other callbacks.

Use `run_background` to run an engine on a worker thread. The returned handle reports progress and can pause, resume, or stop the run. Call `wait` to receive statistics and errors. An in-progress record completes before pause or stop takes effect.

Call `statistics` on the handle to inspect counts by type, errors, and the current generation rate. The snapshot does not stop the run.

Call `snapshot` to read status and statistics together. Its generated count matches the statistics count.

Implement `output::OutputSink` to add a custom destination to the engine. The sink receives the type, timestamp, and record.

Use `create_builtin` to create a built-in generator for the engine. Pass a YAML value with type settings and a seed. Pass `null` for the default settings.

Use `GeneratorRegistry::with_builtins` to register the built-in generators with one seed. The registry can also accept custom generators.

The registry seed applies to `create`. The CLI engine uses its `--seed` value for built-in generators.

Use `config::Configuration` to load, check, inspect, and save YAML or JSON configurations from Rust. Save does not replace a file.

The configuration API resolves a relative `pattern_file` from the source directory. The saved configuration keeps an absolute pattern path.

Use `models::GenerationStatistics` to count records and errors. Set the start and end times before the rate calculation.

Use `performance::PerformanceProfiler` to measure named operations. Read each profile's count and duration statistics in seconds. A profile span stops when its scope ends.

Use `performance::PerformanceMonitor` to record process memory, CPU, and generation metrics. Its history has a configurable limit. CPU usage needs two samples with an interval. Thread count is available on Linux.

Use `performance::MemoryTracker` to compare current process memory with its baseline. The system percentage uses host memory, not a container limit. `check_limit` reports an error when usage exceeds the requested limit.

Use `performance::WorkPool` to run tasks on a fixed worker pool. Each handle returns a result or a panic error. Call `shutdown` to wait for all tasks.

Use `performance::BackgroundPerformanceMonitor` to collect samples at an interval and call a callback. Stop the monitor to collect thread errors.

Use `performance::BatchProcessor` to send batches to registered handlers. The size or elapsed interval triggers a flush when an item arrives. Call `flush` to process a partial batch. If `add_item` triggers a failed flush, the batch includes that item. A retry starts with the failed handler. A handler that fails after a partial write can receive records again.

Use `LogAnalyzer` to count levels, HTTP status codes, IP addresses, user agents, time distribution, and error patterns. It also reports validation counts and average quality scores. Use `generate_samples` for random, systematic, stratified, or quality-based samples. Use `compare_log_sets` and `quality_report` to compare distributions and assess format quality.

Call `LogStatistics::to_dict` for the original summary fields and ranked limits. The full statistics retain all observed values.

Call `QualityReport::to_dict` for the original report summary. Direct serialization also includes individual validation results.

Call `LogSample::to_dict` for the original sample fields and metadata.

Call `LogComparison::to_dict` for the original comparison summary and distribution counts.

Use `validation::ValidationStatistics` to collect validation results from multiple log types. It reports success rates, common errors, and the average quality score.

Use `LogValidator::with_pattern` to validate a custom log type. Use `add_pattern` to accept another format. Use `add_required_field` to check a field with a regular expression.

Some distributions and custom fields differ from the Python tool. The configuration parser rejects unsupported fields.

## Use a process generator

Set `plugin_command` under a custom log type. Use `plugin_args` for its arguments. Set `plugin_config` for its configuration and `plugin_timeout` for the timeout in seconds.

```yaml
log_generator:
  log_types:
    example:
      enabled: true
      plugin_command: /path/to/generator
      plugin_args: ["--mode", "test"]
      plugin_config:
        prefix: example
      plugin_timeout: 5
```

The command runs with the same permissions as log-forge. It does not use a shell unless the configuration names one. Configuration validation does not start the command.

The process reads one JSON object per line from standard input. It writes one JSON object per line to standard output. It can write diagnostics to standard error.

The first request contains `version`, `action`, `config`, and `seed`. The process returns a validation regex in `pattern`.

```json
{"version":1,"action":"init","config":{"prefix":"example"},"seed":42}
{"version":1,"pattern":"^example-[0-9]+$"}
```

Each later request contains a timestamp. The process returns one record. It can return an `error` string instead.

```json
{"version":1,"action":"generate","timestamp":"2024-01-01T00:00:00+00:00"}
{"version":1,"record":"example-1"}
```

A process can return `supports_shutdown: true` in its init response. On normal completion, log-forge sends a `shutdown` action and expects a version-only response.

If a process exits after an error, it can return `shutdown_complete: true` with the error. The caller does not send another shutdown request. The error must include any cleanup failure.

Use `adapters/python_generator.py` with `module:Class` to run an original Python generator. Install its dependencies and make its module available on `PYTHONPATH`. The adapter passes `plugin_config` to the constructor or `set_config` and applies `custom_fields`. It calls `cleanup` on normal completion. The generator pattern must use Rust-compatible regex syntax. The adapter does not call `validate_log` for each record. A generator without its own time method does not use virtual time. The container image includes the adapter but does not include Python.

The adapter supports constructor parameters named `config` or `custom_config`. It gives `config` precedence if both parameters exist.

If the constructor accepts `format_type` or `rfc_format`, the adapter passes those values from `plugin_config`.

After construction, the adapter also calls `cleanup` if setup or generation fails. It reports a cleanup failure with the original error.

Requests and responses have a one-megabyte limit. A timeout or protocol error stops generation. The process generator is stateful and uses one generation thread.

## Check Prometheus Remote Write

Build the release binary. Run the integration check with Docker.

```sh
cargo build --release
python3 scripts/check_prometheus.py
```

The check starts an ephemeral local Prometheus container. It checks the generated counter and the stale marker. The default image uses UID 65534 for temporary storage.

## Compare original formats

Install the original repository dependencies. Run the comparison with that Python environment.

```sh
cargo build --release
python3 scripts/check_original_formats.py --original /path/to/log-generator --samples 50
```

The script tests each original validator against original and Rust records. It also checks all records with the Rust validator and reports rejection counts.

The original Django validator rejects its own traceback records. The original Goloop validator rejects some of its own module names, source names, and sequence values. The Rust validator accepts these formats.

## Compare CLI performance

Build the release binary. Install the Python dependencies in an environment for the original repository.

```sh
cargo build --release
python3 benchmarks/compare_cli.py --original /path/to/log-generator --python /path/to/python
```

The script runs each CLI three times with 10,000 Nginx access records. It checks the record count and reports elapsed time, throughput, and peak resident memory. The memory check supports macOS and Linux.

The implementations use different data distributions and console formats. The result includes process startup and output capture. It does not isolate language performance.

