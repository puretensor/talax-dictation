"""PT-2124 finding 1: do not rewrite a current config that failed AppConfig parse."""

from __future__ import annotations

from pathlib import Path

from talax_pt2124 import (
    FINDING1_ORIGINAL,
    DEFAULT_APP_CONFIG,
    live_load_or_create_config,
    prefix_load_or_create_config,
)


def _write_fixture(tmp_path: Path, contents: str) -> Path:
    path = tmp_path / "config.toml"
    path.write_text(contents)
    return path


def test_prefix_load_clobbers_current_config_on_one_bad_field(tmp_path: Path):
    """Negative control: pre-fix parse_config migrates the file as legacy and saves."""
    _write_fixture(tmp_path, FINDING1_ORIGINAL)
    loaded = prefix_load_or_create_config(tmp_path)
    on_disk = (tmp_path / "config.toml").read_text()

    assert loaded["review_mode"] == DEFAULT_APP_CONFIG["review_mode"]
    assert loaded["injection_strategy"] == DEFAULT_APP_CONFIG["injection_strategy"]
    assert loaded["active_profile"] == "work-devops"
    assert "auto_inject" not in on_disk
    assert "work-devops" in on_disk


def test_load_or_create_config_does_not_clobber_current_config_on_one_bad_field(
    tmp_path: Path,
):
    """Live commands.rs must keep the file and the valid current-format fields."""
    _write_fixture(tmp_path, FINDING1_ORIGINAL)
    loaded = live_load_or_create_config(tmp_path)
    on_disk = (tmp_path / "config.toml").read_text()

    assert "auto_inject" in on_disk, f"parse failure must not rewrite the file: {on_disk}"
    assert "work-devops" in on_disk
    assert 'pre_roll_ms = "300"' in on_disk
    assert loaded["review_mode"] == "auto_inject"
    assert loaded["injection_strategy"] == "clipboard"
    assert loaded["active_profile"] == "work-devops"


def test_syntactically_invalid_toml_is_not_replaced_with_defaults(tmp_path: Path):
    original = "review_mode = \"auto_inject\"\nthis is not = toml [\n"
    _write_fixture(tmp_path, original)
    live_load_or_create_config(tmp_path)
    assert (tmp_path / "config.toml").read_text() == original
