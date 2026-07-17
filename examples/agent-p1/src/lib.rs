// Agent P1 — leader-worker task distribution via cage_peer_send
//
// Build:  cargo build -p agent-p1 --target wasm32-wasip1 --release
// Test:   cage orchestrate \
//           --agent leader=target/wasm32-wasip1/release/agent_p1.wasm \
//           --agent worker-a=target/wasm32-wasip1/release/agent_p1.wasm \
//           --agent worker-b=target/wasm32-wasip1/release/agent_p1.wasm \
//           --rounds 10 \
//           --message '{"role":"leader","tasks":[{"data":"task1"},{"data":"task2"},{"data":"task3"}]}' \
//           -v
//
// The init payload determines role. Workers receive configuration through env vars
// or can be spawned with per-agent messages (future).

use serde_json::{json, Value};

// ── cage host ABI ──────────────────────────────────────────────────

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
    // Phase 2 — inter-agent messaging
    #[link_name = "peer_send"]
    fn cage_peer_send(
        target_ptr: *const u8,
        target_len: i32,
        msg_ptr: *const u8,
        msg_len: i32,
    ) -> i32;
    #[link_name = "inbox_pending"]
    fn cage_inbox_pending() -> i32;
    #[link_name = "inbox_read"]
    fn cage_inbox_read(buf_ptr: *mut u8, buf_max: i32) -> i32;
}

// ── Static memory ─────────────────────────────────────────────────

const HEAP_SIZE: usize = 65536;
const INBOX_BUF_SIZE: usize = 4096;

static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];
static mut HEAP_OFFSET: usize = 0;
/// Reusable buffer for inbox reads (avoids per-tick bump allocation).
static mut INBOX_BUF: [u8; INBOX_BUF_SIZE] = [0; INBOX_BUF_SIZE];

// ── Agent state ────────────────────────────────────────────────────

struct Task {
    id: String,
    assigned_to: Option<String>,
}

struct Result_ {
    task_id: String,
    worker_id: String,
    output: Value,
    timestamp: i64,
}

struct AgentState {
    role: String,
    worker_id: String,
    tasks: Vec<Task>,
    results: Vec<Result_>,
    pending_count: usize,
    completed_count: usize,
    done: bool,
}

static mut STATE: Option<AgentState> = None;

// ── Exports ────────────────────────────────────────────────────────

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
    // bump allocator: no-op
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _cage_init(ptr: i32, len: i32) -> i32 {
    let payload: Value = if ptr != 0 && len > 0 {
        let slice = core::slice::from_raw_parts(ptr as *const u8, len as usize);
        serde_json::from_slice(slice).unwrap_or(Value::Null)
    } else {
        Value::Null
    };

    let role = payload
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("worker")
        .to_string();
    let worker_id = payload
        .get("worker_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let mut tasks = Vec::new();
    if role == "leader" {
        if let Some(task_list) = payload.get("tasks").and_then(|v| v.as_array()) {
            for (i, _) in task_list.iter().enumerate() {
                tasks.push(Task {
                    id: format!("task-{i}"),
                    assigned_to: None,
                });
            }
        }
        log(&format!("leader: loaded {} tasks", tasks.len()));
    }

    STATE = Some(AgentState {
        role,
        worker_id,
        tasks,
        results: Vec::new(),
        pending_count: 0,
        completed_count: 0,
        done: false,
    });

    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _cage_tick() -> i32 {
    let state = match unsafe { STATE.as_mut() } {
        Some(s) => s,
        None => return -1,
    };

    if state.done {
        return 1;
    }

    match state.role.as_str() {
        "leader" => leader_tick(state),
        "worker" => worker_tick(state),
        _ => {
            log("unknown role");
            -1
        }
    }
}

// ── Leader tick ────────────────────────────────────────────────────
//
// Phase 1: Read inbox for completed results from workers
// Phase 2: Distribute unassigned tasks to workers via cage_peer_send
// Phase 3: When all tasks complete, send summary via cage_send

fn leader_tick(state: &mut AgentState) -> i32 {
    // Phase 1: Drain inbox — collect results from workers
    //
    // NOTE: The orchestrator delivers the full AgentMessage JSON into inbox memory.
    // For a cage_peer_send call, the structure is:
    //   {"kind":"peer","payload":{"from":...,"to":...,"payload":<inner_msg>}}
    // So we must navigate msg["payload"]["payload"] to find the inner content.
    //
    // We use a static INBOX_BUF (not _cage_alloc) to avoid exhausting the bump heap.
    loop {
        let pending = unsafe { cage_inbox_pending() };
        if pending == 0 {
            break;
        }
        let buf_ptr = core::ptr::addr_of_mut!(INBOX_BUF).cast::<u8>();
        let n = unsafe { cage_inbox_read(buf_ptr, INBOX_BUF_SIZE as i32) };
        if n <= 0 {
            break;
        }
        let slice = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, n as usize) };
        if let Ok(msg) = serde_json::from_slice::<Value>(slice) {
            // Navigate to inner payload: msg.payload.payload
            let inner = &msg["payload"]["payload"];
            if inner.is_null() {
                continue;
            }
            if let (Some(task_id), Some(worker_id)) = (
                inner.get("task_id").and_then(|v| v.as_str()),
                inner.get("worker_id").and_then(|v| v.as_str()),
            ) {
                let output = inner.get("result").cloned().unwrap_or(Value::Null);
                state.results.push(Result_ {
                    task_id: task_id.to_string(),
                    worker_id: worker_id.to_string(),
                    output,
                    timestamp: unsafe { cage_time_now() },
                });
                state.completed_count += 1;
                state.pending_count = state.pending_count.saturating_sub(1);
                log(&format!("leader: got result {} from {}", task_id, worker_id));
            }
        }
    }

    // Phase 2: Distribute unassigned tasks to workers
    let workers = ["worker-a", "worker-b"];
    let mut worker_idx = state.pending_count; // simple round-robin

    for task in &mut state.tasks {
        if task.assigned_to.is_some() {
            continue;
        }
        if state.pending_count >= workers.len() {
            break; // all workers busy
        }

        let target = workers[worker_idx % workers.len()];
        worker_idx += 1;
        task.assigned_to = Some(target.to_string());
        state.pending_count += 1;

        let msg = json!({
            "task_id": task.id,
            "payload": "process me",
        });

        let target_bytes = target.as_bytes();
        let msg_bytes = msg.to_string().into_bytes();

        let target_ptr = unsafe { _cage_alloc(target_bytes.len() as i32) };
        if target_ptr.is_null() {
            log("leader: alloc failed for target");
            continue;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                target_bytes.as_ptr(),
                target_ptr,
                target_bytes.len(),
            );
        }

        let msg_ptr = unsafe { _cage_alloc(msg_bytes.len() as i32) };
        if msg_ptr.is_null() {
            log("leader: alloc failed for msg");
            continue;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                msg_bytes.as_ptr(),
                msg_ptr,
                msg_bytes.len(),
            );
        }

        let ret = unsafe {
            cage_peer_send(
                target_ptr,
                target_bytes.len() as i32,
                msg_ptr,
                msg_bytes.len() as i32,
            )
        };
        log(&format!("leader: sent {} to {} (ret={})", task.id, target, ret));
    }

    // Phase 3: Check completion
    if state.pending_count == 0
        && !state.tasks.is_empty()
        && state.completed_count == state.tasks.len()
    {
        state.done = true;
        let summary = json!({
            "kind": "result",
            "payload": {
                "type": "completion",
                "total_tasks": state.tasks.len(),
                "results": state.results.iter().map(|r| {
                    json!({ "task_id": r.task_id, "worker_id": r.worker_id })
                }).collect::<Vec<Value>>()
            }
        });

        let summary_bytes = summary.to_string().into_bytes();
        let ptr = unsafe { _cage_alloc(summary_bytes.len() as i32) };
        if !ptr.is_null() {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    summary_bytes.as_ptr(),
                    ptr,
                    summary_bytes.len(),
                );
            }
            unsafe {
                cage_send(ptr, summary_bytes.len() as i32);
            }
        }
        log(&format!("leader: all {} tasks complete!", state.completed_count));
        return 1;
    }

    0
}

