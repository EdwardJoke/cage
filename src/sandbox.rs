use std::collections::HashMap;
use std::collections::VecDeque;

use anyhow::{anyhow, Result};
use wasmtime::*;
use wasmtime_wasi::p1::{add_to_linker_sync, WasiP1Ctx};
use wasmtime_wasi::WasiCtxBuilder;

use crate::ipc::AgentMessage;

/// Orchestrator-managed state for inter-agent messaging.
#[derive(Clone)]
pub(crate) struct InboxState {
    /// Pointer to serialised inbox message in agent linear memory (set by orchestrator).
    pub ptr: i32,
    /// Length of the serialised inbox message.
    pub len: i32,
}

pub(crate) struct SandboxState {
    pub(crate) wasi: WasiP1Ctx,
    pub(crate) agent_message: Option<AgentMessage>,
    pub(crate) env: HashMap<String, String>,
    pub(crate) allowed_urls: Vec<String>,
    pub(crate) fuel_consumed: u64,
    pub(crate) tick_count: u32,

    // ── Phase 2: inter-agent messaging ──
    /// Agent's own identifier (set by orchestrator on spawn).
    pub(crate) agent_id: String,
    /// Messages queued by this agent via `cage_peer_send` (drained by orchestrator after tick).
    pub(crate) outbox: VecDeque<AgentMessage>,
    /// Pointer/length of a pending inbox message written into agent memory by orchestrator.
    pub(crate) inbox_state: Option<InboxState>,
}

pub struct Sandbox {
    engine: Engine,
    store: Option<Store<SandboxState>>,
    instance: Option<Instance>,
    memory: Option<Memory>,
    env: HashMap<String, String>,
    allowed_urls: Vec<String>,
}

macro_rules! apply_headers {
    ($builder:expr, $headers:expr) => {{
        let mut b = $builder;
        if let Some(obj) = ($headers).as_object() {
            for (k, v) in obj {
                if let Some(val) = v.as_str() {
                    b = b.header(k.as_str(), val);
                }
            }
        }
        b
    }};
}

impl Sandbox {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.wasm_multi_value(true);
        config.wasm_bulk_memory(true);

        let engine = Engine::new(&config)?;

