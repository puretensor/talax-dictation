"""PT-2124 finding 2: Wayland type_out remap must drive the injector."""

from __future__ import annotations

from talax_pt2124 import (
    effective_injection_mode,
    injection_mode_from_strategy,
    live_injection_mode_for_session,
)


def test_prefix_injector_ignores_wayland_type_out_remap():
    """Negative control: diagnostics remap, the live injector mapping does not."""
    strategy = "type_out"
    effective, ready = effective_injection_mode(
        strategy, "wayland", target_os="linux"
    )
    injector = injection_mode_from_strategy(strategy)

    assert effective == "clipboard_only"
    assert ready is False
    assert injector == "type_out"


def test_wayland_type_out_remap_must_match_the_injector():
    """Live commands.rs must feed the orchestrator the same remap diagnostics advertise."""
    strategy = "type_out"
    effective, ready = effective_injection_mode(
        strategy, "wayland", target_os="linux"
    )
    injector = live_injection_mode_for_session(
        strategy, "wayland", target_os="linux"
    )

    assert effective == "clipboard_only"
    assert ready is False
    assert injector == "clipboard_only", (
        "injector must use the same remap diagnostics advertise, "
        f"got {injector!r}"
    )


def test_non_wayland_type_out_stays_type_out():
    injector = live_injection_mode_for_session(
        "type_out", "x11", target_os="linux"
    )
    assert injector == "type_out"
    clipboard = live_injection_mode_for_session(
        "clipboard", "wayland", target_os="linux"
    )
    assert clipboard == "clipboard"
