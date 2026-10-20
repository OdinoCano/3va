// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Las trazas de 3va dicen dónde está el error, como las de Node.
//!
//! Node compila el archivo de entrada y cada módulo requerido como una
//! *función* con los parámetros del módulo (`exports, require, module,
//! __filename, __dirname`) y con el nombre del archivo en el `ScriptOrigin`.
//! Por eso la traza dice `archivo:línea:columna` con las posiciones del
//! archivo, y el fotograma del módulo se llama `Object.<anonymous>`.
//!
//! Antes, 3va evaluaba la entrada como script global y compilaba los módulos
//! con `new Function(...)`: las posiciones no eran las del archivo (la columna
//! de la primera línea iba desplazada) y el fotograma del módulo se llamaba
//! `__fn`.
//!
//! Ejecutar: cargo test -p vvva_js --test node_stack_traces

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
async fn the_stack_of_a_required_module_names_the_file_and_the_line() {
    // node 24 dice `at boom (/…/dep.js:2:9)`: el archivo, la línea y la
    // columna del archivo, no las de un envoltorio.
    let (_temp, mut engine) = write_and_eval(
        &[
            (
                "dep.js",
                "function boom() {\n  throw new Error('x');\n}\nmodule.exports = boom;\n",
            ),
            (
                "entry.js",
                "const boom = require('./dep.js');\ntry { boom(); } catch (e) { globalThis._stack = e.stack; }\n",
            ),
        ],
        "entry.js",
    )
    .await;
    let stack = engine
        .eval_to_string("String(globalThis._stack)")
        .await
        .unwrap();
    assert!(
        stack.contains("dep.js:2:"),
        "la traza debe decir el archivo y la línea del módulo: {stack}"
    );
    assert!(
        stack.contains("entry.js:2:"),
        "la traza debe decir el archivo y la línea de la entrada: {stack}"
    );
}

#[tokio::test]
async fn the_frame_of_the_module_is_the_anonymous_function_of_the_module() {
    // Node llama `Object.<anonymous>` al fotograma del módulo CommonJS.
    let (_temp, mut engine) = write_and_eval(
        &[
            ("dep.js", "module.exports = 1;\n"),
            (
                "entry.js",
                "require('./dep.js');\ntry { null.x; } catch (e) { globalThis._stack = e.stack; }\n",
            ),
        ],
        "entry.js",
    )
    .await;
    let stack = engine
        .eval_to_string("String(globalThis._stack)")
        .await
        .unwrap();
    assert!(
        stack.contains("Object.<anonymous>"),
        "el fotograma del módulo debe ser `Object.<anonymous>`: {stack}"
    );
}

#[tokio::test]
async fn the_column_of_the_first_line_is_the_column_of_the_file() {
    // La columna de la primera línea no debe estar desplazada por el
    // envoltorio del módulo.
    let (_temp, mut engine) = write_and_eval(
        &[(
            "entry.js",
            "function boom() { throw new Error('x'); }\ntry { boom(); } catch (e) { globalThis._stack = e.stack; }\n",
        )],
        "entry.js",
    )
    .await;
    let stack = engine
        .eval_to_string("String(globalThis._stack)")
        .await
        .unwrap();
    // node 24: `at boom (/…/entry.js:1:25)`.
    assert!(
        stack.contains("entry.js:1:25"),
        "la columna debe ser la del archivo: {stack}"
    );
}

#[tokio::test]
async fn an_uncaught_error_reports_the_file_the_line_and_the_caret() {
    // Node imprime el archivo y la línea, la línea de código y el `^` debajo,
    // y después el error con su traza.
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("entry.js"), "throw new Error('boom');\n").unwrap();
    let mut engine = engine_with_read(&temp).await;
    let error = engine
        .eval_file(&temp.path().join("entry.js"))
        .await
        .expect_err("el error debe salir del archivo");
    let text = error.to_string();
    assert!(
        text.contains("entry.js:1"),
        "el aviso debe decir el archivo y la línea: {text}"
    );
    assert!(
        text.contains("throw new Error('boom');"),
        "el aviso debe repetir la línea de código: {text}"
    );
    assert!(
        text.contains('^'),
        "el aviso debe señalar la columna con un `^`: {text}"
    );
    assert!(
        text.contains("Error: boom"),
        "el aviso debe llevar el error: {text}"
    );
    assert!(
        text.contains("at "),
        "el aviso debe llevar la traza: {text}"
    );
}