        Ok(Self {
            engine,
            store: None,
            instance: None,
            memory: None,
            env: HashMap::new(),
            allowed_urls: Vec::new(),
        })
    }

    pub fn set_env(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.env.insert(key.into(), value.into());
    }

    pub fn allow_url(&mut self, url: impl Into<String>) {
        self.allowed_urls.push(url.into());
    }

    pub fn load_agent(&mut self, path: &str, fuel: u64) -> Result<()> {
        let module = Module::from_file(&self.engine, path)?;

        let wasi = WasiCtxBuilder::new().inherit_stdio().build_p1();
        let state = SandboxState {
            wasi,
            agent_message: None,
            env: self.env.clone(),
            allowed_urls: self.allowed_urls.clone(),
            fuel_consumed: 0,
            tick_count: 0,
            agent_id: String::new(),
            outbox: VecDeque::new(),
            inbox_state: None,
        };

        let mut store = Store::new(&self.engine, state);
        store.set_fuel(fuel)?;

        let mut linker = Linker::new(&self.engine);
        add_to_linker_sync(&mut linker, |state: &mut SandboxState| &mut state.wasi)?;
        register_host_functions(&mut linker)?;

        let instance = linker.instantiate(&mut store, &module)?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| anyhow!("agent module must export linear memory as \"memory\""))?;

        self.store = Some(store);
        self.instance = Some(instance);
        self.memory = Some(memory);

        log::info!("loaded agent from {path} with {fuel} fuel limit");

        Ok(())
    }

    pub fn init(&mut self, payload: Option<&str>) -> Result<Option<AgentMessage>> {
        let store = self.store.as_mut().unwrap();
        let instance = self.instance.as_ref().unwrap();
        let memory = self.memory.as_ref().unwrap();

        let fuel_before = store.get_fuel()?;

        let (ptr, len) = match payload {
            Some(msg) => {
                let msg_bytes = msg.as_bytes();
                let size = msg_bytes.len() as i32;

                let alloc = instance
                    .get_typed_func::<i32, i32>(&mut *store, "_cage_alloc")?;

                let ptr = alloc.call(&mut *store, size)?;

                if ptr < 0 {
                    anyhow::bail!("_cage_alloc returned negative pointer: {ptr}");
                }

                memory.write(&mut *store, ptr as usize, msg_bytes)?;

                (ptr, size)
            }
            None => (0, 0),
        };

        match instance.get_typed_func::<(i32, i32), i32>(&mut *store, "_cage_init") {
            Ok(init) => {
                let result = init.call(&mut *store, (ptr, len))?;
                if result != 0 {
                    anyhow::bail!("_cage_init returned non-zero: {result}");
                }
                log::info!("agent initialized successfully");
            }
            Err(_) => {
                log::info!("agent does not export `_cage_init`, skipping init call");
            }
        }

        if ptr != 0 {
            if let Ok(free) = instance.get_typed_func::<i32, ()>(&mut *store, "_cage_free") {
                free.call(&mut *store, ptr)?;
            }
        }

        let fuel_after = store.get_fuel()?;
        let consumed = fuel_before.saturating_sub(fuel_after);
        store.data_mut().fuel_consumed += consumed;
        log::info!("agent consumed {consumed} fuel during init");

        Ok(store.data_mut().agent_message.take())
    }

    pub fn stats(&self) -> (u64, u32) {
        let store = self.store.as_ref().unwrap();
        let state = store.data();
        (state.fuel_consumed, state.tick_count)
    }

    pub fn tick(&mut self) -> Result<Option<AgentMessage>> {
        let store = self.store.as_mut().unwrap();
        let instance = self.instance.as_ref().unwrap();

        let fuel_before = store.get_fuel()?;

        match instance.get_typed_func::<(), i32>(&mut *store, "_cage_tick") {
            Ok(tick) => {
                let result = tick.call(&mut *store, ())?;
                if result != 0 {
                    anyhow::bail!("_cage_tick returned non-zero: {result}");
                }
                log::info!("agent tick completed");
            }
            Err(_) => {
                log::info!("agent does not export `_cage_tick`");
            }
        }

        let fuel_after = store.get_fuel()?;
        let consumed = fuel_before.saturating_sub(fuel_after);
        let state = store.data_mut();
        state.fuel_consumed += consumed;
        state.tick_count += 1;
        log::info!("agent consumed {consumed} fuel during tick ({total} total, {count} ticks)",
            total = state.fuel_consumed, count = state.tick_count);

        Ok(state.agent_message.take())
    }
}

