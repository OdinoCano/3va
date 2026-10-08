// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Cross-engine isolation for the process-wide child/cluster tables.
//!
//! `CHILD_TABLE` and `CLUSTER_TABLE` are process-wide and hand out ids from a
//! global counter, so with more than one `JsEngine` in the same process one
//! engine could previously read, write to, or kill another engine's child just
//! by guessing its id. Entries now carry the owning engine's id and every
//! accessor refuses anything it did not spawn.
//!
//! `JsEngine` is one-per-thread (V8 pinned scopes), so two engines are run on
//! two OS threads and coordinate over channels.

use std::sync::Arc;

use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_spawn() -> JsEngine {
    let state = PermissionState::new();
    state.grant(Capability::SpawnProcess);
    JsEngine::new(Arc::new(state)).await.unwrap()
}

fn current_thread_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn another_engine_cannot_touch_this_engines_child() {
    use std::sync::mpsc;

    let (id_tx, id_rx) = mpsc::channel::<u32>();
    let (b_sees_tx, b_sees_rx) = mpsc::channel::<bool>();
    let (b_done_tx, b_done_rx) = mpsc::channel::<()>();
    let (a_alive_tx, a_alive_rx) = mpsc::channel::<bool>();

    // Engine A spawns a long-lived child and reports its id.
    let a = std::thread::spawn(move || {
        current_thread_runtime().block_on(async move {
            let mut engine = engine_with_spawn().await;
            let id: u32 = engine
                .eval_to_string("__spawnCreate('sleep', ['30'], 'ppp')")
                .await
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            id_tx.send(id).unwrap();

            // Wait until engine B has tried to kill it.
            b_done_rx.recv().unwrap();
            let running = engine
                .eval_to_string(&format!("String(!__spawnIsDone({id}))"))
                .await
                .unwrap();
            engine.eval(&format!("__spawnKill({id})")).await.unwrap();
            a_alive_tx.send(running == "true").unwrap();
        });
    });

    let id = id_rx.recv().unwrap();

    // Engine B, on its own thread, tries to observe and kill A's child.
    let b = std::thread::spawn(move || {
        current_thread_runtime().block_on(async move {
            let mut engine = engine_with_spawn().await;
            let sees = engine
                .eval_to_string(&format!("String(__spawnIsDone({id}))"))
                .await
                .unwrap();
            engine.eval(&format!("__spawnKill({id})")).await.unwrap();
            b_sees_tx.send(sees == "true").unwrap();
            b_done_tx.send(()).unwrap();
        });
    });

    assert!(
        b_sees_rx.recv().unwrap(),
        "engine B could see engine A's child {id} as running"
    );
    assert!(
        a_alive_rx.recv().unwrap(),
        "engine B killed engine A's child {id}"
    );

    a.join().unwrap();
    b.join().unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn an_engine_can_still_operate_on_its_own_child() {
    // Guard against the isolation check being so strict it breaks the owner.
    let mut a = engine_with_spawn().await;
    let id: u32 = a
        .eval_to_string("__spawnCreate('sleep', ['30'], 'ppp')")
        .await
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    let running = a
        .eval_to_string(&format!("String(!__spawnIsDone({id}))"))
        .await
        .unwrap();
    assert_eq!(running, "true");

    a.eval(&format!("__spawnKill({id})")).await.unwrap();
}