// ── Worker tick ────────────────────────────────────────────────────
//
// Check inbox for tasks from leader, process, send result back.

fn worker_tick(state: &mut AgentState) -> i32 {
    // Use static INBOX_BUF (not _cage_alloc) to avoid exhausting bump heap.
    loop {
        let pending = unsafe { cage_inbox_pending() };
        if pending == 0 {
            break;
        }
        let buf_ptr = core::ptr::addr_of_mut!(INBOX_BUF).cast::<u8>();
        let n = unsafe { cage_inbox_read(buf_ptr, INBOX_BUF_SIZE as i32) };
        if n <= 0 {
            break;
        }
        let slice = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, n as usize) };
        if let Ok(msg) = serde_json::from_slice::<Value>(slice) {
            // Navigate to inner payload: msg.payload.payload
            let inner = &msg["payload"]["payload"];
            if inner.is_null() {
                continue;
            }
            if let Some(task_id) = inner.get("task_id").and_then(|v| v.as_str()) {
                log(&format!(
                    "worker {}: received task {}",
                    state.worker_id, task_id
                ));

                // Simulate processing
                let timestamp = unsafe { cage_time_now() };
                let random_val = unsafe { cage_random() };
                let output = json!({
                    "processed": true,
                    "timestamp": timestamp,
                    "random": random_val,
                });

                // Send result back to leader
                let result = json!({
                    "task_id": task_id,
                    "worker_id": state.worker_id,
                    "result": output,
                });

                let target = "leader";
                let target_bytes = target.as_bytes();
                let msg_bytes = result.to_string().into_bytes();

                let target_ptr = unsafe { _cage_alloc(target_bytes.len() as i32) };
                if target_ptr.is_null() {
                    continue;
                }
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        target_bytes.as_ptr(),
                        target_ptr,
                        target_bytes.len(),
                    );
                }

                let msg_ptr = unsafe { _cage_alloc(msg_bytes.len() as i32) };
                if msg_ptr.is_null() {
                    continue;
                }
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        msg_bytes.as_ptr(),
                        msg_ptr,
                        msg_bytes.len(),
                    );
                }

                let ret = unsafe {
                    cage_peer_send(
                        target_ptr,
                        target_bytes.len() as i32,
                        msg_ptr,
                        msg_bytes.len() as i32,
                    )
                };
                log(&format!(
                    "worker {}: sent result for {} (ret={})",
                    state.worker_id, task_id, ret
                ));
            }
        }
    }

    0
}

// ── Helpers ────────────────────────────────────────────────────────

fn log(msg: &str) {
    unsafe {
        cage_log(msg.as_ptr(), msg.len() as i32);
    }
}
