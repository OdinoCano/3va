//! JavaScript engine crate — wraps V8 via the `v8` crate, exposes `JsEngine` and all built-in modules.

pub mod async_context;
pub mod builtins;
pub mod esm;
pub mod inspector;
pub mod profiler;
pub mod rejection_tracker;
pub mod transpiler;

use std::net::SocketAddr;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use v8::Isolate;
use vvva_core::Runtime;
use vvva_firewall::Firewall;
use vvva_permissions::PermissionState;

use builtins::TimerManager;
use profiler::Profiler;

static INSPECTOR_STATE_CELL: std::sync::OnceLock<Arc<inspector::InspectorState>> =
    std::sync::OnceLock::new();
static PROFILER_HANDLE_CELL: std::sync::OnceLock<Profiler> = std::sync::OnceLock::new();

// V8 only supports being initialized once per process; a second
// `v8::V8::initialize()` call (e.g. from a second `JsEngine` in the same
// process, as happens constantly across `#[tokio::test]` functions) corrupts
// global V8 state ("Invalid global state" panics deep in the v8 crate).
static V8_INIT: std::sync::Once = std::sync::Once::new();

// Retained so `run_event_loop`/`idle` can pump V8's own background→foreground
// task queue (used by e.g. async WebAssembly compilation). Without this,
// `WebAssembly.instantiate()`'s promise never settles: the microtask
// checkpoint alone does not run tasks V8 posts to the platform.
pub static V8_PLATFORM: std::sync::OnceLock<v8::SharedRef<v8::Platform>> =
    std::sync::OnceLock::new();

/// Initializes the V8 platform, if it hasn't been already. Safe to call any
/// number of times from any number of places in the process — e.g. once per
/// `JsEngine`, plus any standalone `v8::Isolate` created outside of one
/// (like the CJS export-name probe in `vvva_cli`).
/// When a script fails, which stage produced the exception — surfaced in the
/// error message as e.g. `"[parse] SyntaxError: ..."` so callers (like the
/// test262 runner, which must match tc39/test262's `negative.phase`) can tell
/// a compile-time failure from a runtime one without re-running the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalPhase {
    Parse,
    Runtime,
}

impl std::fmt::Display for EvalPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            EvalPhase::Parse => "parse",
            EvalPhase::Runtime => "runtime",
        })
    }
}

pub fn ensure_v8_initialized() {
    V8_INIT.call_once(|| {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform.clone());
        v8::V8::initialize();
        let _ = V8_PLATFORM.set(platform);
    });
}

/// Runs one ready V8 platform task (foreground or background-completed) for
/// `isolate`, if any is pending. Deliberately not looped to exhaustion: a
/// task that reposts more work could otherwise spin this call forever. The
/// caller (idle()/run_event_loop()) already runs repeatedly, so tasks drain
/// incrementally across iterations just like timers do.
fn pump_v8_platform_tasks(isolate: &v8::Isolate) {
    if let Some(platform) = V8_PLATFORM.get() {
        v8::Platform::pump_message_loop(platform, isolate, false);
    }
}

/// Realms created via `$262.createRealm()` (see `install_realm_support`),
/// keyed by index. Stored as isolate embedder data so the registry outlives
/// any single callback invocation, for as long as the isolate lives.
type RealmRegistry = std::cell::RefCell<Vec<v8::Global<v8::Context>>>;

/// Sets `globalThis.__native_createRealm` in the current context to a
/// function that creates a new `v8::Context` in the same isolate and
/// returns `{ global, evalScript }` for it.
fn install_realm_support(scope: &mut v8::ContextScope<v8::HandleScope>) {
    let context = scope.get_current_context();
    let global = context.global(scope);
    let create_fn = v8::Function::new(scope, create_realm_callback).unwrap();
    let key = v8::String::new(scope, "__native_createRealm").unwrap();
    global.set(scope, key.into(), create_fn.into());
}

fn create_realm_callback(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let new_ctx = v8::Context::new(scope, Default::default());
    // Match the caller's security token so cross-realm property access on
    // the new realm's global proxy (e.g. `other.Function`, `other.Array`)
    // isn't rejected by V8's default access checks — without this, every
    // access from outside the new context throws a bare "no access"
    // TypeError, defeating the entire point of exposing `.global`.
    let caller_token = scope.get_current_context().get_security_token(scope);
    new_ctx.set_security_token(caller_token);
    let global_handle = v8::Global::new(scope.as_ref(), new_ctx);

    if scope.get_slot::<RealmRegistry>().is_none() {
        scope.set_slot(RealmRegistry::default());
    }
    let idx = {
        let registry = scope.get_slot::<RealmRegistry>().unwrap();
        let mut list = registry.borrow_mut();
        list.push(global_handle);
        list.len() - 1
    };

    let result = v8::Object::new(scope);

    let global_key = v8::String::new(scope, "global").unwrap();
    let new_global = new_ctx.global(scope);
    result.set(scope, global_key.into(), new_global.into());

    let idx_num = v8::Number::new(scope, idx as f64);
    let eval_fn = v8::Function::builder(realm_eval_script_callback)
        .data(idx_num.into())
        .build(scope)
        .unwrap();
    let eval_key = v8::String::new(scope, "evalScript").unwrap();
    result.set(scope, eval_key.into(), eval_fn.into());

    rv.set(result.into());
}

