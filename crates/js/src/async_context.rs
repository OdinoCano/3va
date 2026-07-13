/// Async context propagation — runtime-level implementation for V8.
///
/// Backed directly by V8's `ContinuationPreservedEmbedderData`: a per-isolate
/// value slot that V8 itself snapshots when a promise reaction is scheduled
/// and restores when that reaction actually runs — the same primitive real
/// Node.js's `AsyncLocalStorage` is built on. That means a plain JS value
/// stored here survives `await`/`.then()` continuations, including
/// concurrent/interleaved chains, with no monkey-patching of `Promise`
/// required (which wouldn't reliably intercept `await` anyway, since it
/// doesn't always go through the exposed `Promise.prototype.then`).
///
/// The JS-level `AsyncLocalStorage`/`AsyncResource` API built on top of these
/// two bindings lives in `builtins/modules.rs` (registered into
/// `__requireCache['async_hooks']`), since `globalThis.__requireCache`
/// doesn't exist yet this early in engine initialization.
use v8::{Function, FunctionCallbackArguments, PinScope, ReturnValue};

pub fn install(
    scope: &mut PinScope,
    _permissions: &std::sync::Arc<vvva_permissions::PermissionState>,
) -> anyhow::Result<()> {
    let acs_get = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              _args: FunctionCallbackArguments<'_>,
              mut rv: ReturnValue<'_>| {
            rv.set(scope.get_continuation_preserved_embedder_data());
        },
    )
    .unwrap();

    let acs_set = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments<'_>,
              _rv: ReturnValue<'_>| {
            scope.set_continuation_preserved_embedder_data(args.get(0));
        },
    )
    .unwrap();

    let context = scope.get_current_context();
    let global = context.global(scope);
    global.set(
        scope,
        v8::String::new(scope, "__acsGet").unwrap().into(),
        acs_get.into(),
    );
    global.set(
        scope,
        v8::String::new(scope, "__acsSet").unwrap().into(),
        acs_set.into(),
    );

    Ok(())
}
