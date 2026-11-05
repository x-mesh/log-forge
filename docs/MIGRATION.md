# Migrate from the Python project

## Build and install

Build the Rust binaries from the project directory.

```sh
cargo build --release --locked
```

The build creates `log-forge` and `log-generator`. Both names run the same CLI. The default `cargo run` target is `log-forge`.

Install both commands into an isolated directory.

```sh
cargo install --path . --locked --root /path/to/install
```

## Use an existing configuration

Check the configuration before a run.

```sh
log-generator config validate --file config/default_config.yaml
log-generator start --config config/default_config.yaml --total 10 --no-monitor
```

The repository includes the nine original configuration examples. The parser rejects unknown fields. Update any rejected fields before a run.

Without a configuration file, `start` uses the Python default configuration. `config show` and `config create` also use these defaults. With a configuration file, `log-generator` merges the Python defaults. `log-forge` merges them only with `--original-defaults`. Use `--standalone-config` to use only the file. The merge retains enabled default types that the file does not mention. Use `--types` to select specific types.

An explicit `module_weights` setting replaces the inherited Goloop `modules` default. An explicit FastAPI custom field replaces its inherited top-level equivalent. An explicit custom `pattern_file` replaces the inherited template and fields.

The Rust implementation does not apply `gc_threshold`. It reports a warning when this setting exists. Rust has no cyclic garbage collector.

## Time and output

Use `--virtual-time` to advance timestamps without a real-time delay. Use `--real-time` to wait between records. Use `--seed` and `--start-time` for repeatable output.

The `start` command appends to files. It preserves existing rotated files. If no backup slot is free, rotation fails.

For file output, an existing directory receives `generated_logs.log`. For JSON output, an existing directory receives `generated_logs.json`. The Python JSON handler expects a file path.

Set `global.multi_file.separate_by_type` to `false` to put log types in the same file. The `{log_type}` placeholder uses `all` in that mode.

## Control and extensions

On Unix, start the generator with `--control-socket PATH`. Use that path with `status`, `monitor`, `pause`, `resume`, or `stop`. On Windows, use a simple name in place of the path. Windows uses a local named pipe. The Windows control path needs runtime verification.

Use Rust's `LogGenerator` trait for native extensions. The process plugin protocol supports external generators. The Python adapter requires a separate Python installation and its dependencies.

## Check results

Run the original format comparison with the original Python dependencies.

```sh
python3 scripts/check_original_formats.py --original /path/to/log-generator --samples 50
```

The original Django and Goloop validators reject some records from their own generators. The comparison reports those cases separately.

The Rust and Python distributions and plugin interfaces differ. Compare results for each workload before replacement.