/// `evalScript(src)` on a realm object returned by `$262.createRealm()`:
/// compiles and runs `src` inside that realm's own `v8::Context`. On
/// failure, rethrows the realm's own exception value as-is (not a copy or a
/// message string) so tests checking cross-realm identity (e.g.
/// `err.constructor === realm.global.SyntaxError`) see the real thing.
fn realm_eval_script_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let idx = args.data().number_value(scope).unwrap_or(-1.0);
    let src = args.get(0).to_rust_string_lossy(scope);

    let ctx_global = {
        let Some(registry) = scope.get_slot::<RealmRegistry>() else {
            let msg = v8::String::new(scope, "realm registry missing").unwrap();
            scope.throw_exception(msg.into());
            return;
        };
        let list = registry.borrow();
        let Some(g) = (idx >= 0.0).then(|| list.get(idx as usize)).flatten() else {
            let msg = v8::String::new(scope, "invalid realm handle").unwrap();
            scope.throw_exception(msg.into());
            return;
        };
        g.clone()
    };

    let context = v8::Local::new(scope, &ctx_global);
    let mut ctx_scope = v8::ContextScope::new(scope, context);
    v8::tc_scope!(let try_catch, &mut ctx_scope);
    let Some(source) = v8::String::new(try_catch, &src) else {
        return;
    };
    let script = match v8::Script::compile(try_catch, source, None) {
        Some(s) => s,
        None => {
            if let Some(exc) = try_catch.exception() {
                try_catch.throw_exception(exc);
            }
            return;
        }
    };
    match script.run(try_catch) {
        Some(result) => rv.set(result),
        None => {
            if let Some(exc) = try_catch.exception() {
                try_catch.throw_exception(exc);
            }
        }
    }
}

// ─── $262.agent support (test262's worker/shared-memory API) ──────────────
//
// Each agent is a real OS thread with its own `v8::Isolate` — isolates
// aren't `Send`, so this can't be simulated with async tasks the way
// createRealm's realms are (those stay in one isolate). What crosses
// threads is only the `SharedArrayBuffer`'s backing store; V8's own
// Atomics.wait/notify implementation already handles cross-isolate
// synchronization correctly once the same backing store is shared, so
// there's no wait/notify logic to reimplement here — just plumbing.

/// A `v8::SharedRef<v8::BackingStore>` derefs to `[Cell<u8>]`, so the v8
/// crate deliberately leaves it `!Sync` — callers must synchronize access
/// themselves. Test262's `SharedArrayBuffer`/`Atomics` contract *is* that
/// synchronization (same guarantee `Deno`/Node's `worker_threads` build on),
/// so this wrapper asserts it to let the handle cross a channel.
struct SendableBackingStore(v8::SharedRef<v8::BackingStore>);
unsafe impl Send for SendableBackingStore {}

type BroadcastMsg = (SendableBackingStore, i32);

/// Per-test-case `$262.agent` state: install a fresh one per
/// `install_test262_agent_support` call (i.e. per test262 case) so agents
/// from unrelated test files — each running on their own thread via
/// `test262::run_suite` — never cross-talk.
#[derive(Default)]
pub struct AgentHub {
    mailboxes: Mutex<Vec<std::sync::mpsc::Sender<BroadcastMsg>>>,
    reports: Mutex<std::collections::VecDeque<String>>,
}

/// Hard cap on how long a spawned agent thread waits for broadcasts before
/// giving up and exiting, in case a test never calls `$262.agent.leaving()`.
/// Matches the largest timeout test262's own harness uses
/// (`$262.agent.timeouts.huge`, see harness/atomicsHelper.js) with headroom.
const AGENT_MAX_LIFETIME: std::time::Duration = std::time::Duration::from_secs(15);

fn process_start() -> std::time::Instant {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *START.get_or_init(std::time::Instant::now)
}

fn install_agent_sleep_and_clock(scope: &mut v8::PinScope, target: v8::Local<v8::Object>) {
    let sleep_fn = v8::Function::new(scope, agent_sleep_callback).unwrap();
    let key = v8::String::new(scope, "sleep").unwrap();
    target.set(scope, key.into(), sleep_fn.into());

    let now_fn = v8::Function::new(scope, agent_monotonic_now_callback).unwrap();
    let key = v8::String::new(scope, "monotonicNow").unwrap();
    target.set(scope, key.into(), now_fn.into());
}

fn agent_sleep_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let ms = args.get(0).number_value(scope).unwrap_or(0.0);
    if ms > 0.0 {
        std::thread::sleep(std::time::Duration::from_millis(ms as u64));
    }
}

fn agent_monotonic_now_callback(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let ms = process_start().elapsed().as_secs_f64() * 1000.0;
    rv.set(v8::Number::new(scope, ms).into());
}

/// Installs the MAIN-thread-side `$262.agent` primitives (`start`,
/// `broadcast`, `getReport`, plus `sleep`/`monotonicNow`) onto `target`
/// (the JS object test262.rs's `$262.agent` literal will use), backed by a
/// fresh `AgentHub` stashed in this context's isolate slot.
fn install_agent_main(
    scope: &mut v8::ContextScope<v8::HandleScope>,
    target: v8::Local<v8::Object>,
) {
    scope.set_slot(Arc::new(AgentHub::default()));

    let start_fn = v8::Function::new(scope, agent_start_callback).unwrap();
    let key = v8::String::new(scope, "start").unwrap();
    target.set(scope, key.into(), start_fn.into());

    let broadcast_fn = v8::Function::new(scope, agent_broadcast_callback).unwrap();
    let key = v8::String::new(scope, "broadcast").unwrap();
    target.set(scope, key.into(), broadcast_fn.into());

    let get_report_fn = v8::Function::new(scope, agent_get_report_callback).unwrap();
    let key = v8::String::new(scope, "getReport").unwrap();
    target.set(scope, key.into(), get_report_fn.into());

    install_agent_sleep_and_clock(scope, target);
}

