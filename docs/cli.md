# CLI Reference

The `cage` binary provides two subcommands: `run` for single agents and `orchestrate` for multi-agent scenarios.

## `cage run`

Run a single WASM agent.

```text
Usage: cage run [OPTIONS] <AGENT>

Arguments:
  <AGENT>              Path to WASM module

Options:
  -m, --message <JSON>    JSON init payload
  -e, --env <KEY=VALUE>   Environment variables (repeatable)
  --allow-url <PREFIX>    Allowed URL prefixes (repeatable)
  -f, --fuel <NUM>        Maximum fuel [default: 200000]
  -v, --verbose           Enable debug logging
  -h, --help              Print help
```

## `cage orchestrate`

Run multi-agent orchestration.

```text
Usage: cage orchestrate [OPTIONS] --agent <ID=PATH>

Options:
  -a, --agent <ID=PATH>   Agent spec (repeatable, required)
  -m, --message <JSON>    JSON init payload (sent to first agent only)
  -e, --env <KEY=VALUE>   Environment variables (all agents)
  --allow-url <PREFIX>    Allowed URL prefixes (all agents)
  -f, --fuel <NUM>        Fuel per agent [default: 500000]
  -r, --rounds <NUM>      Tick rounds [default: 1]
  -v, --verbose           Enable debug logging
  --topology <NAME>       Routing topology [default: direct]
  --hub <ID>              Hub agent ID (required for hub-and-spoke)
  --dlq                   Enable Dead Letter Queue
  -h, --help              Print help
```


## Examples

```bash
# Single agent with HTTP and env
cage run agent.wasm -m '{"task":"go"}' -e API_KEY=abc \
  --allow-url https://api.example.com --fuel 500000 -v

# Broadcast orchestration with DLQ
cage orchestrate -a 'lead=lead.wasm' -a 'w1=w.wasm' \
  --message '{"go":true}' --rounds 5 --fuel 1000000 \
  --topology broadcast --dlq -v

# Hub-and-spoke
cage orchestrate -a 'hub=h.wasm' -a 'w1=w.wasm' \
  --rounds 3 --topology hub-and-spoke --hub hub -v
```

See [docs/routing.md](routing.md) for topology-specific guidance.
