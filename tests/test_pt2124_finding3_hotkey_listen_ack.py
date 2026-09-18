"""PT-2124 finding 3: start() must fail when rdev::listen errors."""

from __future__ import annotations

from talax_pt2124 import (
    live_apply_listen_ack,
    live_hotkey_ready,
    prefix_apply_listen_ack,
)


def test_prefix_start_returns_ok_when_listen_fails():
    """Negative control: pre-fix start() always stores a handle after spawn."""
    start_ok, handle_stored = prefix_apply_listen_ack(Exception("listen failed"))
    assert start_ok is True
    assert handle_stored is True


def test_start_must_not_return_ok_when_listen_fails():
    listen_ack = Exception("listen failed")

    prefix_ok, prefix_handle = prefix_apply_listen_ack(listen_ack)
    assert prefix_ok is True
    assert prefix_handle is True

    start_ok, handle_stored = live_apply_listen_ack(listen_ack)
    assert start_ok is False, (
        "rdev::listen Err must fail start() and leave hotkey_handle unset"
    )
    assert handle_stored is False


def test_listen_timeout_is_treated_as_blocking_success():
    start_ok, handle_stored = live_apply_listen_ack("timeout")
    assert start_ok is True
    assert handle_stored is True


def test_finished_listener_thread_is_not_hotkey_ready():
    assert live_hotkey_ready(handle_present=True, thread_finished=True) is False
    assert live_hotkey_ready(handle_present=True, thread_finished=False) is True
    assert live_hotkey_ready(handle_present=False, thread_finished=True) is False
