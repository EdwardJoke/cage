# Tests for the Cage Python bindings
#
# Run with:  python -m pytest cage-py/tests/ -v

from __future__ import annotations

import json
from pathlib import Path

import pytest

from cage import Policy, Sandbox

# Locate the compiled agent-p0.wasm
HERE = Path(__file__).resolve().parent
PROJECT_ROOT = HERE.parent.parent  # cage-py/../ = project root
AGENT_P0 = PROJECT_ROOT / "target" / "wasm32-wasip1" / "release" / "agent_p0.wasm"


def _agent_p0() -> str:
    """Return the path to the agent-p0 WASM binary."""
    if not AGENT_P0.exists():
        pytest.skip(f"agent-p0.wasm not found at {AGENT_P0}")
    return str(AGENT_P0)


# ── Tests ────────────────────────────────────────────────────────────


def test_basic_lifecycle():
    """Init + tick with a well-formed policy."""
    policy = (
        Policy(fuel=500_000)
        .env("MY_SECRET", "sup3rs3cr3t")
        .allow_url("https://httpbin.org")
    )

    box = Sandbox(_agent_p0(), policy)
    try:
        msgs = box.run({"hello": "world"})
        assert len(msgs) >= 2, f"expected ≥2 messages, got {len(msgs)}"

        # First message should be init_complete
        init_msg = msgs[0]
        assert init_msg["kind"] == "init_complete"
        payload = init_msg["payload"]
        assert payload["init_payload"] == {"hello": "world"}
        assert payload["env_found"]["MY_SECRET"] == "sup3rs3cr3t"

        # Second message should be tick_complete
        tick_msg = msgs[1]
        assert tick_msg["kind"] == "tick_complete"

        stats = box.stats()
        assert stats["tick_count"] >= 1
        assert stats["fuel_consumed"] > 0
    finally:
        del box


def test_context_manager():
    """Sandbox can be used as a context manager."""
    policy = Policy(fuel=500_000)
    with Sandbox(_agent_p0(), policy) as box:
        msgs = box.run({"ping": "pong"})
        assert len(msgs) >= 1


def test_fuel_exhaustion():
    """Insufficient fuel should raise RuntimeError."""
    policy = Policy(fuel=100)  # way too small
    with pytest.raises(RuntimeError, match="out of fuel|trap"):
        Sandbox(_agent_p0(), policy).run({"hello": "world"})


def test_url_whitelist_rejection():
    """URLs not in the whitelist must be rejected."""
    policy = Policy(fuel=500_000).allow_url("https://example.com")  # NOT httpbin.org

    box = Sandbox(_agent_p0(), policy)
    msgs = box.run({})

    # The tick message should report the denied URL
    tick_msgs = [m for m in msgs if m["kind"] == "tick_complete"]
    assert len(tick_msgs) >= 1
    payload = tick_msgs[0]["payload"]
    response_str = payload.get("http_response", "{}")
    response = (
        json.loads(response_str) if isinstance(response_str, str) else response_str
    )
    assert "URL not allowed" in str(response)


def test_env_injection():
    """Injected env vars should appear in env_found; unknown ones in env_missing."""
    policy = Policy(fuel=500_000).env("SECRET", "shh")
    box = Sandbox(_agent_p0(), policy)
    msgs = box.run({})

    init_msgs = [m for m in msgs if m["kind"] == "init_complete"]
    assert len(init_msgs) >= 1
    payload = init_msgs[0]["payload"]

    assert payload["env_found"]["SECRET"] == "shh"
    # HOME / USER / PATH / SHELL are NOT in the injected env
    for key in ("HOME", "USER", "PATH", "SHELL"):
        assert key in payload["env_missing"], f"{key} should be in env_missing"


def test_stats_after_run():
    """stats() should report non-zero fuel after execution."""
    policy = Policy(fuel=500_000)
    box = Sandbox(_agent_p0(), policy)
    box.run({"hello": "world"})
    stats = box.stats()
    assert stats["fuel_consumed"] > 0
    assert stats["tick_count"] >= 1


def test_empty_init_message():
    """Agent should handle None / empty init message gracefully."""
    policy = Policy(fuel=500_000)
    box = Sandbox(_agent_p0(), policy)
    # Pass empty dict
    msgs = box.run({})
    assert len(msgs) >= 1
    assert msgs[0]["kind"] == "init_complete"
