//! Real OS-thread `worker_threads` implementation for v2.0.0.
//!
//! Each `new Worker(file, { workerData })` spawns an independent OS thread that
//! runs its own `JsEngine` (QuickJS context) and its own Tokio runtime.
//! Data crosses the thread boundary as JSON strings via `std::sync::mpsc`.
//!
//! ## Message flow
//!
//! ```text
//! Main thread                         Worker thread
//! ───────────                         ─────────────
//! worker.postMessage(v)
//!   → JSON.stringify(v)
//!   → __workerSend(id, json)      ──→ Rust mpsc → worker JS
//!                                       parentPort.emit('message', data)
//!
//! parentPort.postMessage(v)           parentPort.postMessage(v)
//!   → JSON.stringify(v)                 → __postMessageToParent(json)
//!   → pushes to VecDeque           ──→ shared queue
//! main setInterval polls
//!   __workerRecv(id) → json|null
//!   → worker.emit('message', data)
//! ```

use rquickjs::{Ctx, Function, Result};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use vvva_permissions::PermissionState;

// ── Global worker registry ────────────────────────────────────────────────────

type WorkerId = u32;

struct WorkerChannel {
    /// Send messages from main → worker.
    sender: std::sync::mpsc::SyncSender<String>,
    /// Queue of messages from worker → main (worker pushes, main polls).
    incoming: Arc<Mutex<VecDeque<String>>>,
    /// Join handle — `None` once joined.
    thread: Option<std::thread::JoinHandle<()>>,
}

static REGISTRY: OnceLock<Mutex<HashMap<WorkerId, WorkerChannel>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<WorkerId, WorkerChannel>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_worker_id() -> WorkerId {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// ── Worker thread entrypoint ──────────────────────────────────────────────────

fn worker_main(
    filename: String,
    worker_data_json: String,
    perms: Arc<PermissionState>,
    receiver: std::sync::mpsc::Receiver<String>,
    outgoing: Arc<Mutex<VecDeque<String>>>,
) {
    // Every worker needs its own tokio runtime (QuickJS is not Send).
    let rt = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[worker] failed to create tokio runtime: {e}");
            return;
        }
    };

    // Wrap the incoming channel so we can poll it from JS.
    let recv = Arc::new(Mutex::new(receiver));

    rt.block_on(async {
        let engine = match crate::JsEngine::new(perms).await {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[worker] engine init failed: {e}");
                return;
            }
        };

        // Inject worker globals and native helpers before running user code.
        let out = outgoing.clone();
        let recv2 = recv.clone();
        if let Err(e) = engine
            .context
            .with(move |ctx: rquickjs::Ctx| {
                inject_worker_globals(&ctx, &worker_data_json, out, recv2)
            })
            .await
        {
            eprintln!("[worker] inject_worker_globals: {e}");
            return;
        }

        // Evaluate the worker file.
        let path = PathBuf::from(&filename);
        if let Err(e) = engine.eval_file(&path).await {
            eprintln!("[worker] error in {filename}: {e}");
        }

        // Drain any remaining messages that arrived after the script completed.
        // Small event-loop spin to process pending promises.
        for _ in 0..100 {
            let _ = engine
                .context
                .with(|_ctx| Ok::<_, rquickjs::Error>(()))
                .await;
            tokio::task::yield_now().await;
        }
    });
}

