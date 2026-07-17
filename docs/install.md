# Installation Guide

Prerequisites, build commands, and common setup errors.

## Prerequisites & Build

Install the [Rust toolchain](https://rustup.rs/) (edition 2024, Rust 1.85+):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-wasip1
pip install maturin pytest   # optional: Python bindings
```

Build the host crate and example agents:

```bash
cargo build --release
cargo build -p agent-p0 --target wasm32-wasip1 --release
cargo build -p agent-p1 --target wasm32-wasip1 --release
cd cage-py && maturin develop --release   # optional
```

## Verify

```bash
cargo run --release -- --help
```

Expected output shows `run` and `orchestrate` subcommands. See [docs/quickstart.md](quickstart.md) for your first run.

## Troubleshooting

| Error | Fix |
|-------|-----|
| `unknown import 'cage::...'` | Agent must use `#[link(wasm_import_module = "cage")]` |
| `_cage_alloc returned negative pointer` | Increase `HEAP_SIZE` in the agent |
| `out of fuel` | Increase `--fuel`. Agent-p1 typically needs 1–3M per round |
| `ModuleNotFoundError: No module named 'cage'` | Run `maturin develop --release` from `cage-py/` |
