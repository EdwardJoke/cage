# Architecture

The tick cycle, message flow, and core components of the Cage runtime.

## Tick Cycle

Each round follows a three-phase pipeline:

```
┌──────────────┐   ┌──────────────┐   ┌──────────────┐
│  Deliver     │   │  Execute     │   │  Drain       │
│  Inbox       │──►│  _cage_tick  │──►│  Outbox      │
│              │   │              │   │  + Route     │
└──────────────┘   └──────────────┘   └──────────────┘
```

1. **Deliver Inbox** — Pop a pending message, serialize to JSON, write into agent linear memory via `_cage_alloc`
2. **Execute Tick** — Call `_cage_tick()`. Agent reads inbox, processes messages, queues outbounds via `cage_peer_send`
3. **Drain Outbox** — Collect outbound messages, resolve targets via the active [MessageRouter](routing.md), push to target inboxes

After draining, the orchestrator optionally saves a **checkpoint** (every N rounds, or on demand). Checkpoints serialize agent state (fuel, inbox/outbox, env, status) to JSON. The `--full-snapshot` flag also includes WASM linear memory for exact byte-level reconstruction.

## Components

- **Sandbox** — single-agent Wasmtime wrapper with fuel metering and 9 [host functions](host-functions.md)
- **Orchestrator** — manages multiple agents, drives the tick cycle, routes messages, supports checkpoint/restore
- **Router** — pluggable message routing (Direct, Broadcast, Pattern, Hub-and-Spoke)
- **Snapshot** — serializes orchestrator state (agents, round count, cumulative stats, DLQ) to JSON; `load` reconstructs the orchestrator and re-instantiates all WASM modules
- **DLQ** — Dead Letter Queue for unroutable messages

## Agent Lifecycle & Fuel

```
Spawn → Running → Paused → Running
          │
          └── → Crashed (trap, fuel exhaustion)
Kill → Terminated (removed)
```

Agents run under Wasmtime's instruction-level fuel metering. Each instruction consumes fuel. When exhausted, the agent traps. Ensures deterministic execution budgets and fair scheduling across agents.
