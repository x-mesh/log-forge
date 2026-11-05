#!/usr/bin/env python3
import contextlib
import importlib
import inspect
import json
import random
import sys
from datetime import datetime


def respond(payload):
    sys.stdout.write(json.dumps(payload, ensure_ascii=False, allow_nan=False) + "\n")
    sys.stdout.flush()


def cleanup_error(generator):
    cleanup = getattr(generator, "cleanup", None)
    if callable(cleanup):
        try:
            with contextlib.redirect_stdout(sys.stderr):
                cleanup()
        except Exception as error:
            return f"{type(error).__name__}: {error}"
    return None


def create(spec, config, seed):
    module_name, separator, class_name = spec.partition(":")
    if not separator or not module_name or not class_name:
        raise ValueError("generator must use module:Class syntax")
    random.seed(seed)
    module = importlib.import_module(module_name)
    generator_class = getattr(module, class_name)
    if not isinstance(config, dict):
        raise ValueError("plugin_config must be an object")
    config = dict(config)
    statuses = config.get("status_codes")
    if isinstance(statuses, dict):
        config["status_codes"] = {
            int(key) if isinstance(key, str) and key.isdecimal() else key: value
            for key, value in statuses.items()
        }
    if isinstance(config.get("custom_fields"), dict):
        fields = dict(config["custom_fields"])
        statuses = fields.get("status_codes")
        if isinstance(statuses, dict):
            fields["status_codes"] = {
                int(key) if isinstance(key, str) and key.isdecimal() else key: value
                for key, value in statuses.items()
            }
        config["custom_fields"] = fields
    parameters = inspect.signature(generator_class.__init__).parameters
    kwargs = {
        name: config[name]
        for name in ("format_type", "rfc_format")
        if name in parameters and name in config
    }
    if "config" in parameters:
        generator = generator_class(config=config, **kwargs)
    elif "custom_config" in parameters:
        generator = generator_class(custom_config=config, **kwargs)
    else:
        generator = generator_class(**kwargs)
    try:
        if "config" not in parameters and "custom_config" not in parameters:
            setter = getattr(generator, "set_config", None)
            if callable(setter):
                setter(config)
        if config.get("custom_fields"):
            setter = getattr(generator, "set_custom_fields", None)
            if callable(setter):
                setter(config["custom_fields"])
        faker = getattr(generator, "faker", None)
        if callable(getattr(faker, "seed_instance", None)):
            faker.seed_instance(seed)
        for method in ("generate_log", "get_log_pattern", "validate_log"):
            if not callable(getattr(generator, method, None)):
                raise TypeError(f"generator lacks {method}")
    except Exception as error:
        failure = cleanup_error(generator)
        if failure:
            raise RuntimeError(f"{type(error).__name__}: {error}; cleanup failed: {failure}") from error
        raise
    timed = getattr(generator, "generate_log_with_time", None)
    implementation = getattr(timed, "__func__", None)
    if not callable(timed) or (
        getattr(implementation, "__module__", "") == "log_generator.core.interfaces"
        and getattr(implementation, "__qualname__", "") == "LogGenerator.generate_log_with_time"
    ):
        print("warning: generator does not apply the requested timestamp", file=sys.stderr)
    return generator


def main():
    if len(sys.argv) != 2:
        raise ValueError("expected module:Class")
    generator = None
    for line in sys.stdin:
        action = None
        try:
            request = json.loads(line)
            if request.get("version") != 1:
                raise ValueError("unsupported protocol version")
            action = request.get("action")
            with contextlib.redirect_stdout(sys.stderr):
                if action == "init" and generator is None:
                    config = request.get("config")
                    generator = create(sys.argv[1], {} if config is None else config, request["seed"])
                    pattern = generator.get_log_pattern()
                    if not isinstance(pattern, str) or not pattern:
                        raise ValueError("generator pattern must be a nonempty string")
                    response = {"version": 1, "pattern": pattern, "supports_shutdown": True}
                elif action == "generate" and generator is not None:
                    timestamp = datetime.fromisoformat(request["timestamp"].replace("Z", "+00:00"))
                    timed = getattr(generator, "generate_log_with_time", None)
                    record = timed(timestamp) if callable(timed) else generator.generate_log()
                    if not isinstance(record, str) or not record:
                        raise ValueError("generator record must be a nonempty string")
                    response = {"version": 1, "record": record}
                elif action == "shutdown" and generator is not None:
                    cleanup = getattr(generator, "cleanup", None)
                    if callable(cleanup):
                        cleanup()
                    response = {"version": 1}
                else:
                    raise ValueError("unexpected protocol action")
            respond(response)
            if action == "shutdown":
                return
        except Exception as error:
            message = f"{type(error).__name__}: {error}"
            if generator is not None and action in ("init", "generate"):
                failure = cleanup_error(generator)
                if failure:
                    message += f"; cleanup failed: {failure}"
            response = {"version": 1, "error": message}
            if generator is not None and action == "generate":
                response["shutdown_complete"] = True
            respond(response)
            return


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"{type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(1)
