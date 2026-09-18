# PT-2124 unit `talaxdictati` — landing report

Repo: `talax-dictation` (worktree `/var/tmp/cursor-fleet/pt2124-talaxdictati/wt`).
All three findings were **VALID** at HEAD `02b5082`. No existing test was modified.

## Baseline (before changes)

```
cd /var/tmp/cursor-fleet/pt2124-talaxdictati/wt
python3 -m pytest -q tests
```

Result: **ERROR: file or directory not found: tests** — no tests ran, exit **4**.

No `tests/` tree existed. Workspace pytest cache was empty of nodeids.

## Finding 1 of 3 — Refuse to rewrite a current config that failed AppConfig parse

**Disposition:** VALID

**Mechanism (confirmed at HEAD):** `parse_config` tried `AppConfig` then any TOML that deserialized as `LegacyAppConfig` (all fields optional). A current file with one mistyped field (`pre_roll_ms = "300"`) failed `AppConfig`, succeeded as legacy, and `migrate_legacy_config` copied only hotkey/model/active_profile. `load_or_create_config` then `save_config_to_disk` over the original. Syntactically invalid TOML took the same fallthrough and wrote `AppConfig::default()` on top of the file.

**Fix:**
- Gate the legacy path on a document that has the old `mode` key and lacks current keys (`review_mode`, `injection_strategy`, `vad_enabled`, `pre_roll_ms`, `silence_stop_ms`).
- Add `#[serde(default)]` on `AppConfig` so a missing new key does not fail the current parse.
- On an existing file: persist only a true legacy migration. On strict-parse failure, keep the file and recover typed fields (`recover_partial_app_config`). On unreadable / invalid TOML, keep the file and use in-memory defaults.
- Only create-and-write a default when the config path is `NotFound` (then the home `~/.talax/config.toml` migrate path may run).

**Deviation from the suggested fix:** legacy is classified *before* the `AppConfig` parse. With `#[serde(default)]`, a true legacy file (`mode = "auto-inject"`, no current keys) would otherwise deserialize as current defaults and skip migration. Recovering valid fields (instead of returning only `AppConfig::default()`) matches the suggested reproducer, which requires `loaded.review_mode == "auto_inject"` while leaving the mistyped `pre_roll_ms` on disk.

**Files:** `crates/talax-app/src/commands.rs`, `tests/talax_pt2124.py`, `tests/test_pt2124_finding1_config_clobber.py`

**Pinning tests:**
- `test_prefix_load_clobbers_current_config_on_one_bad_field` — reconstructed pre-fix loader migrates the fixture as legacy and writes `review_first` / `clipboard_only`.
- `test_load_or_create_config_does_not_clobber_current_config_on_one_bad_field` — live `commands.rs` must keep `auto_inject`, `work-devops`, and `pre_roll_ms = "300"` on disk and load those valid fields.
- `test_syntactically_invalid_toml_is_not_replaced_with_defaults` — garbage TOML is not overwritten.

**Negative control (pre-fix source, before the commands.rs change):**

```
python3 -m pytest -q tests/test_pt2124_finding1_config_clobber.py --tb=short
```

```
.FF                                                                      [100%]
=================================== FAILURES ===================================
_ test_load_or_create_config_does_not_clobber_current_config_on_one_bad_field __
tests/test_pt2124_finding1_config_clobber.py:42: in test_load_or_create_config_does_not_clobber_current_config_on_one_bad_field
    assert "auto_inject" in on_disk, f"parse failure must not rewrite the file: {on_disk}"
E   AssertionError: parse failure must not rewrite the file: hotkey = "Ctrl+Shift+Space"
E     model = "small.en-q5_1"
E     review_mode = "review_first"
E     injection_strategy = "clipboard_only"
E     active_profile = "work-devops"
E     vad_enabled = true
E     pre_roll_ms = 300
E     silence_stop_ms = 700
E
E   assert 'auto_inject' in 'hotkey = "Ctrl+Shift+Space"\nmodel = "small.en-q5_1"\nreview_mode = "review_first"\ninjection_strategy = "clipboard_only"\nactive_profile = "work-devops"\nvad_enabled = true\npre_roll_ms = 300\nsilence_stop_ms = 700\n'
________ test_syntactically_invalid_toml_is_not_replaced_with_defaults _________
tests/test_pt2124_finding1_config_clobber.py:54: in test_syntactically_invalid_toml_is_not_replaced_with_defaults
    assert (tmp_path / "config.toml").read_text() == original
E   assert 'hotkey = "Ct...op_ms = 700\n' == 'review_mode ...ot = toml [\n'
2 failed, 1 passed in 0.04s
```

The live loader followed the pre-fix algorithm (unconditional `LegacyAppConfig` + save). The file was rewritten to defaults. After the fix the same tests pass.

**Commit:** `0b878fd` `PT-2124 nexus-1/1: keep current config.toml when AppConfig parse fails`

## Finding 2 of 3 — Honor the Wayland type_out remap in the injector

**Disposition:** VALID

