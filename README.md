# log-forge

Generate synthetic logs for pipeline tests, parsers, and monitoring systems. log-forge is a Rust port of a Python log generator. It keeps the log types of that original project and adds Redis, SSH, Kafka, and HAProxy logs. If you use the Python project, read the [migration guide](docs/MIGRATION.md).

## Install

On Linux or macOS, run the install script. It installs `log-forge` and `log-generator` to `~/.local/bin`.

```sh
curl -fsSL https://raw.githubusercontent.com/x-mesh/log-forge/main/install.sh | sh
```

Set `LOG_FORGE_VERSION` to install a specific release, for example `0.1.0`. Set `LOG_FORGE_INSTALL_DIR` to use a different directory. The script checks the SHA-256 checksum of the archive. On Windows, download the zip archive from the [releases page](https://github.com/x-mesh/log-forge/releases).

## Quick start

To build from source, install Rust 1.99.1. CI uses this version. Build the release binaries.

```sh
cargo build --release --locked
```

Generate three Redis records on stdout.

```sh
./target/release/log-forge start --output console --types redis --total 3 --seed 42 --start-time '2024-01-01 00:00:00' --no-monitor
```

The command prints these records:

```text
99415:M 01 Jan 2024 00:00:00.000 * DB saved on disk
1289:M 01 Jan 2024 00:00:00.100 - 87 clients connected (2 replicas), 3609885 bytes in use
78000:M 01 Jan 2024 00:00:00.200 * Background saving terminated with success
```

List the available types with `./target/release/log-forge list-types`.

## Write logs to a file

Run `log-forge` without a subcommand to write one log type to a file. This direct mode has its own options. Use a new output path. If the file exists, this command replaces it.

```sh
./target/release/log-forge --total 10000 --output access.log --seed 42 --start 2024-01-01T00:00:00Z
```

The direct mode uses the start time for each record. Omit `--start` to use the current UTC time. Use `--log-type` to select a type. The default is `nginx_access`. Use `--format common` for Common Log Format. The default is Combined Log Format.

## Use a configuration file

A configuration file selects several log types and their frequencies. Run `init` to create one. In a terminal, it asks for the log types, output, record count, and start time. Press Enter to accept a default.

```sh
./target/release/log-forge init
```

Each question has an option. Add `--yes` to skip the questions, for example in a script:

```sh
./target/release/log-forge init --types nginx_access=0.7,syslog=0.3 --start-time '2024-01-01 00:00:00' --yes
```

The command does not replace an existing file. You can also write the file yourself. Save this configuration as `config.yaml`.

```yaml
log_generator:
  global:
    output_format: multi_file
    output_path: ./logs
    total_logs: 10000
    generation_interval: 0.1
    virtual_time: true
    start_time: '2024-01-01 00:00:00'
  log_types:
    nginx_access:
      enabled: true
      frequency: 0.7
    syslog:
      enabled: true
      frequency: 0.3
```

```sh
./target/release/log-forge start --config config.yaml --seed 42
```

The command writes `nginx_access_20240101.log` and `syslog_20240101.log` to `./logs`. It reports the record counts on stderr.

## Check generated logs

```sh
./target/release/log-forge validate --input ./logs
./target/release/log-forge validate --input access.log --type nginx_access
./target/release/log-forge analyze --input access.log --type nginx_access --pretty
./target/release/log-forge sample --type nginx_access --count 5
```

The `validate` command infers a type from a file name such as `nginx_access_20240101.log`. Use `--type` if the file name does not include a type. The command reports invalid records and returns a nonzero exit code. It groups MySQL slow logs and Django tracebacks into records.

The `analyze` command reads the same files and prints statistics as JSON. The `sample` command creates new records on stdout. Use `--seed` and `--start-time` to reproduce a sample.

The analysis includes validation rate, error rate, average quality score, and field completeness score. These metrics use every input record. Empty input has no quality metrics.

Use `--type custom --pattern 'REGEX'` with `validate` or `analyze` to check a custom format. For a built-in type, the pattern adds an accepted format. The pattern uses Rust regex syntax.

## Run in a container

Build the image. Generate three records.

```sh
docker build -t log-forge .
docker run --rm log-forge start --output console --total 3 --no-monitor
```

The image runs as a non-root user. It includes the example configurations and pattern files.

## Log types

The generator provides these built-in types:

- Web and application: `nginx_access`, `nginx_error`, `apache_access`, `apache_error`, `django_request`, `django_sql`, and `fastapi`.
- Infrastructure: `docker`, `kubernetes_pod`, `kubernetes_event`, `syslog`, and `goloop`.
- Data and security: `mysql_query`, `mysql_error`, `mysql_slow`, `postgresql`, `postgresql_slow`, `redis`, `ssh_auth`, and `kafka_broker`.
- Load balancer: `haproxy`.

The CLI also supports custom templates, scenarios, service logs, error patterns, and process generators. The [reference](docs/REFERENCE.md#configure-log-types) describes the settings for each type.

Use `--seed` and `--start-time` to reproduce output with the same version. Virtual time advances by the configured interval. MySQL slow query records span multiple lines. The output count refers to records.

## Configuration and output

Use `config create --file config.yaml` to create a configuration. The command does not replace an existing file. Use `config validate --file config.yaml` to check a file. Use `config show --file config.yaml` to display it.

The build also provides `log-generator`. Both commands use the same options. Their configuration defaults and error handling differ.

Without a file, both commands use the original defaults. With a file, `log-generator` merges those defaults. `log-forge` uses only the file unless you add `--original-defaults`. Use `--standalone-config` to disable the merge.

The `start` command supports console, file, multi-file, JSON, TCP, UDP, and HTTP outputs. File output appends to existing files. If rotation reaches the backup limit, it preserves backups and reports an error. HTTP output uses plain HTTP without TLS.

`log-generator` reports record errors and continues by default. Use `--fail-fast` to stop on an error. `log-forge` stops by default. Use `--continue-on-error` to continue. In that mode, the command reports close errors on stderr and still exits successfully.

## Release

Set the new version in `Cargo.toml`. Run `make check`. Push a tag that matches the version, for example `v0.1.0`. The release workflow builds archives for Linux, macOS, and Windows and publishes them with checksums.

## Documentation

- [CLI and API reference](docs/REFERENCE.md): output, control commands, generator options, patterns, and extensions.
- [Migration guide](docs/MIGRATION.md): configuration and CLI differences from the Python project.
- [Migration checks](docs/PARITY.md): tested patterns, execution environments, and known differences.
- [Examples](examples/): Rust examples for generators, output, analysis, and patterns.
