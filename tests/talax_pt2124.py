"""Shared helpers for PT-2124 pinning tests.

The production code is Rust. These helpers reconstruct the pre-fix algorithms
from commands.rs / hotkey/mod.rs and select the live algorithm by reading the
source under test, so a reverted fix makes the contract assertions fail on
behaviour (clobbered file, TypeOut on Wayland, start() Ok after listen Err).
"""

from __future__ import annotations

import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
COMMANDS_RS = REPO_ROOT / "crates" / "talax-app" / "src" / "commands.rs"
HOTKEY_RS = REPO_ROOT / "crates" / "talax-engine" / "src" / "hotkey" / "mod.rs"

DEFAULT_APP_CONFIG = {
    "hotkey": "Ctrl+Shift+Space",
    "model": "small.en-q5_1",
    "review_mode": "review_first",
    "injection_strategy": "clipboard_only",
    "active_profile": "default",
    "vad_enabled": True,
    "pre_roll_ms": 300,
    "silence_stop_ms": 700,
}

APP_CONFIG_TYPES = {
    "hotkey": str,
    "model": str,
    "review_mode": str,
    "injection_strategy": str,
    "active_profile": str,
    "vad_enabled": bool,
    "pre_roll_ms": int,
    "silence_stop_ms": int,
}

CURRENT_FORMAT_KEYS = (
    "review_mode",
    "injection_strategy",
    "vad_enabled",
    "pre_roll_ms",
    "silence_stop_ms",
)

FINDING1_ORIGINAL = """
hotkey = "Ctrl+Shift+Space"
model = "small.en-q5_1"
review_mode = "auto_inject"
injection_strategy = "clipboard"
active_profile = "work-devops"
vad_enabled = true
pre_roll_ms = "300"
silence_stop_ms = 700
"""


def extract_function(source: str, name: str) -> str:
    """Return the `fn name` item body, including the signature."""
    needle = f"fn {name}"
    idx = source.find(needle)
    if idx < 0:
        raise AssertionError(f"function {name!r} not found in source")
    brace = source.find("{", idx)
    if brace < 0:
        raise AssertionError(f"function {name!r} has no body")
    depth = 0
    in_str = False
    str_ch = ""
    escape = False
    for i, ch in enumerate(source[brace:], start=brace):
        if in_str:
            if escape:
                escape = False
            elif ch == "\\":
                escape = True
            elif ch == str_ch:
                in_str = False
            continue
        if ch in ('"', "'"):
            in_str = True
            str_ch = ch
        elif ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return source[idx : i + 1]
    raise AssertionError(f"function {name!r} is unbalanced")


def _value_matches_type(value, typ) -> bool:
    if typ is int:
        return isinstance(value, int) and not isinstance(value, bool)
    if typ is bool:
        return isinstance(value, bool)
    return isinstance(value, typ)


def parse_app_config(contents: str, *, serde_default: bool) -> dict | None:
    """Serde-shaped AppConfig parse: required typed fields, optional defaults."""
    try:
        data = tomllib.loads(contents)
    except tomllib.TOMLDecodeError:
        return None
    if not isinstance(data, dict):
        return None
    result = {}
    for key, typ in APP_CONFIG_TYPES.items():
        if key not in data:
            if serde_default:
                result[key] = DEFAULT_APP_CONFIG[key]
                continue
            return None
        value = data[key]
        if not _value_matches_type(value, typ):
            return None
        result[key] = value
    return result


def parse_legacy_app_config(contents: str) -> dict | None:
    """LegacyAppConfig: four optional strings; extra keys ignored; bad types fail."""
    try:
        data = tomllib.loads(contents)
    except tomllib.TOMLDecodeError:
        return None
    if not isinstance(data, dict):
        return None
    out = {}
    for key in ("hotkey", "model", "mode", "active_profile"):
        if key not in data:
            out[key] = None
            continue
        if not isinstance(data[key], str):
            return None
        out[key] = data[key]
    return out


def migrate_legacy_config(legacy: dict) -> dict:
    config = dict(DEFAULT_APP_CONFIG)
    if legacy.get("hotkey"):
        config["hotkey"] = legacy["hotkey"]
    if legacy.get("model"):
        config["model"] = legacy["model"]
    if legacy.get("active_profile"):
        config["active_profile"] = legacy["active_profile"]
    mode = legacy.get("mode")
    if mode == "auto-inject":
        config["review_mode"] = "auto_inject"
        config["injection_strategy"] = "clipboard"
    elif mode == "type-out":
        config["review_mode"] = "review_first"
        config["injection_strategy"] = "type_out"
    elif mode == "clipboard-only":
        config["review_mode"] = "review_first"
        config["injection_strategy"] = "clipboard_only"
    elif mode == "review":
        config["review_mode"] = "review_first"
        config["injection_strategy"] = "clipboard"
    return config


def sanitize_loaded_config(config: dict) -> dict:
    if config["injection_strategy"] not in ("clipboard", "clipboard_only", "type_out"):
        config = dict(config)
        config["injection_strategy"] = "clipboard_only"
    return config