**Mechanism (confirmed at HEAD):** `effective_injection_mode` remapped Linux + Wayland + `type_out` to `("clipboard_only", false)` and diagnostics exposed that. `injection_mode_from_config` still returned `InjectionMode::TypeOut`. `AppState::new` and `save_app_config` fed that into `RecordingOrchestrator::set_injection_mode`.

**Fix:** `injection_mode_from_config` now delegates to `injection_mode_for_session`, which applies `effective_injection_mode` then maps the effective strategy. Call-site signatures are unchanged so existing tests were not edited. The new helper is what the orchestrator and the pinning test both use.

**Files:** `crates/talax-app/src/commands.rs`, `tests/test_pt2124_finding2_wayland_inject.py`

**Pinning tests:**
- `test_prefix_injector_ignores_wayland_type_out_remap` — diagnostics say `clipboard_only`; pre-fix injector mapping stays `type_out`.
- `test_wayland_type_out_remap_must_match_the_injector` — live source must return `clipboard_only` for the injector on Linux/Wayland/`type_out`.
- `test_non_wayland_type_out_stays_type_out` — X11 `type_out` and Wayland `clipboard` are unchanged.

**Negative control (pre-fix source, before the injector remap):**

```
python3 -m pytest -q tests/test_pt2124_finding2_wayland_inject.py --tb=short
```

```
.F.                                                                      [100%]
=================================== FAILURES ===================================
_____________ test_wayland_type_out_remap_must_match_the_injector ______________
tests/test_pt2124_finding2_wayland_inject.py:37: in test_wayland_type_out_remap_must_match_the_injector
    assert injector == "clipboard_only", (
E   AssertionError: injector must use the same remap diagnostics advertise, got 'type_out'
E   assert 'type_out' == 'clipboard_only'
1 failed, 2 passed in 0.04s
```

**Commit:** `231fa5d` `PT-2124 nexus-1/2: drive the injector from the Wayland type_out remap`

## Finding 3 of 3 — Fail HotkeyListener::start when rdev::listen errors

**Disposition:** VALID

**Mechanism (confirmed at HEAD):** `HotkeyListener::start` spawned a thread and immediately returned `Ok(HotkeyHandle)`. Inside the thread, `rdev::listen` errors were only traced. `get_runtime_diagnostics` set `hotkey_ready` from `hotkey_handle.is_some()`, so a listen that failed at once still showed Listening.

**Fix:**
- The listen thread sends a oneshot ack (`Ok` if listen returned, `Err` from rdev).
- `accept_listen_ack` treats timeout as “still blocking / live”, and every other outcome as `HotkeyError::ListenFailed`. `start()` waits 250ms, joins on failure, and does not store a handle.
- `HotkeyHandle::is_listening` is true only while the join handle exists and is not finished.
- `get_runtime_diagnostics` uses `is_listening`, not `is_some()`.

**Files:** `crates/talax-engine/src/hotkey/mod.rs`, `crates/talax-app/src/commands.rs`, `tests/talax_pt2124.py`, `tests/test_pt2124_finding3_hotkey_listen_ack.py`

**Pinning tests:**
- `test_prefix_start_returns_ok_when_listen_fails` — reconstructed pre-fix spawn always stores a handle.
- `test_start_must_not_return_ok_when_listen_fails` — live `start()` must fail the ack and leave no handle.
- `test_listen_timeout_is_treated_as_blocking_success` — a blocking listen is still Ok.
- `test_finished_listener_thread_is_not_hotkey_ready` — a stored handle whose thread has exited is not ready.

**Negative control (pre-fix source, before the ack / readiness change):**

```
python3 -m pytest -q tests/test_pt2124_finding3_hotkey_listen_ack.py --tb=short
```

```
.F.F                                                                     [100%]
=================================== FAILURES ===================================
_______________ test_start_must_not_return_ok_when_listen_fails ________________
tests/test_pt2124_finding3_hotkey_listen_ack.py:27: in test_start_must_not_return_ok_when_listen_fails
    assert start_ok is False, (
E   AssertionError: rdev::listen Err must fail start() and leave hotkey_handle unset
E   assert True is False
______________ test_finished_listener_thread_is_not_hotkey_ready _______________
tests/test_pt2124_finding3_hotkey_listen_ack.py:40: in test_finished_listener_thread_is_not_hotkey_ready
    assert live_hotkey_ready(handle_present=True, thread_finished=True) is False
E   assert True is False
E    +  where True = live_hotkey_ready(handle_present=True, thread_finished=True)
2 failed, 2 passed in 0.04s
```

**Commit:** `662c3b3` `PT-2124 nexus-1/3: fail start() when the hotkey listen thread errors`

## Final gate

```
cd /var/tmp/cursor-fleet/pt2124-talaxdictati/wt
python3 -m pytest -q tests
```

```
..........                                                               [100%]
10 passed in 0.01s
```

Result after the three fixes: **10 passed** (baseline was exit 4, no tests). Existing Rust tests were not modified. `VERSION` / SemVer were not touched.

No pre-existing test file was edited. `VERSION` / SemVer were not touched.
