// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! El archivo de entrada se ejecuta como módulo CommonJS, como en Node.
//!
//! Antes de 2.13.0 el archivo de entrada se evaluaba como script global: sus
//! `const`/`let` de nivel superior entraban en el ámbito léxico global (y
//! dejaban en TDZ el `setTimeout` de *todas* las dependencias), y un `return`
//! de nivel superior era un error de sintaxis.
//!
//! Ejecutar: cargo test -p vvva_js --test cjs_entry_module

use std::sync::Arc;
use tempfile::TempDir;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_read(dir: &TempDir) -> JsEngine {
    let state = PermissionState::new();
    state.grant(Capability::FileRead(dir.path().to_path_buf()));
    JsEngine::new(Arc::new(state)).await.unwrap()
}

async fn write_and_eval(files: &[(&str, &str)], entry: &str) -> (TempDir, JsEngine) {
    let temp = TempDir::new().unwrap();
    for (name, content) in files {
        std::fs::write(temp.path().join(name), content).unwrap();
    }
    let mut engine = engine_with_read(&temp).await;
    engine.eval_file(&temp.path().join(entry)).await.unwrap();
    (temp, engine)
}

#[tokio::test]
async fn top_level_const_does_not_shadow_the_globals_of_a_dependency() {
    // `const { setTimeout } = require('timers/promises')` en la entrada dejaba
    // en TDZ el `setTimeout` global: la dependencia que se carga *después*
    // veía `setTimeout is not defined`.
    let (temp, mut engine) = write_and_eval(
        &[
            ("dep.js", "module.exports = typeof setTimeout;"),
            (
                "entry.js",
                r#"
                const r = require("./dep.js");
                const { setTimeout } = require("timers/promises");
                globalThis._dep_sees = r;
                "#,
            ),
        ],
        "entry.js",
    )
    .await;
    let _ = temp;
    let seen = engine
        .eval_to_string("String(globalThis._dep_sees)")
        .await
        .unwrap();
    assert_eq!(
        seen, "function",
        "la dependencia debe ver el `setTimeout` global"
    );
}

#[tokio::test]
async fn top_level_return_is_allowed_in_a_commonjs_entry() {
    // Node permite `return` de nivel superior en un archivo CommonJS.
    let (_temp, mut engine) = write_and_eval(
        &[(
            "entry.js",
            r#"
            globalThis._before = "before";
            if (true) return;
            globalThis._after = "after";
            "#,
        )],
        "entry.js",
    )
    .await;
    let before = engine
        .eval_to_string("String(globalThis._before)")
        .await
        .unwrap();
    let after = engine
        .eval_to_string("String(globalThis._after)")
        .await
        .unwrap();
    assert_eq!(before, "before");
    assert_eq!(after, "undefined", "el `return` debe cortar la ejecución");
}

#[tokio::test]
async fn the_entry_exports_are_the_module_exports() {
    let (_temp, mut engine) = write_and_eval(
        &[(
            "entry.js",
            r#"
            module.exports = { answer: 42 };
            "#,
        )],
        "entry.js",
    )
    .await;
    let answer = engine
        .eval_to_string("String(require('./entry.js').answer)")
        .await
        .unwrap();
    assert_eq!(
        answer, "42",
        "`module.exports` de la entrada debe ser el suyo"
    );
}

#[tokio::test]
async fn require_main_is_the_entry_module() {
    let (_temp, mut engine) = write_and_eval(
        &[(
            "entry.js",
            r#"
            globalThis._main = require.main === module;
            globalThis._id = module.id;
            "#,
        )],
        "entry.js",
    )
    .await;
    let main = engine
        .eval_to_string("String(globalThis._main)")
        .await
        .unwrap();
    let id = engine
        .eval_to_string("String(globalThis._id)")
        .await
        .unwrap();
    assert_eq!(main, "true", "`require.main` debe ser el módulo de entrada");
    assert_eq!(id, ".", "`module.id` de la entrada debe ser '.'");
}

#[tokio::test]
async fn filename_and_dirname_are_the_entry_paths() {
    let (temp, mut engine) = write_and_eval(
        &[(
            "entry.js",
            r#"
            globalThis._filename = __filename;
            globalThis._dirname = __dirname;
            "#,
        )],
        "entry.js",
    )
    .await;
    let filename = engine
        .eval_to_string("String(globalThis._filename)")
        .await
        .unwrap();
    let dirname = engine
        .eval_to_string("String(globalThis._dirname)")
        .await
        .unwrap();
    let expected = temp.path().join("entry.js").canonicalize().unwrap();
    assert_eq!(
        filename,
        expected.to_string_lossy(),
        "`__filename` debe ser la ruta de la entrada"
    );
    assert_eq!(
        dirname,
        expected.parent().unwrap().to_string_lossy(),
        "`__dirname` debe ser el directorio de la entrada"
    );
}

#[tokio::test]
async fn this_is_the_module_exports_at_the_top_level() {
    let (_temp, mut engine) = write_and_eval(
        &[(
            "entry.js",
            r#"
            globalThis._this_is_exports = this === module.exports;
            "#,
        )],
        "entry.js",
    )
    .await;
    let same = engine
        .eval_to_string("String(globalThis._this_is_exports)")
        .await
        .unwrap();
    assert_eq!(same, "true", "en Node, `this` es `module.exports`");
}

#[tokio::test]
async fn top_level_var_does_not_leak_into_global_this() {
    let (_temp, mut engine) = write_and_eval(
        &[(
            "entry.js",
            r#"
            var leaked = 1;
            let leakedLet = 2;
            "#,
        )],
        "entry.js",
    )
    .await;
    let leaked = engine
        .eval_to_string("typeof globalThis.leaked")
        .await
        .unwrap();
    let leaked_let = engine
        .eval_to_string("typeof globalThis.leakedLet")
        .await
        .unwrap();
    assert_eq!(leaked, "undefined", "un `var` de la entrada no es global");
    assert_eq!(
        leaked_let, "undefined",
        "un `let` de la entrada no es global"
    );
}

#[tokio::test]
async fn a_top_level_await_still_awaits() {
    // La entrada con `await` de nivel superior sigue siendo un módulo ESM: el
    // `await` se espera (no se devuelve la promesa, como en 2.12.1).
    let (_temp, mut engine) = write_and_eval(
        &[(
            "entry.mjs",
            r#"
            await Promise.resolve();
            globalThis._awaited = "yes";
            "#,
        )],
        "entry.mjs",
    )
    .await;
    let awaited = engine
        .eval_to_string("String(globalThis._awaited)")
        .await
        .unwrap();
    assert_eq!(
        awaited, "yes",
        "el `await` de nivel superior debe completarse"
    );
}

#[tokio::test]
async fn a_commonjs_entry_does_not_see_a_pending_tdz() {
    // Una dependencia que se carga desde la entrada ve los globals intactos.
    let (_temp, mut engine) = write_and_eval(
        &[
            (
                "dep.js",
                "globalThis._dep_ok = typeof setTimeout === 'function';",
            ),
            ("entry.js", r#"require("./dep.js");"#),
        ],
        "entry.js",
    )
    .await;
    let ok = engine
        .eval_to_string("String(globalThis._dep_ok)")
        .await
        .unwrap();
    assert_eq!(ok, "true");
}
