# Message Routing

Cage supports 4 pluggable routing topologies via the `MessageRouter` trait. Configured with `--topology` on the CLI or `Orchestrator(topology="...")` in Python.

## Topologies

| Topology | `--topology` | Behavior |
|----------|-------------|----------|
| **Direct** | `direct` (default) | Routes to exact agent ID from `payload["to"]`. Backward compatible with Phase 2. |
| **Broadcast** | `broadcast` | Delivers to every agent except the sender. Ignores `payload["to"]`. |
| **Pattern** | `pattern` | Matches agent IDs by glob/wildcard (e.g. `"worker-*"` matches `worker-a`, `worker-b`). |
| **Hub-and-Spoke** | `hub-and-spoke` | All non-hub traffic routes through a designated hub agent. Configure with `--hub <id>`. |

## Dead Letter Queue

When `--dlq` is enabled, unroutable messages are stored in a DLQ instead of dropped.

```bash
cargo run --release -- orchestrate \
  --agent 'leader=leader.wasm' --agent 'worker=worker.wasm' \
  --rounds 3 --topology direct --dlq -v
```

Inspect the DLQ via `orch.dlq()` or `orch.drain_dlq()` (Python/Rust).

## CLI Examples

```bash
# Broadcast — leader sends to all workers at once
--topology broadcast

# Pattern — only matches "worker-*" agents
--agent 'leader=lead.wasm' --agent 'worker-a=w.wasm' \
  --agent 'worker-b=w.wasm' --agent 'other=o.wasm' \
  --topology pattern

# Hub-and-Spoke — all traffic through "hub"
--agent 'hub=hub.wasm' --agent 'worker-a=w.wasm' \
  --topology hub-and-spoke --hub hub
```

See [docs/cli.md](cli.md) for full flag reference.
