# Installation Guide

Prerequisites, build commands, and common setup errors.

## Quick Install

The fastest way to get started:

```bash
# Install the cage CLI from crates.io
cargo install coplex-cage

# Install Python bindings from PyPI
pip install coplex-cage
```

The `cage` binary is now available globally. See [docs/quickstart.md](quickstart.md) for your first run.

## Build from Source

Alternatively, clone the repo and build locally:

```bash
git clone <repo-url> && cd cage
cargo build --release
cd cage-py && maturin develop --release   # optional: Python bindings
```

## Prerequisites

- [Rust toolchain](https://rustup.rs/) (edition 2024, Rust 1.85+)
- `wasm32-wasip1` target (for building WASM agents): `rustup target add wasm32-wasip1`
- (Optional) `maturin` and `pytest`: `pip install maturin pytest`
- (Optional) Python 3.8+ for bindings

## Troubleshooting

| Error | Fix |
|-------|-----|
| `unknown import 'cage::...'` | Agent must use `#[link(wasm_import_module = "cage")]` |
| `_cage_alloc returned negative pointer` | Increase `HEAP_SIZE` in the agent |
| `out of fuel` | Increase `--fuel`. Agent-p1 typically needs 1–3M per round |
| `ModuleNotFoundError: No module named 'cage'` | `pip install coplex-cage` or build from source via `maturin develop --release` |
| `command not found: cage` | Run `cargo install coplex-cage` first |
