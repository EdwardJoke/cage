# Host Functions

The 9 host functions available to WASM agents via the `"cage"` import module.

## Required Imports

```rust
#[link(wasm_import_module = "cage")]
unsafe extern "C" {
    fn cage_log(ptr: *const u8, len: i32) -> i32;
    fn cage_send(ptr: *const u8, len: i32) -> i32;
    fn cage_time_now() -> i64;
    fn cage_random() -> i32;
    fn cage_env_get(key_ptr: *const u8, key_len: i32) -> *mut u8;
    fn cage_http_request(req_ptr: *const u8, req_len: i32,
                         resp_ptr: *mut u8, resp_max: i32) -> i32;
    fn cage_peer_send(target_ptr: *const u8, target_len: i32,
                      msg_ptr: *const u8, msg_len: i32) -> i32;
    fn cage_inbox_pending() -> i32;
    fn cage_inbox_read(buf_ptr: *mut u8, buf_max: i32) -> i32;
}
```

## Function Reference

| Function | Returns | Description |
|----------|---------|-------------|
| `log` | `0` / `-1` | Emit a log message from agent memory |
| `send` | `0` / `-1` | Return structured JSON result to host |
| `time_now` | `i64` | Nanoseconds since Unix epoch |
| `random` | `i32` | Cryptographically random 32-bit integer |
| `env_get` | ptr / `0` / `-1` | Read injected env var by key (returns pointer in agent memory, `0` if missing) |
| `http_request` | length / `-1` | HTTP request to whitelisted URL. Returns bytes written, or negative for truncated response |
| `peer_send` | `0` / negative | Queue message to another agent's outbox |
| `inbox_pending` | `1` / `0` | Check if inbox message is available |
| `inbox_read` | length / `0` / negative | Read inbox message into agent buffer |

## HTTP Request Format

Send JSON with `method`, `url`, optional `headers` and `body`:

```json
{"method":"GET","url":"https://api.example.com/data"}
```

Response is JSON: `{"status":200,"headers":{},"body":"..."}`.

URLs must match a prefix in the agent's whitelist (set via `--allow-url` or `Sandbox::allow_url()`).