fn agent_start_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Some(hub) = scope.get_slot::<Arc<AgentHub>>().cloned() else {
        let msg = v8::String::new(scope, "$262.agent used before installation").unwrap();
        scope.throw_exception(msg.into());
        return;
    };
    let src = args.get(0).to_rust_string_lossy(scope);

    let (tx, rx) = std::sync::mpsc::channel::<BroadcastMsg>();
    hub.mailboxes.lock().unwrap().push(tx);

    std::thread::spawn(move || run_agent_thread(hub, rx, src));
}

fn agent_broadcast_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Some(hub) = scope.get_slot::<Arc<AgentHub>>().cloned() else {
        return;
    };
    let Ok(sab) = args.get(0).try_cast::<v8::SharedArrayBuffer>() else {
        let msg =
            v8::String::new(scope, "$262.agent.broadcast expects a SharedArrayBuffer").unwrap();
        scope.throw_exception(msg.into());
        return;
    };
    let id = args.get(1).number_value(scope).unwrap_or(0.0) as i32;
    let store = sab.get_backing_store();

    let mut mailboxes = hub.mailboxes.lock().unwrap();
    mailboxes.retain(|tx| tx.send((SendableBackingStore(store.clone()), id)).is_ok());
}

fn agent_get_report_callback(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(hub) = scope.get_slot::<Arc<AgentHub>>().cloned() else {
        return;
    };
    let next = hub.reports.lock().unwrap().pop_front();
    match next {
        Some(s) => {
            let v = v8::String::new(scope, &s).unwrap();
            rv.set(v.into());
        }
        None => rv.set_null(),
    }
}

/// Per-agent-isolate state for the callback registered via
/// `$262.agent.receiveBroadcast(fn)`, and the flag `$262.agent.leaving()`
/// sets to tell `run_agent_thread`'s loop to stop.
#[derive(Default)]
struct AgentSideState {
    receive_broadcast: std::cell::RefCell<Option<v8::Global<v8::Function>>>,
    leaving: std::cell::Cell<bool>,
}

