# Agent Development

Writing a Cage agent in Rust for the `wasm32-wasip1` target.

## Required Exports & Imports

Every agent must export `memory`, `_cage_alloc`, `_cage_free`, `_cage_init`, and `_cage_tick`:

| Export | Signature | Description |
|--------|-----------|-------------|
| `memory` | linear memory | At least 64KB |
| `_cage_alloc` | `(size: i32) -> *mut u8` | Bump-allocate on agent heap |
| `_cage_free` | `(ptr: *mut u8)` | No-op for bump allocator |
| `_cage_init` | `(ptr: i32, len: i32) -> i32` | Called on spawn with optional init payload |
| `_cage_tick` | `() -> i32` | Called each tick round |

Import host functions under `#[link(wasm_import_module = "cage")]` — see [docs/host-functions.md](host-functions.md) for all 9.

## Cargo Configuration & Build

```toml
[package]
name = "my-agent"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
serde_json = "1"
```

Build: `cargo build --target wasm32-wasip1 --release`

## Environment Variables & Memory

Inject secrets at spawn time (not hardcoded): `cage_env_get("API_KEY")`. Set via CLI `--env API_KEY=sk-...` or `Sandbox::set_env()`.

Example agents use a bump allocator. For production, use `dlmalloc` or `wee_alloc`. See `examples/agent-p0/` and `examples/agent-p1/` for complete working agents.