pub(crate) fn register_host_functions(linker: &mut Linker<SandboxState>) -> Result<()> {
    linker.func_wrap(
        "cage",
        "log",
        |mut caller: Caller<'_, SandboxState>, ptr: i32, len: i32| -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => {
                    log::error!("agent has no exported memory for cage_log");
                    return -1;
                }
            };

            let mut buf = vec![0u8; len as usize];
            if memory.read(&caller, ptr as usize, &mut buf).is_err() {
                log::error!("cage_log: failed to read agent memory");
                return -1;
            }

            let s = String::from_utf8_lossy(&buf);
            log::info!("[agent] {s}");
            0
        },
    )?;

    linker.func_wrap(
        "cage",
        "send",
        |mut caller: Caller<'_, SandboxState>, ptr: i32, len: i32| -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => {
                    log::error!("agent has no exported memory for cage_send");
                    return -1;
                }
            };

            let mut buf = vec![0u8; len as usize];
            if memory.read(&caller, ptr as usize, &mut buf).is_err() {
                log::error!("cage_send: failed to read agent memory");
                return -1;
            }

            match serde_json::from_slice::<AgentMessage>(&buf) {
                Ok(msg) => {
                    log::info!("[agent message] kind={}", msg.kind);
                    caller.data_mut().agent_message = Some(msg);
                    0
                }
                Err(e) => {
                    log::error!("cage_send: failed to parse AgentMessage: {e}");
                    -1
                }
            }
        },
    )?;

    linker.func_wrap(
        "cage",
        "time_now",
        |_caller: Caller<'_, SandboxState>| -> i64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0)
        },
    )?;

    linker.func_wrap(
        "cage",
        "random",
        |_caller: Caller<'_, SandboxState>| -> i32 {
            let mut bytes = [0u8; 4];
            let _ = getrandom::fill(&mut bytes);
            i32::from_ne_bytes(bytes)
        },
    )?;

    linker.func_wrap(
        "cage",
        "env_get",
        |mut caller: Caller<'_, SandboxState>, key_ptr: i32, key_len: i32| -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => return -1,
            };

            let mut buf = vec![0u8; key_len as usize];
            if memory.read(&caller, key_ptr as usize, &mut buf).is_err() {
                return -1;
            }
            let key = String::from_utf8_lossy(&buf);

            let value = match caller.data().env.get(key.as_ref()) {
                Some(v) => v.clone(),
                None => return 0,
            };

            let alloc_export = match caller.get_export("_cage_alloc") {
                Some(Extern::Func(f)) => f,
                _ => return -1,
            };

            let alloc_size = value.len() + 1;
            let mut ptr_result = [Val::I32(0)];
            if alloc_export
                .call(&mut caller, &[Val::I32(alloc_size as i32)], &mut ptr_result)
                .is_err()
            {
                return -1;
            }

            let out_ptr = match ptr_result[0] {
                Val::I32(p) if p >= 0 => p,
                _ => return -1,
            };

            if memory
                .write(&mut caller, out_ptr as usize, value.as_bytes())
                .is_err()
            {
                return -1;
            }

            let null_byte = [0u8];
            if memory
                .write(&mut caller, (out_ptr + value.len() as i32) as usize, &null_byte)
                .is_err()
            {
                return -1;
            }

            out_ptr
        },
    )?;

    linker.func_wrap(
        "cage",
        "http_request",
        |mut caller: Caller<'_, SandboxState>,
         req_ptr: i32,
         req_len: i32,
         resp_ptr: i32,
         resp_max: i32|
         -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => return -1,
            };

            let mut buf = vec![0u8; req_len as usize];
            if memory.read(&caller, req_ptr as usize, &mut buf).is_err() {
                return -1;
            }

            let req: serde_json::Value = match serde_json::from_slice(&buf) {
                Ok(v) => v,
                Err(e) => {
                    log::error!("cage_http_request: invalid JSON: {e}");
                    return -1;
                }
            };

            let method = req["method"].as_str().unwrap_or("GET");
            let url = match req["url"].as_str() {
                Some(u) => u,
                None => return -1,
            };

            let allowed = is_url_allowed(url, &caller.data().allowed_urls);
            if !allowed {
                log::warn!("cage_http_request: URL not in whitelist: {url}");
                let denied = serde_json::json!({"status": 0, "error": "URL not allowed"});
                return write_response(&memory, &mut caller, resp_ptr, resp_max, &denied);
            }

            execute_http(method, url, &req, &memory, &mut caller, resp_ptr, resp_max)
        },
    )?;

    // ── Phase 2: inter-agent peer messaging ──────────────────────

    linker.func_wrap(
        "cage",
        "peer_send",
        |mut caller: Caller<'_, SandboxState>,
         target_ptr: i32,
         target_len: i32,
         msg_ptr: i32,
         msg_len: i32|
         -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => return -1,
            };

            // 1. Read target agent ID
            let mut target_buf = vec![0u8; target_len as usize];
            if memory.read(&caller, target_ptr as usize, &mut target_buf).is_err() {
                return -1;
            }
            let target_id = String::from_utf8_lossy(&target_buf).to_string();
            if target_id.is_empty() {
                return -1;
            }

            // 2. Read payload JSON
            let mut payload_buf = vec![0u8; msg_len as usize];
            if memory.read(&caller, msg_ptr as usize, &mut payload_buf).is_err() {
                return -2;
            }
            let payload: serde_json::Value = match serde_json::from_slice(&payload_buf) {
                Ok(v) => v,
                Err(_) => return -2,
            };

            // 3. Build PeerMessage
            let from = caller.data().agent_id.clone();
            let peer_msg = AgentMessage {
                kind: "peer".to_string(),
                payload: serde_json::json!({
                    "from": from,
                    "to": target_id,
                    "payload": payload,
                }),
            };

            // 4. Queue to outbox
            caller.data_mut().outbox.push_back(peer_msg);
            log::info!(
                "[peer_send] {} -> {}",
                caller.data().agent_id,
                target_id,
            );
            0
        },
    )?;

    linker.func_wrap(
        "cage",
        "inbox_pending",
        |caller: Caller<'_, SandboxState>| -> i32 {
            if caller.data().inbox_state.is_some() { 1 } else { 0 }
        },
    )?;

    linker.func_wrap(
        "cage",
        "inbox_read",
        |mut caller: Caller<'_, SandboxState>, buf_ptr: i32, buf_max: i32| -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => return -1,
            };

            // Clone inbox state to avoid holding an immutable borrow via caller.data()
            // while also doing mutable operations via memory.read/write.
            let inbox = match caller.data().inbox_state.clone() {
                Some(inbox) => inbox,
                None => return 0,  // no message pending
            };

            // Read from inbox memory written by orchestrator
            let mut read_buf = vec![0u8; inbox.len as usize];
            if memory.read(&caller, inbox.ptr as usize, &mut read_buf).is_err() {
                return -1;
            }

            // Write to caller's buffer (truncate if too small)
            let copy_len = (inbox.len).min(buf_max);
            if memory.write(&mut caller, buf_ptr as usize, &read_buf[..copy_len as usize]).is_err() {
                return -1;
            }

            // IMPORTANT: clear pending flag so cage_inbox_pending() returns 0
            // (prevents infinite loop in agent that checks inbox_pending after read)
            caller.data_mut().inbox_state = None;

            inbox.len  // return actual length (so caller can retry if truncated)
        },
    )?;

    Ok(())
}