/// Runs one `$262.agent.start(script)` agent: a bare isolate + context on
/// its own OS thread (isolates aren't `Send`, so this can't share the
/// caller's). Evaluates `script`, then services broadcasts (invoking any
/// `receiveBroadcast` callback the script registered) until `leaving()` is
/// called, the channel closes, or `AGENT_MAX_LIFETIME` elapses.
fn run_agent_thread(hub: Arc<AgentHub>, rx: std::sync::mpsc::Receiver<BroadcastMsg>, src: String) {
    ensure_v8_initialized();
    let mut isolate = v8::Isolate::new(Default::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);

    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut scope = scope.init();
    let context = v8::Context::new(&scope, Default::default());
    let mut scope = v8::ContextScope::new(&mut scope, context);

    scope.set_slot(hub.clone());
    scope.set_slot(Rc::new(AgentSideState::default()));

    let global = scope.get_current_context().global(&scope);
    let dollar262 = v8::Object::new(&scope);
    let agent_obj = v8::Object::new(&scope);

    let recv_fn = v8::Function::new(&mut scope, agent_receive_broadcast_callback).unwrap();
    let key = v8::String::new(&scope, "receiveBroadcast").unwrap();
    agent_obj.set(&scope, key.into(), recv_fn.into());

    let report_fn = v8::Function::new(&mut scope, agent_report_callback).unwrap();
    let key = v8::String::new(&scope, "report").unwrap();
    agent_obj.set(&scope, key.into(), report_fn.into());

    let leaving_fn = v8::Function::new(&mut scope, agent_leaving_callback).unwrap();
    let key = v8::String::new(&scope, "leaving").unwrap();
    agent_obj.set(&scope, key.into(), leaving_fn.into());

    install_agent_sleep_and_clock(&mut scope, agent_obj);

    let key = v8::String::new(&scope, "agent").unwrap();
    dollar262.set(&scope, key.into(), agent_obj.into());
    let key = v8::String::new(&scope, "$262").unwrap();
    global.set(&scope, key.into(), dollar262.into());

    {
        v8::tc_scope!(let try_catch, &mut scope);
        if let Some(source) = v8::String::new(try_catch, &src) {
            match v8::Script::compile(try_catch, source, None) {
                Some(script) => {
                    if script.run(try_catch).is_none()
                        && let Some(exc) = try_catch.exception()
                    {
                        eprintln!(
                            "[test262 agent] script threw: {}",
                            exc.to_rust_string_lossy(try_catch)
                        );
                    }
                }
                None => {
                    if let Some(exc) = try_catch.exception() {
                        eprintln!(
                            "[test262 agent] parse error: {}",
                            exc.to_rust_string_lossy(try_catch)
                        );
                    }
                }
            }
        }
    }

    let deadline = std::time::Instant::now() + AGENT_MAX_LIFETIME;
    while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
        match rx.recv_timeout(remaining) {
            Ok((store, id)) => {
                let state = scope.get_slot::<Rc<AgentSideState>>().unwrap().clone();
                let callback = state.receive_broadcast.borrow().clone();
                if let Some(cb) = callback {
                    v8::tc_scope!(let try_catch, &mut scope);
                    let func = v8::Local::new(try_catch, &cb);
                    let sab = v8::SharedArrayBuffer::with_backing_store(try_catch, &store.0);
                    let id_val = v8::Number::new(try_catch, id as f64);
                    let recv = v8::undefined(try_catch);
                    if func
                        .call(try_catch, recv.into(), &[sab.into(), id_val.into()])
                        .is_none()
                        && let Some(exc) = try_catch.exception()
                    {
                        eprintln!(
                            "[test262 agent] receiveBroadcast threw: {}",
                            exc.to_rust_string_lossy(try_catch)
                        );
                    }
                }
                if scope
                    .get_slot::<Rc<AgentSideState>>()
                    .unwrap()
                    .leaving
                    .get()
                {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn agent_receive_broadcast_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Some(state) = scope.get_slot::<Rc<AgentSideState>>().cloned() else {
        return;
    };
    let Ok(func) = args.get(0).try_cast::<v8::Function>() else {
        return;
    };
    *state.receive_broadcast.borrow_mut() = Some(v8::Global::new(scope.as_ref(), func));
}

fn agent_report_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Some(hub) = scope.get_slot::<Arc<AgentHub>>().cloned() else {
        return;
    };
    let msg = args.get(0).to_rust_string_lossy(scope);
    hub.reports.lock().unwrap().push_back(msg);
}

fn agent_leaving_callback(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    if let Some(state) = scope.get_slot::<Rc<AgentSideState>>() {
        state.leaving.set(true);
    }
}

pub struct JsEngine {
    isolate: v8::OwnedIsolate,
    context: Option<v8::Global<v8::Context>>,
    _permissions: Arc<PermissionState>,
    timer_manager: Arc<TimerManager>,
    runtime_core: Mutex<Runtime>,
    inspector: Option<Arc<inspector::InspectorState>>,
    profiler: Option<Profiler>,
    profiler_interval_ms: u32,
    ws_pool: builtins::websocket::WsPool,
    server_mode: bool,
    // V8 manages its own heap independently of Rust's global allocator, so
    // switching that allocator (e.g. to mimalloc) has zero effect on V8's
    // memory footprint. Left unprompted, V8 grows its heap to whatever
    // high-water mark a burst of allocation reaches and never shrinks back
    // down on its own — measured as ~3.5KB of RSS retained per HTTP request
    // served, indefinitely, under sustained load (see bench/README.md).
    // `low_memory_notification()` is the only thing that tells V8 to
    // actually try to free memory; run_event_loop calls it on a throttle
    // (LOW_MEMORY_HINT_INTERVAL) so busy periods aren't paused by a full
    // GC on every single tick.
    last_low_memory_hint: std::time::Instant,
}

const LOW_MEMORY_HINT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

impl JsEngine {
    pub async fn new(permissions: Arc<PermissionState>) -> anyhow::Result<Self> {
        Self::new_full(permissions, None, None, None).await
    }

    pub async fn new_with_firewall(
        permissions: Arc<PermissionState>,
        firewall: Arc<Firewall>,
    ) -> anyhow::Result<Self> {
        Self::new_full(permissions, None, None, Some(firewall)).await
    }

    pub async fn new_with_inspector(
        permissions: Arc<PermissionState>,
        inspect_addr: Option<SocketAddr>,
    ) -> anyhow::Result<Self> {
        Self::new_full(permissions, inspect_addr, None, None).await
    }

    pub async fn new_with_firewall_and_inspector(
        permissions: Arc<PermissionState>,
        firewall: Arc<Firewall>,
        inspect_addr: Option<SocketAddr>,
    ) -> anyhow::Result<Self> {
        Self::new_full(permissions, inspect_addr, None, Some(firewall)).await
    }

    pub async fn new_with_profiler(
        permissions: Arc<PermissionState>,
        interval_ms: u32,
    ) -> anyhow::Result<Self> {
        Self::new_full(permissions, None, Some(interval_ms), None).await
    }

    async fn new_full(
        permissions: Arc<PermissionState>,
        inspect_addr: Option<SocketAddr>,
        prof_interval_ms: Option<u32>,
        firewall: Option<Arc<Firewall>>,
    ) -> anyhow::Result<Self> {
        let trace = std::env::var_os("VVVA_STARTUP_TRACE").is_some();
        let t0 = std::time::Instant::now();
        ensure_v8_initialized();
        if trace {
            eprintln!("[startup] ensure_v8_initialized: {:?}", t0.elapsed());
        }

        let t1 = std::time::Instant::now();
        let mut isolate = Isolate::new(Default::default());
        if trace {
            eprintln!("[startup] Isolate::new: {:?}", t1.elapsed());
        }
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);

        let timer_manager = TimerManager::new();
        let runtime_core = Mutex::new(Runtime::new((*permissions).clone()));

        let inspector = inspect_addr.map(inspector::start);
        let profiler = prof_interval_ms.map(|_| Profiler::new());

        let ws_pool: builtins::websocket::WsPool =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

        let mut engine = Self {
            isolate,
            context: None,
            _permissions: permissions.clone(),
            timer_manager: timer_manager.clone(),
            runtime_core,
            inspector,
            profiler,
            profiler_interval_ms: prof_interval_ms.unwrap_or(100),
            ws_pool: ws_pool.clone(),
            last_low_memory_hint: std::time::Instant::now(),
            server_mode: false,
        };

        engine.initialize(permissions, timer_manager, firewall, ws_pool)?;

        Ok(engine)
    }

    fn initialize(
        &mut self,
        permissions: Arc<PermissionState>,
        timer_manager: Arc<TimerManager>,
        firewall: Option<Arc<Firewall>>,
        ws_pool: builtins::websocket::WsPool,
    ) -> anyhow::Result<()> {
        let inspector_state = self.inspector.clone();
        let interval_ms = self.profiler_interval_ms;
        let profiler = self.profiler.clone();

        rejection_tracker::install(&mut self.isolate);
        let mut handle_scope_storage = Box::pin(v8::HandleScope::new(&mut *self.isolate));
        let mut handle_scope = handle_scope_storage.as_mut().init();
        let context = v8::Context::new(&handle_scope, Default::default());
        self.context = Some(v8::Global::new(&handle_scope, context));
        let mut scope = v8::ContextScope::new(&mut handle_scope, context);

        let trace = std::env::var_os("VVVA_STARTUP_TRACE").is_some();
        let t = std::time::Instant::now();
        async_context::install(&mut scope, &permissions)?;
        if trace {
            eprintln!("[startup] async_context::install: {:?}", t.elapsed());
        }

        let t = std::time::Instant::now();
        builtins::inject_all(&mut scope, permissions, timer_manager, firewall, ws_pool)?;
        if trace {
            eprintln!("[startup] builtins::inject_all: {:?}", t.elapsed());
        }

        if let Some(state) = inspector_state {
            INSPECTOR_STATE_CELL.set(state).ok();
            let callback = v8::Function::new(
                &mut scope,
                move |_scope: &mut v8::PinScope,
                      _args: v8::FunctionCallbackArguments,
                      _rv: v8::ReturnValue| {
                    let s = INSPECTOR_STATE_CELL.get().unwrap().clone();
                    tokio::task::block_in_place(move || s.pause());
                },
            )
            .unwrap();
            let context = scope.get_current_context();
            let global = context.global(&scope);
            let key = v8::String::new(&scope, "__3va_debugger__").unwrap().into();
            global.set(&scope, key, callback.into());
        }

        if let (Some(js_src), Some(handle)) = (
            profiler
                .as_ref()
                .map(|_| profiler::profiler_js(interval_ms)),
            profiler.clone(),
        ) {
            PROFILER_HANDLE_CELL.set(handle).ok();
            let callback = v8::Function::new(
                &mut scope,
                move |scope: &mut v8::PinScope,
                      args: v8::FunctionCallbackArguments,
                      mut _rv: v8::ReturnValue| {
                    let ts = args.get(0).uint32_value(scope).unwrap_or(0);
                    let stack = args.get(1).to_rust_string_lossy(scope);
                    let label = args.get(2);
                    let lbl = if label.is_null_or_undefined() {
                        None
                    } else {
                        Some(label.to_rust_string_lossy(scope))
                    };
                    PROFILER_HANDLE_CELL
                        .get()
                        .unwrap()
                        .push_raw(ts as u64, &stack, lbl);
                },
            )
            .unwrap();
            let context = scope.get_current_context();
            let global = context.global(&scope);
            let key = v8::String::new(&scope, "__profilerPush").unwrap().into();
            global.set(&scope, key, callback.into());

            let src = v8::String::new(&scope, &js_src).unwrap();
            let _ = v8::Script::compile(&scope, src, None).and_then(|s| s.run(&scope));
        }

        Ok(())
    }

    pub async fn drain_ws_connections(&self) {
        let pool = self.ws_pool.clone();
        tokio::task::spawn_blocking(move || {
            builtins::websocket::drain_ws_pool(&pool, std::time::Duration::from_secs(30));
        })
        .await
        .ok();
    }

    pub async fn eval(&mut self, code: &str) -> anyhow::Result<()> {
        let code = code.to_string();
        let context_global = self.context.clone().expect("engine not initialized");
        let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
        let mut scope = scope.init();
        let context = v8::Local::new(&scope, &context_global);
        let mut scope = v8::ContextScope::new(&mut scope, context);
        v8::tc_scope!(let try_catch, &mut scope);
        let source = v8::String::new(try_catch, &code).unwrap();
        let script = match v8::Script::compile(try_catch, source, None) {
            Some(s) => s,
            None => {
                let text = try_catch
                    .exception()
                    .map(|e| e.to_rust_string_lossy(try_catch))
                    .unwrap_or_else(|| "unknown error".to_string());
                return Err(anyhow::anyhow!("[{}] {text}", EvalPhase::Parse));
            }
        };
        match script.run(try_catch) {
            Some(_) => Ok(()),
            None => {
                let text = try_catch
                    .exception()
                    .map(|e| e.to_rust_string_lossy(try_catch))
                    .unwrap_or_else(|| "unknown error".to_string());
                Err(anyhow::anyhow!("[{}] {text}", EvalPhase::Runtime))
            }
        }
    }

    /// Installs `globalThis.__native_createRealm`, the native primitive
    /// behind test262's `$262.createRealm()`: creates a brand new
    /// `v8::Context` in this same isolate — fresh intrinsics (its own
    /// Array/Object/Error/etc, distinct by identity from the caller's) —
    /// and returns `{ global, evalScript }` for it. Realms are kept alive
    /// in an isolate-slot registry for the engine's lifetime so returned
    /// handles don't dangle once this call returns.
    pub async fn install_test262_realm_support(&mut self) -> anyhow::Result<()> {
        let context_global = self.context.clone().expect("engine not initialized");
        let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
        let mut scope = scope.init();
        let context = v8::Local::new(&scope, &context_global);
        let mut scope = v8::ContextScope::new(&mut scope, context);
        install_realm_support(&mut scope);
        Ok(())
    }

    /// Installs `$262.agent` support for the test262 runner: creates a
    /// fresh `globalThis.__native_agent` object with `start`/`broadcast`/
    /// `getReport`/`sleep`/`monotonicNow` bound to real cross-thread
    /// primitives (see the `$262.agent` module comment near `AgentHub`).
    pub async fn install_test262_agent_support(&mut self) -> anyhow::Result<()> {
        let context_global = self.context.clone().expect("engine not initialized");
        let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
        let mut scope = scope.init();
        let context = v8::Local::new(&scope, &context_global);
        let mut scope = v8::ContextScope::new(&mut scope, context);

        let global = scope.get_current_context().global(&scope);
        let agent_obj = v8::Object::new(&scope);
        install_agent_main(&mut scope, agent_obj);
        let key = v8::String::new(&scope, "__native_agent").unwrap();
        global.set(&scope, key.into(), agent_obj.into());
        Ok(())
    }

    pub async fn eval_to_string(&mut self, code: &str) -> anyhow::Result<String> {
        let code = code.to_string();
        let context_global = self.context.clone().expect("engine not initialized");
        let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
        let mut scope = scope.init();
        let context = v8::Local::new(&scope, &context_global);
        let scope = v8::ContextScope::new(&mut scope, context);
        let source = v8::String::new(&scope, &code).unwrap();
        let script = v8::Script::compile(&scope, source, None)
            .ok_or_else(|| anyhow::anyhow!("compile error"))?;
        let result = script
            .run(&scope)
            .ok_or_else(|| anyhow::anyhow!("execution error"))?;
        Ok(result.to_rust_string_lossy(&scope))
    }

    pub async fn idle(&mut self) {
        pump_v8_platform_tasks(&self.isolate);
        self.isolate.perform_microtask_checkpoint();
    }

    /// Fires any expired `setTimeout`/`setInterval` callbacks a script
    /// registered, running the JS side of each timer. `idle()` only drains
    /// V8 platform tasks and microtasks — it never advances the timer wheel —
    /// so async work that is paced by `setTimeout` (common in
    /// test262 `flags: [async]` cases) needs this called in its own poll
    /// loop to make progress.
    pub async fn pump_timers(&mut self) -> anyhow::Result<()> {
        let tm = self.timer_manager.clone();
        let context_global = self.context.clone().expect("engine not initialized");
        let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
        let mut scope = scope.init();
        let context = v8::Local::new(&scope, &context_global);
        let mut scope = v8::ContextScope::new(&mut scope, context);
        builtins::timers::TimerManager::fire_pending(&mut scope, tm)
    }

    pub async fn with_scope<R>(
        &mut self,
        f: impl FnOnce(&mut v8::ContextScope<v8::HandleScope>) -> R,
    ) -> R {
        let context_global = self.context.clone().expect("engine not initialized");
        let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
        let mut scope = scope.init();
        let context = v8::Local::new(&scope, &context_global);
        let mut scope = v8::ContextScope::new(&mut scope, context);
        f(&mut scope)
    }

    pub async fn eval_file_with_args(
        &mut self,
        path: &Path,
        args: &[String],
    ) -> anyhow::Result<()> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let filename = canonical.to_string_lossy().to_string();
        let script_arg = filename.replace('\\', "\\\\").replace('"', "\\\"");
        let extra: String = args
            .iter()
            .map(|a| format!(", \"{}\"", a.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect();
        let inject = format!(
            "if (globalThis.process && Array.isArray(globalThis.process.argv)) \
             {{ globalThis.process.argv = [globalThis.process.argv[0], \"{script_arg}\"{extra}]; }}"
        );
        self.eval(&inject).await?;
        self.eval_file(path).await
    }

    pub async fn eval_file(&mut self, path: &Path) -> anyhow::Result<()> {
        let source = std::fs::read_to_string(path)?;

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let is_esm = ext == "mjs" || is_esm_source(&source);
        let has_tla = transpiler::has_top_level_await(&source);

        let transpiled: String = if is_esm && has_tla {
            // TLA only valid in ESM — wrap transpiled CJS in async IIFE
            let base_code = transpiler::transpile_to_cjs(&source, matches!(ext, "tsx" | "jsx"));
            transpiler::wrap_with_async_main(&base_code)
        } else if is_esm {
            transpiler::transpile_to_cjs(&source, matches!(ext, "tsx" | "jsx"))
        } else {
            match ext {
                "tsx" | "jsx" => transpiler::transpile_jsx(&source),
                "ts" | "mts" | "cts" => transpiler::transpile(&source),
                _ => {
                    if transpiler::looks_like_jsx(&source) {
                        transpiler::transpile_js(&source)
                    } else {
                        source
                    }
                }
            }
        };

        let code = if self.inspector.is_some() {
            inspector::rewrite_debugger_statements(&transpiled).into_owned()
        } else {
            transpiled
        };

        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let filename = canonical.to_string_lossy().to_string();
        let dirname = canonical
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let meta_url = url::Url::from_file_path(&canonical)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| format!("file://{}", filename.replace('\\', "/")));

        // The file passed to `3va run`/`3va start` is the program the user
        // explicitly chose to execute — even when it physically lives under
        // `node_modules/<pkg>` (e.g. a real npm/yarn `.bin` symlink target
        // like `node_modules/react-native/cli.js`). `__pkgScopeFor()` in
        // modules.rs derives permission scope purely from path shape, so
        // without this it would sandbox the entry script itself as if it
        // were merely a transitively-required dependency of some other app.
        // Record the entry's own node_modules package name (if any) so the
        // require() wrapper can treat that one scope as an alias for "."
        // — every *other* dependency the entry script pulls in is still
        // scoped normally.
        let entry_pkg_scope = entry_package_scope(&dirname);

        let code = transpiler::replace_import_meta(&code);

        {
            let context_global = self.context.clone().expect("engine not initialized");
            let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
            let mut scope = scope.init();
            let context = v8::Local::new(&scope, &context_global);
            let mut scope = v8::ContextScope::new(&mut scope, context);

            let f = filename.replace('\\', "\\\\").replace('\'', "\\'");
            let d = dirname.replace('\\', "\\\\").replace('\'', "\\'");
            let u = meta_url.replace('\\', "\\\\").replace('\'', "\\'");
            let entry_scope_js = match &entry_pkg_scope {
                Some(name) => format!("'{}'", name.replace('\\', "\\\\").replace('\'', "\\'")),
                None => "null".to_string(),
            };

            let setup = format!(
                "globalThis.__vvva_entry_scope = {entry_scope_js};\
             globalThis.__filename = '{f}'; globalThis.__dirname = '{d}';\
             globalThis.__vvva_meta_url__ = '{u}';\
             globalThis.__vvva_meta_env__ = (typeof process !== 'undefined' ? \
                Object.assign(Object.create(null), \
                 {{ MODE: (process.env && process.env.NODE_ENV) || 'production', \
                    PROD: (process.env && process.env.NODE_ENV) !== 'development', \
                    DEV:  (process.env && process.env.NODE_ENV) === 'development', \
                    SSR:  true, \
                    BASE_URL: '/' }}, (typeof globalThis.__vvva_env_raw__ !== 'undefined') \
                       ? globalThis.__vvva_env_raw__ : (process.env || {{}})) : \
                {{ MODE: 'production', PROD: true, DEV: false, SSR: true, BASE_URL: '/' }});\
             if (typeof globalThis.__vvva_meta_resolve__ === 'undefined') \
                globalThis.__vvva_meta_resolve__ = function(s) {{ return require.resolve(s); }};\
             if (typeof globalThis.__vvva_meta_glob__ === 'undefined') \
                globalThis.__vvva_meta_glob__ = function() {{ return {{}}; }};\
             globalThis.__vvva_import_meta__ = {{url: globalThis.__vvva_meta_url__, env: globalThis.__vvva_meta_env__, hot: undefined, glob: globalThis.__vvva_meta_glob__, resolve: globalThis.__vvva_meta_resolve__, require: globalThis.require, dirname: globalThis.__dirname, filename: globalThis.__filename, main: false}};\
             if (globalThis.process && Array.isArray(globalThis.process.argv) \
             && globalThis.process.argv.length < 2) \
             {{ globalThis.process.argv.push('{f}'); }}\
             if (typeof globalThis.require !== 'undefined') {{ \
               var __vvva_entry_module__ = {{ \
                 id: '.', filename: '{f}', loaded: true, \
                 exports: {{}}, parent: null, children: [], paths: [] \
               }}; \
               globalThis.require.main = __vvva_entry_module__; \
               globalThis.module = __vvva_entry_module__; \
             }}",
                f = f,
                d = d,
                u = u,
            );

            let setup_src = v8::String::new(&scope, &setup).unwrap();
            let _ = v8::Script::compile(&scope, setup_src, None).and_then(|s| s.run(&scope));

            let code_src = v8::String::new(&scope, &code).unwrap();
            v8::tc_scope!(let try_catch, &mut scope);
            let script = match v8::Script::compile(try_catch, code_src, None) {
                Some(s) => s,
                None => {
                    let text = try_catch
                        .exception()
                        .map(|e| e.to_rust_string_lossy(try_catch))
                        .unwrap_or_else(|| "unknown error".to_string());
                    return Err(anyhow::anyhow!("[{}] {text}", EvalPhase::Parse));
                }
            };
            if script.run(try_catch).is_none() {
                let text = try_catch
                    .exception()
                    .map(|e| e.to_rust_string_lossy(try_catch))
                    .unwrap_or_else(|| "unknown error".to_string());
                return Err(anyhow::anyhow!("[{}] {text}", EvalPhase::Runtime));
            }
        }

        self.run_event_loop().await?;

        Ok(())
    }

    /// Call before eval_file_with_args for long-running servers (3va dev).
    /// Removes the iteration cap so the event loop runs until process.exit() or SIGINT.
    pub fn set_server_mode(&mut self, enabled: bool) {
        self.server_mode = enabled;
    }

    pub async fn run_event_loop(&mut self) -> anyhow::Result<()> {
        // An open `http.createServer()`/`net.createServer()` listener needs
        // to keep this loop alive indefinitely too, exactly like
        // `server_mode` — "waiting for the next connection" is real pending
        // work even though it never shows up as a timer or task.
        let has_listener = || {
            builtins::http_server::has_active_listeners() || builtins::tcp::has_active_listeners()
        };
        let has_child = builtins::child_process::has_active_children;
        // Bare cap for a script with no listener/child/server_mode at any
        // point — a runaway-timer safety net, not a budget for real work.
        const BOUNDED_MAX_ITERATIONS: usize = 100_000;
        let mut iterations = 0usize;
        let mut last_heartbeat = std::time::Instant::now();

        // A do-while, not a while: callers' eval()/eval_to_string() don't
        // perform their own microtask checkpoint, so this loop's body needs
        // to run at least once per call to flush microtasks queued by
        // synchronous script (e.g. a promise rejected by an AbortController
        // fired before this loop ever saw a pending timer). Looping further
        // is still gated on real pending work, not a hardcoded flag.
        loop {
            iterations += 1;

            let tm = self.timer_manager.clone();
            {
                let context_global = self.context.clone().expect("engine not initialized");
                let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
                let mut scope = scope.init();
                let context = v8::Local::new(&scope, &context_global);
                let mut scope = v8::ContextScope::new(&mut scope, context);
                builtins::timers::TimerManager::fire_pending(&mut scope, tm)?;
            }
            builtins::napi::drain_async_completions();
            pump_v8_platform_tasks(&self.isolate);
            self.isolate.perform_microtask_checkpoint();
            rejection_tracker::report_pending();

            if self.last_low_memory_hint.elapsed() >= LOW_MEMORY_HINT_INTERVAL {
                self.isolate.low_memory_notification();
                self.last_low_memory_hint = std::time::Instant::now();
            }

            let expired = self.runtime_core.lock().unwrap().poll_timers();
            for timer in expired {
                (timer.callback)();
                if timer.repeating
                    && let Some(interval) = timer.interval
                {
                    self.runtime_core
                        .lock()
                        .unwrap()
                        .set_timeout(interval, timer.callback);
                }
            }

            tokio::task::yield_now().await;

            if last_heartbeat.elapsed() >= std::time::Duration::from_secs(10) {
                last_heartbeat = std::time::Instant::now();
                let js_timers = self.timer_manager.pending_count();
                let js_next = self.timer_manager.next_expiry().map(|d| d.as_millis());
                let rust_tasks = self.runtime_core.lock().unwrap().pending_task_count();
                let rust_next = self
                    .runtime_core
                    .lock()
                    .unwrap()
                    .next_timer_duration()
                    .map(|d| d.as_millis());
                eprintln!(
                    "[heartbeat] js_timers={js_timers} js_next={js_next:?}ms rust_tasks={rust_tasks} rust_next={rust_next:?}ms iter={iterations}"
                );
            }

            let next_js = self.timer_manager.next_expiry();
            let next_rust = self.runtime_core.lock().unwrap().next_timer_duration();
            let wait = match (next_js, next_rust) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };

            if let Some(wait) = wait
                && wait > std::time::Duration::ZERO
            {
                tokio::time::sleep(wait.min(std::time::Duration::from_millis(50))).await;
            } else if wait.is_none() && !builtins::napi::has_pending_native_async() {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }

            // Recomputed every iteration — a listener/child/server_mode that
            // only becomes true partway through the script (e.g. a CLI that
            // spawns a build subprocess after several awaited steps) must
            // still lift the iteration cap from here on. Computing this once
            // up front (the old `max_iterations`) let a slow-to-appear child
            // process get silently abandoned once the bounded cap was hit,
            // even while `still_pending` below was reporting real work —
            // exactly the `3va run .../cli.js -- run-android` bug where the
            // whole process exited cleanly mid-Gradle-build.
            let unlimited = self.server_mode || has_listener() || has_child();
            let still_pending = self.timer_manager.has_pending()
                || self.runtime_core.lock().unwrap().pending_task_count() > 0
                || builtins::napi::has_pending_native_async()
                || has_listener()
                || has_child();
            if !still_pending || (!unlimited && iterations >= BOUNDED_MAX_ITERATIONS) {
                break;
            }
        }

        Ok(())
    }

    pub async fn take_profiler(&self) -> Option<Profiler> {
        self.profiler.clone()
    }

    pub fn is_profiling(&self) -> bool {
        self.profiler.is_some()
    }

    pub async fn take_heap_snapshot(&mut self) -> anyhow::Result<String> {
        // V8's own heap profiler — real nodes/edges/strings for every live
        // object, not a hand-rolled stub. It streams the serialized
        // .heapsnapshot JSON (Chrome DevTools format) as byte chunks that
        // just need concatenating.
        let mut buf: Vec<u8> = Vec::new();
        self.isolate.take_heap_snapshot(|chunk| {
            buf.extend_from_slice(chunk);
            true
        });
        Ok(std::string::String::from_utf8(buf)?)
    }
}