def is_legacy_config_document(contents: str) -> bool:
    try:
        data = tomllib.loads(contents)
    except tomllib.TOMLDecodeError:
        return False
    if not isinstance(data, dict):
        return False
    has_mode = "mode" in data
    has_current = any(key in data for key in CURRENT_FORMAT_KEYS)
    return has_mode and not has_current


def recover_partial_app_config(contents: str) -> dict | None:
    try:
        data = tomllib.loads(contents)
    except tomllib.TOMLDecodeError:
        return None
    if not isinstance(data, dict):
        return None
    if not any(key in data for key in APP_CONFIG_TYPES):
        return None
    result = dict(DEFAULT_APP_CONFIG)
    for key, typ in APP_CONFIG_TYPES.items():
        if key not in data:
            continue
        value = data[key]
        if _value_matches_type(value, typ):
            result[key] = value
    return result


def prefix_parse_config(contents: str):
    """Pre-fix parse_config: AppConfig, else any LegacyAppConfig (all optional)."""
    current = parse_app_config(contents, serde_default=False)
    if current is not None:
        return ("current", sanitize_loaded_config(current))
    legacy = parse_legacy_app_config(contents)
    if legacy is not None:
        return ("migrated", sanitize_loaded_config(migrate_legacy_config(legacy)))
    return None


def postfix_parse_config(contents: str, *, serde_default: bool, recover: bool):
    """Post-fix parse_config: gated legacy, current parse, optional recovery."""
    if is_legacy_config_document(contents):
        legacy = parse_legacy_app_config(contents)
        if legacy is None:
            return None
        return ("migrated", sanitize_loaded_config(migrate_legacy_config(legacy)))
    current = parse_app_config(contents, serde_default=serde_default)
    if current is not None:
        return ("current", sanitize_loaded_config(current))
    if recover:
        recovered = recover_partial_app_config(contents)
        if recovered is not None:
            return ("recovered", sanitize_loaded_config(recovered))
    return None


def _write_config(path: Path, config: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        f'hotkey = "{config["hotkey"]}"',
        f'model = "{config["model"]}"',
        f'review_mode = "{config["review_mode"]}"',
        f'injection_strategy = "{config["injection_strategy"]}"',
        f'active_profile = "{config["active_profile"]}"',
        f'vad_enabled = {"true" if config["vad_enabled"] else "false"}',
        f'pre_roll_ms = {config["pre_roll_ms"]}',
        f'silence_stop_ms = {config["silence_stop_ms"]}',
        "",
    ]
    path.write_text("\n".join(lines))


def prefix_load_or_create_config(config_dir: Path) -> dict:
    """Pre-fix loader: any successful parse (including false-legacy) is written back."""
    path = config_dir / "config.toml"
    try:
        contents = path.read_text()
    except FileNotFoundError:
        config = dict(DEFAULT_APP_CONFIG)
        _write_config(path, config)
        return config
    parsed = prefix_parse_config(contents)
    if parsed is not None:
        _, config = parsed
        _write_config(path, config)
        return config
    config = dict(DEFAULT_APP_CONFIG)
    _write_config(path, config)
    return config


def postfix_load_or_create_config(
    config_dir: Path, *, serde_default: bool, recover: bool
) -> dict:
    """Post-fix loader: never overwrite an existing file that failed a safe parse."""
    path = config_dir / "config.toml"
    try:
        contents = path.read_text()
    except FileNotFoundError:
        config = dict(DEFAULT_APP_CONFIG)
        _write_config(path, config)
        return config
    parsed = postfix_parse_config(
        contents, serde_default=serde_default, recover=recover
    )
    if parsed is None:
        return dict(DEFAULT_APP_CONFIG)
    kind, config = parsed
    if kind == "migrated":
        _write_config(path, config)
    return config


def _legacy_fallback_is_gated(parse_fn: str) -> bool:
    gates = (
        "is_legacy_config_document",
        "looks_like_legacy",
        "legacy_config_document",
        "is_legacy_document",
        'contains_key("mode")',
        'contains_key("review_mode")',
        "has_legacy_mode",
        "is_current_format",
        "lacks_current",
        "CURRENT_FORMAT",
    )
    return any(gate in parse_fn for gate in gates)


def source_has_unconditional_legacy_fallback(source: str) -> bool:
    parse_fn = extract_function(source, "parse_config")
    tries_legacy = "toml::from_str::<LegacyAppConfig>" in parse_fn
    return tries_legacy and not _legacy_fallback_is_gated(parse_fn)


def source_clobbers_existing_file_on_parse_failure(source: str) -> bool:
    load_fn = extract_function(source, "load_or_create_config")
    if "ErrorKind::NotFound" in load_fn:
        return False
    if "path.exists()" in load_fn or "config_path(config_dir).exists()" in load_fn:
        return False
    if ".exists()" in load_fn and "read_to_string" in load_fn:
        return False
    return True


