"""Tests for cage PyO3 orchestrator bindings."""

import os
import pytest
from cage import Orchestrator, RoundSummary


# ── WASM paths (adjust if your build output is elsewhere) ────────────

AGENT_P0_PATH = os.path.join(
    os.path.dirname(__file__),
    "..",
    "..",
    "examples",
    "agent-p0",
    "target",
    "wasm32-wasip1",
    "release",
    "agent_p0.wasm",
)

AGENT_P1_PATH = os.path.join(
    os.path.dirname(__file__),
    "..",
    "..",
    "examples",
    "agent-p1",
    "target",
    "wasm32-wasip1",
    "release",
    "agent_p1.wasm",
)

HAS_AGENT_P0 = os.path.exists(AGENT_P0_PATH)
HAS_AGENT_P1 = os.path.exists(AGENT_P1_PATH)

NEED_BUILD_MSG = (
    "WASM agent not built – run:\n"
    "  cargo build -p agent-p0 --target wasm32-wasip1 --release\n"
    "  cargo build -p agent-p1 --target wasm32-wasip1 --release"
)

wasm_test = pytest.mark.skipif(not HAS_AGENT_P0, reason=NEED_BUILD_MSG)
wasm_p1_test = pytest.mark.skipif(not HAS_AGENT_P1, reason=NEED_BUILD_MSG)


# ── Construction ─────────────────────────────────────────────────────


def test_create_orchestrator():
    """Orchestrator() creates an empty orchestrator."""
    orch = Orchestrator()
    assert orch is not None
    assert orch.agent_count() == 0


# ── Spawn / Kill ─────────────────────────────────────────────────────


@wasm_test
def test_spawn_and_kill():
    """Spawn an agent then kill it."""
    orch = Orchestrator()
    orch.spawn("a", AGENT_P0_PATH)
    assert orch.agent_count() == 1
    assert orch.agent_status("a") == "Running"

    orch.kill("a")
    assert orch.agent_count() == 0


@wasm_test
def test_spawn_duplicate_raises():
    """Spawning an agent with the same id raises KeyError."""
    orch = Orchestrator()
    orch.spawn("x", AGENT_P0_PATH)
    with pytest.raises(KeyError):
        orch.spawn("x", AGENT_P0_PATH)


def test_kill_nonexistent_raises():
    """Killing a non-existent agent raises KeyError."""
    orch = Orchestrator()
    with pytest.raises(KeyError):
        orch.kill("ghost")


# ── Tick ─────────────────────────────────────────────────────────────


def test_tick_all_empty():
    """tick_all on an empty orchestrator returns a zeroed summary."""
    orch = Orchestrator()
    summary = orch.tick_all()
    assert isinstance(summary, RoundSummary)
    assert summary.messages_routed == 0
    assert summary.messages_dropped == 0
    assert summary.round_fuel == 0
    assert summary.crashed == []


def test_tick_agent_not_found():
    """tick_agent raises KeyError for a non-existent agent."""
    orch = Orchestrator()
    with pytest.raises(KeyError):
        orch.tick_agent("ghost")


@wasm_test
def test_tick_agent_basic():
    """Tick a single agent that exports _cage_tick."""
    orch = Orchestrator()
    orch.spawn("a", AGENT_P0_PATH)
    orch.tick_agent("a")
    assert orch.agent_status("a") in ("Running", "Crashed")


@wasm_test
def test_tick_all_with_agent():
    """Tick all with one agent returns a valid summary."""
    orch = Orchestrator()
    orch.spawn("a", AGENT_P0_PATH)
    summary = orch.tick_all()
    assert summary.messages_routed >= 0
    assert summary.messages_dropped >= 0


# ── Pause / Resume ───────────────────────────────────────────────────


@wasm_test
def test_pause_resume():
    """Pause and resume an agent."""
    orch = Orchestrator()
    orch.spawn("a", AGENT_P0_PATH)
    orch.pause("a")
    assert orch.agent_status("a") == "Paused"
    orch.resume("a")
    assert orch.agent_status("a") == "Running"


def test_pause_nonexistent_raises():
    """Pausing a non-existent agent raises KeyError."""
    orch = Orchestrator()
    with pytest.raises(KeyError):
        orch.pause("ghost")


@wasm_test
def test_resume_not_paused_raises():
    """Resuming a Running agent raises RuntimeError."""
    orch = Orchestrator()
    orch.spawn("a", AGENT_P0_PATH)
    with pytest.raises(RuntimeError):
        orch.resume("a")


# ── List agents ──────────────────────────────────────────────────────


@wasm_p1_test
def test_list_agents():
    """list_agents returns (id, status) tuples."""
    orch = Orchestrator()
    orch.spawn("leader", AGENT_P1_PATH)
    orch.spawn("worker", AGENT_P1_PATH)
    agents = orch.list_agents()
    assert len(agents) == 2
    pairs = dict(agents)
    assert pairs["leader"] == "Running"
    assert pairs["worker"] == "Running"


# ── Leader-worker integration (needs agent-p1 built) ────────────────


@wasm_p1_test
def test_leader_worker_rounds():
    """Drive a 2-agent × 3-round leader-worker scenario."""
    orch = Orchestrator()
    orch.spawn("leader", AGENT_P1_PATH)
    orch.spawn("worker-a", AGENT_P1_PATH)
    orch.spawn("worker-b", AGENT_P1_PATH)

    for r in range(3):
        summary = orch.tick_all()
        print(
            f"Round {r}: routed={summary.messages_routed}, "
            f"dropped={summary.messages_dropped}"
        )

    agents = dict(orch.list_agents())
    print(f"Final status: {agents}")

    # All agents should still be running after 3 rounds
    for status in agents.values():
        assert status == "Running"


# ── Repr ─────────────────────────────────────────────────────────────


def test_repr():
    """__repr__ returns a descriptive string."""
    orch = Orchestrator()
    r = repr(orch)
    assert "Orchestrator" in r
    assert "agents=0" in r


def test_repr_with_agents():
    """__repr__ includes agent count after spawn."""
    if not HAS_AGENT_P0:
        pytest.skip("WASM agent not built")
    orch = Orchestrator()
    orch.spawn("a", AGENT_P0_PATH)
    r = repr(orch)
    assert "agents=1" in r
