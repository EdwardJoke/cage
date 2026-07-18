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
| `save(path)` | `None` | Save checkpoint to JSON file |
| `save_full(path)` | `None` | Save checkpoint with WASM memory snapshots |
| `load(path)` (static) | `Orchestrator` | Load orchestrator from checkpoint |
| `export_summary()` | `str` | Human-readable JSON summary (no binary memory) |
| `set_save_every(n)` | `None` | Auto-save checkpoint every N rounds |
| `set_checkpoint_dir(dir)` | `None` | Directory for auto-save checkpoints |

## RoundSummary & Errors

**Fields:** `messages_routed`, `messages_dropped`, `messages_dlq`, `dlq_depth`, `routing_topology`, `round_fuel`, `crashed`, `agent_inbox_depths`

**Errors:** `KeyError` (agent not found), `RuntimeError` (WASM load/tick failure)

## Checkpoint & Resume Example

```python
from cage import Orchestrator

orch = Orchestrator()
orch.spawn("leader", "agent_p1.wasm")
orch.spawn("worker-a", "agent_p1.wasm")

for _ in range(3):
    summary = orch.tick_all()

# Save checkpoint (no memory snapshots — smaller file)
orch.save("checkpoint.json")

# Save with full WASM memory (slower, larger, exact reconstruction)
orch.save_full("checkpoint-full.json")

# Later: restore and continue
restored = Orchestrator.load("checkpoint.json")
summary = restored.tick_all()
print(f"Resumed: routed {summary.messages_routed}")
```