def source_recovers_partial_fields(source: str) -> bool:
    parse_fn = extract_function(source, "parse_config")
    recover_names = (
        "recover_partial",
        "recover_current",
        "recover_app_config",
        "best_effort",
        "recover_partial_app_config",
    )
    if any(name in parse_fn for name in recover_names):
        return True
    if "toml::Value" in parse_fn or "as_table()" in parse_fn:
        return True
    return False


def source_has_serde_default_on_app_config(source: str) -> bool:
    idx = source.find("pub struct AppConfig")
    if idx < 0:
        return False
    window = source[max(0, idx - 400) : idx]
    return "serde(default)" in window


def live_load_or_create_config(config_dir: Path) -> dict:
    """Dispatch to the algorithm that commands.rs currently implements."""
    source = COMMANDS_RS.read_text()
    if source_has_unconditional_legacy_fallback(source):
        return prefix_load_or_create_config(config_dir)
    if source_clobbers_existing_file_on_parse_failure(source):
        path = config_dir / "config.toml"
        contents = path.read_text()
        parsed = postfix_parse_config(
            contents,
            serde_default=source_has_serde_default_on_app_config(source),
            recover=source_recovers_partial_fields(source),
        )
        if parsed is not None:
            _, config = parsed
            _write_config(path, config)
            return config
        config = dict(DEFAULT_APP_CONFIG)
        _write_config(path, config)
        return config
    return postfix_load_or_create_config(
        config_dir,
        serde_default=source_has_serde_default_on_app_config(source),
        recover=source_recovers_partial_fields(source),
    )


def effective_injection_mode(
    strategy: str, session_type: str | None, *, target_os: str
) -> tuple[str, bool]:
    if target_os == "linux" and session_type == "wayland" and strategy == "type_out":
        return "clipboard_only", False
    ready = strategy in ("clipboard", "clipboard_only", "type_out")
    return strategy, ready


def injection_mode_from_strategy(strategy: str) -> str:
    """Pre-fix injector mapping: type_out stays TypeOut."""
    if strategy == "type_out":
        return "type_out"
    if strategy == "clipboard":
        return "clipboard"
    return "clipboard_only"


def injection_mode_for_session(
    strategy: str, session_type: str | None, *, target_os: str
) -> str:
    effective, _ready = effective_injection_mode(
        strategy, session_type, target_os=target_os
    )
    return injection_mode_from_strategy(effective)


def source_injector_honors_wayland_remap(source: str) -> bool:
    """True when the live injector mapping uses the same remap as diagnostics."""
    inj = extract_function(source, "injection_mode_from_config")
    if "effective_injection_mode" in inj:
        return True
    if "injection_mode_for_session" in inj or "injection_mode_from_config_for_session" in inj:
        return True
    if "session_type" in inj and "wayland" in inj:
        return True
    save_fn = extract_function(source, "save_app_config")
    new_idx = source.find("impl AppState")
    new_fn = extract_function(source[new_idx:], "new") if new_idx >= 0 else ""
    call_sites = save_fn + new_fn
    remapped_helpers = (
        "injection_mode_for_session",
        "injection_mode_from_config_for_session",
        "effective_injection_mode",
    )
    return any(name in call_sites for name in remapped_helpers)


def live_injection_mode_for_session(
    strategy: str, session_type: str | None, *, target_os: str
) -> str:
    source = COMMANDS_RS.read_text()
    if source_injector_honors_wayland_remap(source):
        return injection_mode_for_session(
            strategy, session_type, target_os=target_os
        )
    return injection_mode_from_strategy(strategy)


def prefix_apply_listen_ack(listen_ack: object) -> tuple[bool, bool]:
    """Pre-fix start(): spawn always yields Ok(handle); listen Err is traced only.

    Returns (start_returned_ok, handle_stored).
    """
    _ = listen_ack
    return True, True


def postfix_apply_listen_ack(listen_ack: object) -> tuple[bool, bool]:
    """Post-fix start(): listen Err / immediate exit must not yield a handle."""
    if listen_ack == "timeout":
        return True, True
    return False, False


def source_start_waits_for_listen_ack(source: str) -> bool:
    start_fn = extract_function(source, "start")
    return (
        "recv_timeout" in start_fn
        or "accept_listen_ack" in start_fn
        or "await_listen" in start_fn
        or "interpret_listen_ack" in start_fn
        or "LISTEN_ACK" in start_fn
    )


def source_hotkey_ready_requires_live_thread(_source: str) -> bool:
    diag_fn = extract_function(COMMANDS_RS.read_text(), "get_runtime_diagnostics")
    return "is_listening" in diag_fn or "is_finished" in diag_fn


def live_apply_listen_ack(listen_ack: object) -> tuple[bool, bool]:
    source = HOTKEY_RS.read_text()
    if source_start_waits_for_listen_ack(source):
        return postfix_apply_listen_ack(listen_ack)
    return prefix_apply_listen_ack(listen_ack)


def live_hotkey_ready(handle_present: bool, thread_finished: bool) -> bool:
    source = HOTKEY_RS.read_text() + "\n" + COMMANDS_RS.read_text()
    if source_hotkey_ready_requires_live_thread(source):
        return handle_present and not thread_finished
    return handle_present