/// Inject `isMainThread = false`, `workerData`, `parentPort`, and
/// native helpers into the worker's context.
fn inject_worker_globals(
    ctx: &Ctx,
    worker_data_json: &str,
    outgoing: Arc<Mutex<VecDeque<String>>>,
    receiver: Arc<Mutex<std::sync::mpsc::Receiver<String>>>,
) -> Result<()> {
    let globals = ctx.globals();

    // Override isMainThread flag in the already-loaded worker_threads module.
    ctx.eval::<(), _>(
        r#"
        if (typeof globalThis.__requireCache !== 'undefined' &&
            globalThis.__requireCache['worker_threads']) {
            globalThis.__requireCache['worker_threads'].isMainThread = false;
            globalThis.__requireCache['node:worker_threads'].isMainThread = false;
        }
        globalThis.__isMainThread = false;
        "#,
    )?;

    // Expose workerData.
    let wd = worker_data_json.to_string();
    let wd_code = format!(
        "globalThis.__workerDataJson = {}; \
         if (typeof globalThis.__requireCache !== 'undefined' && \
             globalThis.__requireCache['worker_threads']) {{ \
             globalThis.__requireCache['worker_threads'].workerData = globalThis.__workerDataJson; \
         }}",
        wd
    );
    ctx.eval::<(), _>(wd_code.as_str())?;

    // __postMessageToParent(json) — called by parentPort.postMessage inside the worker.
    let fn_out = outgoing.clone();
    let post_to_parent = Function::new(ctx.clone(), move |json: String| {
        fn_out.lock().unwrap().push_back(json);
    })?;
    globals.set("__postMessageToParent", post_to_parent)?;

    // __workerRecvFromParent() → String|null — poll one message sent by the main thread.
    let fn_recv = receiver.clone();
    let recv_fn = Function::new(ctx.clone(), move || -> Option<String> {
        fn_recv.lock().unwrap().try_recv().ok()
    })?;
    globals.set("__workerRecvFromParent", recv_fn)?;

    // Wire parentPort using the injected helpers.
    ctx.eval::<(), _>(
        r#"
        (function() {
            var EventEmitter = (globalThis.__requireCache && globalThis.__requireCache['events'])
                || { call: function() {}, prototype: { emit: function() {}, on: function() {} } };

            function ParentPort() {
                if (EventEmitter.call) EventEmitter.call(this);
            }
            if (EventEmitter.prototype) {
                ParentPort.prototype = Object.create(EventEmitter.prototype);
                ParentPort.prototype.constructor = ParentPort;
            }

            ParentPort.prototype.postMessage = function(data) {
                __postMessageToParent(JSON.stringify(data));
            };
            ParentPort.prototype.close = function() {
                this.emit('close');
            };
            ParentPort.prototype.unref = function() { return this; };
            ParentPort.prototype.ref = function() { return this; };

            var parentPort = new ParentPort();

            // Poll messages from main thread every 10ms.
            setInterval(function() {
                var msg;
                while ((msg = __workerRecvFromParent()) !== null && msg !== undefined) {
                    try {
                        parentPort.emit('message', JSON.parse(msg));
                    } catch(e) {
                        parentPort.emit('message', msg);
                    }
                }
            }, 10);

            globalThis.parentPort = parentPort;
            if (globalThis.__requireCache && globalThis.__requireCache['worker_threads']) {
                globalThis.__requireCache['worker_threads'].parentPort = parentPort;
                globalThis.__requireCache['node:worker_threads'].parentPort = parentPort;
            }
        }());
        "#,
    )?;

    Ok(())
}

// ── Native functions injected into the MAIN thread ────────────────────────────

