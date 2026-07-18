# Python Bindings

Python API for driving multi-agent orchestration via PyO3.

## Installation & Usage

Install from PyPI (recommended):

```bash
pip install coplex-cage
```

Or build from source:

```bash
cd cage-py && maturin develop --release
```

```python
from cage import Orchestrator

# Create orchestrator (default: direct topology)
orch = Orchestrator()
# orch = Orchestrator(topology="broadcast")
# orch = Orchestrator(topology="pattern")
# orch = Orchestrator(topology="hub-and-spoke", hub="hub")
# orch = Orchestrator(dlq_enabled=True)

orch.spawn("leader", "agent_p1.wasm")
orch.spawn("worker-a", "agent_p1.wasm")

summary = orch.tick_all()
print(f"Routed: {summary.messages_routed}, Topology: {summary.routing_topology}")
```

> **Note:** The Python package imports as `cage` (not `coplex_cage`). Use `from cage import Orchestrator`.

## API Reference

| Method | Returns | Description |
|--------|---------|-------------|
| `Orchestrator(...)` | `Orchestrator` | Optional: `topology`, `hub`, `dlq_enabled` |
| `spawn(id, wasm_path)` | `None` | Load and register a WASM agent |
| `tick_agent(id)` | `None` | Tick a single agent |
| `tick_all()` | `RoundSummary` | Tick all agents (releases GIL) |
| `kill(id)` | `None` | Remove an agent |
| `pause(id)` / `resume(id)` | `None` | Pause or resume an agent |
| `agent_count()` | `int` | Number of registered agents |
| `agent_status(id)` | `str` | Running, Paused, Crashed, Terminated |
| `list_agents()` | `list[tuple]` | All agents with status |

## RoundSummary & Errors

**Fields:** `messages_routed`, `messages_dropped`, `messages_dlq`, `dlq_depth`, `routing_topology`, `round_fuel`, `crashed`, `agent_inbox_depths`, `observed_messages`

**Errors:** `KeyError` (agent not found), `RuntimeError` (WASM load/tick failure)