fn write_response(
    memory: &Memory,
    caller: &mut Caller<'_, SandboxState>,
    resp_ptr: i32,
    resp_max: i32,
    value: &serde_json::Value,
) -> i32 {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let total = bytes.len();

    if total > resp_max as usize {
        if resp_max > 0 {
            let _ = memory.write(caller, resp_ptr as usize, &bytes[..resp_max as usize]);
        }
        -(total as i32)
    } else {
        if total > 0 {
            let _ = memory.write(caller, resp_ptr as usize, &bytes);
        }
        total as i32
    }
}

fn execute_http(
    method: &str,
    url: &str,
    req: &serde_json::Value,
    memory: &Memory,
    caller: &mut Caller<'_, SandboxState>,
    resp_ptr: i32,
    resp_max: i32,
) -> i32 {
    use ureq::Agent;

    let agent = Agent::new_with_defaults();
    let headers = &req["headers"];

    let result = match method {
        "GET" => apply_headers!(agent.get(url), headers).call(),
        "POST" => {
            let body = req["body"].as_str().unwrap_or("");
            apply_headers!(agent.post(url), headers).send(body)
        }
        "PUT" => {
            let body = req["body"].as_str().unwrap_or("");
            apply_headers!(agent.put(url), headers).send(body)
        }
        "PATCH" => {
            let body = req["body"].as_str().unwrap_or("");
            apply_headers!(agent.patch(url), headers).send(body)
        }
        "DELETE" => apply_headers!(agent.delete(url), headers).call(),
        "HEAD" => apply_headers!(agent.head(url), headers).call(),
        "OPTIONS" => apply_headers!(agent.options(url), headers).call(),
        _ => {
            return write_response(
                memory,
                caller,
                resp_ptr,
                resp_max,
                &serde_json::json!({"status": 0, "error": format!("unsupported method: {method}")}),
            );
        }
    };

    let resp_json = match result {
        Ok(resp) => {
            let status = resp.status().as_u16();

            let mut hdrs = serde_json::Map::new();
            for (name, val) in resp.headers() {
                if let Ok(v) = val.to_str() {
                    hdrs.insert(name.to_string(), serde_json::Value::String(v.to_string()));
                }
            }

            let body_text = resp.into_body().read_to_string().unwrap_or_default();

            serde_json::json!({
                "status": status,
                "headers": hdrs,
                "body": body_text,
            })
        }
        Err(e) => serde_json::json!({
            "status": 0,
            "error": e.to_string(),
        }),
    };

    write_response(memory, caller, resp_ptr, resp_max, &resp_json)
}

fn is_url_allowed(url: &str, whitelist: &[String]) -> bool {
    whitelist.iter().any(|prefix| {
        if !url.starts_with(prefix) {
            return false;
        }
        let after = &url[prefix.len()..];
        after.is_empty() || after.starts_with('/')
            || after.starts_with('?') || after.starts_with('#')
            || after.starts_with(':')
    })
}
