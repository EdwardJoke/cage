# Quick Start

Step-by-step guide to your first multi-agent run.

## Build & Run a Single Agent

```bash
cargo build --release
cargo build -p agent-p1 --target wasm32-wasip1 --release

cargo run --release -- run target/wasm32-wasip1/release/agent_p0.wasm \
  --message '{"hello":"world"}' --env FOO=bar \
  --allow-url https://httpbin.org --fuel 500000
```

Agent-p0 exercises all 9 host functions and logs the results.

## Multi-Agent Orchestration

```bash
cargo run --release -- orchestrate \
  --agent 'leader=target/wasm32-wasip1/release/agent_p1.wasm' \
  --agent 'worker-a=target/wasm32-wasip1/release/agent_p1.wasm' \
  --agent 'worker-b=target/wasm32-wasip1/release/agent_p1.wasm' \
  --message '{"role":"leader","tasks":[{"data":"t1"},{"data":"t2"},{"data":"t3"}]}' \
  --rounds 6 --fuel 3000000 -v
```

Leader distributes tasks to workers, collects results, and completes within 6 rounds. Try different [routing topologies](routing.md) with `--topology broadcast`.

## Python Bindings

```bash
cd cage-py && maturin develop --release && python
```

```python
from cage import Orchestrator
orch = Orchestrator()
orch.spawn("leader", "target/wasm32-wasip1/release/agent_p1.wasm")
summary = orch.tick_all()
print(f"Routed: {summary.messages_routed}")
```

See [docs/python.md](python.md) for the full API.