/// Mirrors `__pkgScopeFor()` in `builtins/modules.rs`: the innermost
/// `node_modules/<pkg>` (or `node_modules/@scope/pkg`) segment in `dir`, or
/// `None` when `dir` isn't inside any `node_modules` (the app's own code —
/// `__pkgScopeFor` returns `'.'` in that case).
fn entry_package_scope(dir: &str) -> Option<std::string::String> {
    let mut last = None;
    let mut rest = dir;
    while let Some(idx) = rest
        .find("node_modules/")
        .or_else(|| rest.find("node_modules\\"))
    {
        let after = &rest[idx + "node_modules/".len()..];
        let mut seg = after.split(['/', '\\']);
        let first = seg.next().unwrap_or("");
        let pkg = if let Some(name) = first.strip_prefix('@') {
            let second = seg.next().unwrap_or("");
            if second.is_empty() {
                first.to_string()
            } else {
                format!("@{name}/{second}")
            }
        } else {
            first.to_string()
        };
        if !pkg.is_empty() {
            last = Some(pkg);
        }
        rest = after;
    }
    last
}

fn is_esm_source(code: &str) -> bool {
    let mut in_block_comment = false;
    for line in code.lines() {
        let trimmed = line.trim();
        if in_block_comment {
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("/*") {
            in_block_comment = true;
            continue;
        }
        if trimmed.starts_with("import ")
            || trimmed.starts_with("import{")
            || trimmed.starts_with("export ")
            || trimmed.starts_with("export{")
            || trimmed.starts_with("export default")
            || has_dynamic_import(trimmed)
        {
            return true;
        }
    }
    false
}

/// True if `line` contains a bare `import(` call (dynamic import) not
/// preceded by an identifier character — see the matching helper in
/// `esm.rs` for why this is needed: without it, a file using only dynamic
/// `import()` (no static import/export) never gets routed through
/// transpile_to_cjs, so the transpiler's `import(` → `__importAsync(`
/// rewrite never fires and V8 chokes on unsupported native dynamic import.
fn has_dynamic_import(line: &str) -> bool {
    if let Some(pos) = line.find("import(") {
        let before_ok = pos == 0 || {
            let prev = line.as_bytes()[pos - 1];
            !(prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$')
        };
        if before_ok {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_engine_initialization() {
        let permissions = Arc::new(PermissionState::new());
        let engine = JsEngine::new(permissions).await;
        assert!(engine.is_ok(), "Engine failed to initialize");
    }

    #[tokio::test]
    async fn test_engine_evaluation() {
        let permissions = Arc::new(PermissionState::new());
        let mut engine = JsEngine::new(permissions).await.unwrap();

        let result = engine.eval("const x = 1 + 1;").await;
        assert!(result.is_ok());

        let error_result = engine.eval("const x = ;").await;
        assert!(error_result.is_err());
    }
}
