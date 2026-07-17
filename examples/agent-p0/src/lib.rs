// Agent P0 — exercises every cage host interface in one pass.
//
// Build:  cargo build -p agent-p0 --target wasm32-wasip1 --release
// Run:    cargo run --release -- run <wasm> --message '{"hello":"world"}'
//           --env FOO=bar --allow-url https://httpbin.org --fuel 500000 -v

use serde_json::{json, Value};

// ── cage host ABI ─────────────────────────────────────────────────────

#[link(wasm_import_module = "cage")]
unsafe extern "C" {
    #[link_name = "log"]
    fn cage_log(ptr: *const u8, len: i32) -> i32;
    #[link_name = "send"]
    fn cage_send(ptr: *const u8, len: i32) -> i32;
    #[link_name = "time_now"]
    fn cage_time_now() -> i64;
    #[link_name = "random"]
    fn cage_random() -> i32;
    #[link_name = "env_get"]
    fn cage_env_get(key_ptr: *const u8, key_len: i32) -> *mut u8;
    #[link_name = "http_request"]
    fn cage_http_request(
        req_ptr: *const u8,
        req_len: i32,
        resp_ptr: *mut u8,
        resp_max: i32,
    ) -> i32;
}

// ── Static regions for cross-boundary memory ─────────────────────────

const HEAP_SIZE: usize = 49152;  // bump-allocator heap (probe allocates dynamically)

static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];
static mut HEAP_OFFSET: usize = 0;

// ── Agent exports (memory + alloc/free + lifecycle) ──────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _cage_alloc(size: i32) -> *mut u8 {
    let size = size.max(0) as usize;
    let offset = core::ptr::addr_of!(HEAP_OFFSET).read();
    let end = offset + size;
    if end > HEAP_SIZE {
        return core::ptr::null_mut();
    }
    core::ptr::addr_of_mut!(HEAP_OFFSET).write(end);
    core::ptr::addr_of_mut!(HEAP).cast::<u8>().add(offset)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _cage_free(_ptr: *mut u8) {
    // bump allocator: free is a no-op
}

// ── Host-call wrappers ───────────────────────────────────────────────

fn log(msg: &str) {
    unsafe { cage_log(msg.as_ptr(), msg.len() as i32); }
}

fn send(value: &Value) {
    let json_str = serde_json::to_string(value).expect("send serialization");
    unsafe { cage_send(json_str.as_ptr(), json_str.len() as i32); }
}

fn env_get(key: &str) -> Option<String> {
    let ptr = unsafe { cage_env_get(key.as_ptr(), key.len() as i32) };
    if ptr.is_null() {
        return None;
    }
    let mut len = 0usize;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        Some(
            String::from_utf8_lossy(core::slice::from_raw_parts(ptr, len)).to_string(),
        )
    }
}

// ── _cage_init ────────────────────────────────────────────────────────
//
// Validates: log, send, time_now, random, env_get (found + missing paths)

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _cage_init(ptr: i32, len: i32) -> i32 {
    log("_cage_init: entering");

    // 1. Parse the init payload
    let payload: Option<Value> = if ptr != 0 && len > 0 {
        let slice = core::slice::from_raw_parts(ptr as *const u8, len as usize);
        Some(serde_json::from_slice(slice).unwrap_or(Value::Null))
    } else {
        None
    };

    // 2. System time
    let time_ns = cage_time_now();

    // 3. Random 32-bit integer
    let random = cage_random();

    // 4. Env var lookups — report found + missing separately
    let env_keys = ["HOME", "USER", "PATH", "SHELL"];
    let mut env_found = serde_json::Map::new();
    let mut env_missing = Vec::new();
    for key in &env_keys {
        match env_get(key) {
            Some(v) => {
                env_found.insert(key.to_string(), Value::String(v));
            }
            None => {
                env_missing.push(Value::String(key.to_string()));
            }
        }
    }

    // 5. Send results as an AgentMessage
    let mut payload_map = serde_json::Map::new();
    if let Some(p) = payload {
        payload_map.insert("init_payload".to_string(), p);
    }
    payload_map.insert("time_ns".to_string(), json!(time_ns));
    payload_map.insert("random".to_string(), json!(random));
    payload_map.insert("env_found".to_string(), Value::Object(env_found));
    payload_map.insert("env_missing".to_string(), Value::Array(env_missing));

    send(&json!({"kind": "init_complete", "payload": Value::Object(payload_map)}));

    log("_cage_init: complete");
    0
}

// ── _cage_tick ────────────────────────────────────────────────────────
//
// Exercises: log, send, time_now, random, http_request (probe + allocate + retry)
//
// The probe pattern (resp_max=0) discovers the required buffer size, then
// allocates from the bump heap and re-issues the request. This validates the
// full contract of cage_http_request: sizing probe → allocation → fulfillment.

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _cage_tick() -> i32 {
    log("_cage_tick: entering");

    let time_ns = cage_time_now();
    let random = cage_random();

    // HTTP GET to httpbin.org via the probe pattern
    let req_json = r#"{"method":"GET","url":"https://httpbin.org/get"}"#;

    // ── Phase 1: probe — discover how many bytes the response needs ──
    // resp_max=0 tells the host to only report the required size (negated).
    let probe = cage_http_request(
        req_json.as_ptr(),
        req_json.len() as i32,
        core::ptr::null_mut(),
        0,
    );

    let mut payload_map = serde_json::Map::new();
    payload_map.insert("time_ns".to_string(), json!(time_ns));
    payload_map.insert("random".to_string(), json!(random));

    if probe < 0 {
        // Probe succeeded: -probe is the total bytes required
        let total = (-probe) as usize;
        payload_map.insert("http_probe_size".to_string(), json!(total));

        // ── Phase 2: allocate a buffer of exactly the needed size ──
        let buf = _cage_alloc(total as i32);
        if buf.is_null() {
            payload_map.insert("http_status".to_string(), json!("alloc_failed"));
            send(&json!({"kind": "tick_complete", "payload": Value::Object(payload_map)}));
            log("_cage_tick: alloc_failed — bump heap exhausted");
            return 0;
        }

        // ── Phase 3: re-issue the request into the sized buffer ──
        let n = cage_http_request(
            req_json.as_ptr(),
            req_json.len() as i32,
            buf,
            total as i32,
        );

        if n > 0 {
            let slice = core::slice::from_raw_parts(buf, n as usize);
            let resp_str = String::from_utf8_lossy(slice).to_string();
            payload_map.insert("http_status".to_string(), json!("ok"));
            payload_map.insert("http_bytes".to_string(), json!(n));
            payload_map.insert("http_response".to_string(), json!(resp_str));
        } else {
            payload_map.insert("http_status".to_string(), json!("retry_failed"));
            payload_map.insert("http_retry".to_string(), json!(n));
        }
    } else if probe == 0 {
        payload_map.insert("http_status".to_string(), json!("empty"));
    } else {
        // probe > 0 with resp_max=0 should not happen, but defend against it
        payload_map.insert("http_status".to_string(), json!("unexpected"));
        payload_map.insert("http_bytes".to_string(), json!(probe));
    }

    send(&json!({"kind": "tick_complete", "payload": Value::Object(payload_map)}));

    log("_cage_tick: complete");
    0
}
