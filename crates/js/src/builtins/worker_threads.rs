//! Real OS-thread `worker_threads` implementation.
//!
//! Each `new Worker(file, { workerData })` spawns an independent OS thread.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use v8::{Function, FunctionCallbackArguments, PinScope, ReturnValue};
use vvva_permissions::PermissionState;

// Thread-local, not a process-wide static — see the identical fix (and
// rationale) in fs.rs's FS_PERMISSIONS: a `OnceLock` here only keeps the
// *first* engine's permissions ever created in the process, so every later
// `JsEngine` (every other test, or a second engine in a long-lived process)
// silently inherits the first one's grants instead of its own.
thread_local! {
    static WT_PERMISSIONS: std::cell::RefCell<Option<Arc<PermissionState>>> =
        const { std::cell::RefCell::new(None) };
}
fn perms() -> Arc<PermissionState> {
    WT_PERMISSIONS.with(|p| {
        p.borrow()
            .clone()
            .expect("inject_worker_threads not called on this thread")
    })
}

type WorkerId = u32;

struct WorkerChannel {
    sender: std::sync::mpsc::SyncSender<String>,
    incoming: Arc<Mutex<VecDeque<String>>>,
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

pub fn inject_worker_threads_native(scope: &mut PinScope, permissions: Arc<PermissionState>) {
    let context = scope.get_current_context();
    let global = context.global(scope);
    WT_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions));

    let create_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let filename = args.get(0).to_rust_string_lossy(scope);
            let worker_data = args.get(1).to_rust_string_lossy(scope);

            let id = next_worker_id();
            let incoming: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
            let (tx, rx) = std::sync::mpsc::sync_channel::<String>(256);

            let in_clone = incoming.clone();
            let perms_clone = perms();

            let handle = std::thread::Builder::new()
                .name(format!("3va-worker-{}", id))
                .spawn(move || {
                    let rt = match tokio::runtime::Runtime::new() {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("[worker] failed to create tokio runtime: {}", e);
                            return;
                        }
                    };

                    let recv = Arc::new(Mutex::new(rx));

                    rt.block_on(async {
                        let mut engine = match crate::JsEngine::new(perms_clone).await {
                            Ok(e) => e,
                            Err(e) => {
                                eprintln!("[worker] engine init failed: {}", e);
                                return;
                            }
                        };

                        let out = in_clone.clone();
                        let recv2 = recv.clone();
                        let inject_result = engine
                            .with_scope(|scope| {
                                inject_worker_globals(scope, &worker_data, out, recv2)
                            })
                            .await;
                        if let Err(e) = inject_result {
                            eprintln!("[worker] inject_worker_globals: {}", e);
                            return;
                        }

                        let path = PathBuf::from(&filename);
                        if let Err(e) = engine.eval_file(&path).await {
                            eprintln!("[worker] error in {}: {}", filename, e);
                        }

                        for _ in 0..100 {
                            engine.idle().await;
                            tokio::task::yield_now().await;
                        }
                    });
                })
                .expect("failed to spawn worker thread");

            registry().lock().unwrap().insert(
                id,
                WorkerChannel {
                    sender: tx,
                    incoming,
                    thread: Some(handle),
                },
            );

            rv.set(v8::Number::new(scope, id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__workerCreate").unwrap().into(),
        create_fn.into(),
    );

    let send_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0);
            let json = args.get(1).to_rust_string_lossy(scope);

            let reg = registry().lock().unwrap();
            let result = if let Some(ch) = reg.get(&id) {
                ch.sender.try_send(json).is_ok()
            } else {
                false
            };
            rv.set(v8::Boolean::new(scope, result).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__workerSend").unwrap().into(),
        send_fn.into(),
    );

    let recv_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0);

            let reg = registry().lock().unwrap();
            let msg = if let Some(ch) = reg.get(&id) {
                ch.incoming.lock().unwrap().pop_front()
            } else {
                None
            };

            match msg {
                Some(s) => rv.set(v8::String::new(scope, &s).unwrap().into()),
                None => rv.set(v8::undefined(scope).into()),
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__workerRecv").unwrap().into(),
        recv_fn.into(),
    );

    let term_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0);

            let mut reg = registry().lock().unwrap();
            if let Some(mut ch) = reg.remove(&id) {
                drop(ch.sender);
                if let Some(handle) = ch.thread.take() {
                    drop(handle);
                }
                rv.set(v8::Boolean::new(scope, true).into());
            } else {
                rv.set(v8::Boolean::new(scope, false).into());
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__workerTerminate").unwrap().into(),
        term_fn.into(),
    );

    let js_code = r#"
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
            this._pollInterval = setInterval(function() {
                var msg;
                while ((msg = __workerRecv(self._id)) !== null && msg !== undefined) {
                    try {
                        this.emit('message', JSON.parse(msg));
                    } catch(e) {
                        this.emit('message', msg);
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

        // MessageChannel/MessagePort — pure in-process (no native binding
        // needed, unlike Worker which crosses real OS threads): a connected
        // pair of EventEmitters that hand messages to each other async.
        function MessagePort() {
            if (EventEmitter) EventEmitter.call(this);
            this._peer = null;
        }
        if (EventEmitter) {
            MessagePort.prototype = Object.create(EventEmitter.prototype);
            MessagePort.prototype.constructor = MessagePort;
        }
        MessagePort.prototype.postMessage = function(data) {
            var peer = this._peer;
            if (!peer) return;
            setTimeout(function() { peer.emit('message', data); }, 0);
        };
        MessagePort.prototype.close = function() { this.emit('close'); };
        MessagePort.prototype.start = function() {};
        MessagePort.prototype.unref = function() { return this; };
        MessagePort.prototype.ref = function() { return this; };

        function MessageChannel() {
            this.port1 = new MessagePort();
            this.port2 = new MessagePort();
            this.port1._peer = this.port2;
            this.port2._peer = this.port1;
        }

        // The other worker_threads.rs JS blocks (inject_worker_globals'
        // parentPort setup) only *patch* __requireCache['worker_threads']
        // if it already exists — nothing ever created the base object, so
        // require('worker_threads') always failed with "Cannot find
        // module". Create it here unconditionally.
        globalThis.__requireCache = globalThis.__requireCache || {};
        globalThis.__requireCache['worker_threads'] = globalThis.__requireCache['worker_threads'] || {};
        globalThis.__requireCache['node:worker_threads'] = globalThis.__requireCache['worker_threads'];
        globalThis.__requireCache['worker_threads'].Worker = Worker;
        globalThis.__requireCache['worker_threads'].MessageChannel = MessageChannel;
        globalThis.__requireCache['worker_threads'].MessagePort = MessagePort;
        globalThis.__requireCache['worker_threads'].isMainThread = true;
        globalThis.__requireCache['worker_threads'].SHARE_ENV = Symbol('nodejs.worker_threads.SHARE_ENV');
        globalThis.__requireCache['worker_threads'].workerData = null;
        globalThis.__requireCache['worker_threads'].parentPort = null;
        globalThis.__requireCache['worker_threads'].threadId = 0;
    })();
    "#;

    let source = v8::String::new(scope, js_code).unwrap();
    if let Some(script) = v8::Script::compile(scope, source, None) {
        let _ = script.run(scope);
    }
}

fn inject_worker_globals(
    scope: &mut v8::ContextScope<v8::HandleScope>,
    worker_data_json: &str,
    outgoing: Arc<Mutex<VecDeque<String>>>,
    receiver: Arc<Mutex<std::sync::mpsc::Receiver<String>>>,
) -> anyhow::Result<()> {
    let context = scope.get_current_context();
    let global = context.global(scope);

    let js_code = r#"
    if (typeof globalThis.__requireCache !== 'undefined' &&
        globalThis.__requireCache['worker_threads']) {
        globalThis.__requireCache['worker_threads'].isMainThread = false;
        globalThis.__requireCache['node:worker_threads'].isMainThread = false;
    }
    globalThis.__isMainThread = false;
    "#;

    let source = v8::String::new(scope, js_code).unwrap();
    if let Some(script) = v8::Script::compile(scope, source, None) {
        let _ = script.run(scope);
    }

    let wd_code = format!(
        "globalThis.__workerDataJson = {}; \
         if (typeof globalThis.__requireCache !== 'undefined' && \
             globalThis.__requireCache['worker_threads']) {{ \
             globalThis.__requireCache['worker_threads'].workerData = globalThis.__workerDataJson; \
         }}",
        worker_data_json
    );

    let source = v8::String::new(scope, &wd_code).unwrap();
    if let Some(script) = v8::Script::compile(scope, source, None) {
        let _ = script.run(scope);
    }

    let out_ptr = Arc::into_raw(outgoing) as *mut std::ffi::c_void;
    let out_external = v8::External::new(scope, out_ptr);
    let post_to_parent = v8::Function::builder(
        |scope: &mut PinScope, args: FunctionCallbackArguments, _rv: ReturnValue| {
            let fn_out = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                Arc::from_raw(ptr as *const Mutex<VecDeque<String>>)
            };
            let json = args.get(0).to_rust_string_lossy(scope);
            fn_out.lock().unwrap().push_back(json);
            std::mem::forget(fn_out);
        },
    )
    .data(out_external.into())
    .build(scope)
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__postMessageToParent")
            .unwrap()
            .into(),
        post_to_parent.into(),
    );

    let recv_ptr = Arc::into_raw(receiver) as *mut std::ffi::c_void;
    let recv_external = v8::External::new(scope, recv_ptr);
    let recv_fn = v8::Function::builder(
        |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let fn_recv = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                Arc::from_raw(ptr as *const Mutex<std::sync::mpsc::Receiver<String>>)
            };
            let msg = fn_recv.lock().unwrap().try_recv().ok();
            match msg {
                Some(s) => rv.set(v8::String::new(scope, &s).unwrap().into()),
                None => rv.set(v8::undefined(scope).into()),
            }
            std::mem::forget(fn_recv);
        },
    )
    .data(recv_external.into())
    .build(scope)
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__workerRecvFromParent")
            .unwrap()
            .into(),
        recv_fn.into(),
    );

    let js_code2 = r#"
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
        globalThis.__requireCache = globalThis.__requireCache || {};
        globalThis.__requireCache['worker_threads'] = globalThis.__requireCache['worker_threads'] || {};
        globalThis.__requireCache['node:worker_threads'] = globalThis.__requireCache['worker_threads'];
        globalThis.__requireCache['worker_threads'].parentPort = parentPort;
        globalThis.__requireCache['worker_threads'].isMainThread = false;
    })();
    "#;

    let source = v8::String::new(scope, js_code2).unwrap();
    if let Some(script) = v8::Script::compile(scope, source, None) {
        let _ = script.run(scope);
    }

    Ok(())
}