/// Inject `__workerCreate`, `__workerSend`, `__workerTerminate`, `__workerRecv`
/// into the main-thread JS context, and upgrade the `worker_threads.Worker`
/// constructor to use them.
pub fn inject_worker_threads_native(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let perms = permissions.clone();

    // __workerCreate(filename, workerDataJson) → workerId: u32
    let create_fn = Function::new(
        ctx.clone(),
        move |filename: String, worker_data: String| -> u32 {
            let id = next_worker_id();
            let incoming: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
            let (tx, rx) = std::sync::mpsc::sync_channel::<String>(256);

            let in_clone = incoming.clone();
            let perms_clone = perms.clone();
            let handle = std::thread::Builder::new()
                .name(format!("3va-worker-{id}"))
                .spawn(move || worker_main(filename, worker_data, perms_clone, rx, in_clone))
                .expect("failed to spawn worker thread");

            registry().lock().unwrap().insert(
                id,
                WorkerChannel {
                    sender: tx,
                    incoming,
                    thread: Some(handle),
                },
            );
            id
        },
    )?;
    ctx.globals().set("__workerCreate", create_fn)?;

    // __workerSend(workerId, dataJson) → bool
    let send_fn = Function::new(ctx.clone(), move |id: u32, json: String| -> bool {
        let reg = registry().lock().unwrap();
        if let Some(ch) = reg.get(&id) {
            ch.sender.try_send(json).is_ok()
        } else {
            false
        }
    })?;
    ctx.globals().set("__workerSend", send_fn)?;

    // __workerRecv(workerId) → String|null  (polls one pending message)
    let recv_fn = Function::new(ctx.clone(), move |id: u32| -> Option<String> {
        let reg = registry().lock().unwrap();
        if let Some(ch) = reg.get(&id) {
            ch.incoming.lock().unwrap().pop_front()
        } else {
            None
        }
    })?;
    ctx.globals().set("__workerRecv", recv_fn)?;

    // __workerTerminate(workerId) → bool
    let term_fn = Function::new(ctx.clone(), move |id: u32| -> bool {
        let mut reg = registry().lock().unwrap();
        if let Some(mut ch) = reg.remove(&id) {
            // Drop the sender — the worker will get an Err on next recv and can check.
            // We cannot forcibly kill a thread in Rust; just disconnect.
            drop(ch.sender);
            if let Some(handle) = ch.thread.take() {
                drop(handle); // non-blocking — worker thread detached
            }
            true
        } else {
            false
        }
    })?;
    ctx.globals().set("__workerTerminate", term_fn)?;

    // Upgrade the JS Worker constructor to use native backing.
    ctx.eval::<(), _>(
        r#"
        (function() {
            var EventEmitter = (globalThis.__requireCache && globalThis.__requireCache['events']) || null;

            function Worker(filename, options) {
                if (EventEmitter) EventEmitter.call(this);
                options = options || {};
                var workerDataJson = JSON.stringify(options.workerData !== undefined ? options.workerData : null);
                this._id = __workerCreate(String(filename), workerDataJson);
                this.threadId = this._id;
                this.resourceLimits = {};

                var self = this;
                // Poll for incoming messages every 16ms (~60 fps).
                this._pollInterval = setInterval(function() {
                    var msg;
                    while ((msg = __workerRecv(self._id)) !== null && msg !== undefined) {
                        try {
                            self.emit('message', JSON.parse(msg));
                        } catch(e) {
                            self.emit('message', msg);
                        }
                    }
                }, 16);
            }

            if (EventEmitter) {
                Worker.prototype = Object.create(EventEmitter.prototype);
                Worker.prototype.constructor = Worker;
            }

            Worker.prototype.postMessage = function(data) {
                __workerSend(this._id, JSON.stringify(data));
            };

            Worker.prototype.terminate = function() {
                clearInterval(this._pollInterval);
                __workerTerminate(this._id);
                return Promise.resolve(0);
            };

            Worker.prototype.unref = function() { return this; };
            Worker.prototype.ref = function() { return this; };

            // Patch the worker_threads module.
            if (globalThis.__requireCache && globalThis.__requireCache['worker_threads']) {
                globalThis.__requireCache['worker_threads'].Worker = Worker;
                globalThis.__requireCache['node:worker_threads'].Worker = Worker;
                globalThis.__requireCache['worker_threads'].isMainThread = true;
                globalThis.__requireCache['node:worker_threads'].isMainThread = true;
            }
        }());
        "#,
    )?;

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_id_is_unique() {
        let a = next_worker_id();
        let b = next_worker_id();
        assert_ne!(a, b);
        assert!(b > a);
    }

    #[test]
    fn registry_insert_and_recv() {
        let incoming: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let id = next_worker_id();

        // Simulate worker sending a message to main.
        incoming
            .lock()
            .unwrap()
            .push_back(r#"{"hello":"world"}"#.into());

        registry().lock().unwrap().insert(
            id,
            WorkerChannel {
                sender: tx,
                incoming: incoming.clone(),
                thread: None,
            },
        );

        let reg = registry().lock().unwrap();
        let ch = reg.get(&id).unwrap();
        let msg = ch.incoming.lock().unwrap().pop_front();
        assert_eq!(msg.as_deref(), Some(r#"{"hello":"world"}"#));
        drop(reg);
        drop(rx); // prevent leak warning
    }
}
