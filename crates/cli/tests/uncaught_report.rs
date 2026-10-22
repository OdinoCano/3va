// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! El aviso de una excepción no capturada y el de `process.emitWarning` salen
//! por stderr con la forma de Node.
//!
//! Node imprime el archivo y la línea, la línea de código, el `^` debajo y la
//! traza; y el aviso de `process.emitWarning` sale como
//! `(node:PID) [CÓDIGO] Tipo: mensaje`.
//!
//! Estos tests ejecutan el binario de verdad, porque lo que se comprueba es
//! lo que ve el usuario en su terminal.
//!
//! Ejecutar: cargo test -p vvva_cli --test uncaught_report

use std::path::Path;
use std::process::Command;

fn run(script: &Path, extra: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_3va"))
        .arg("run")
        .arg("--no-prompt")
        .arg(script)
        .arg(format!(
            "--allow-read={}",
            script.parent().unwrap().display()
        ))
        .args(extra)
        .output()
        .expect("failed to run `3va run`");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn write_script(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn an_uncaught_error_prints_the_file_the_line_the_code_and_the_caret() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        "boom.js",
        "function boom() { throw new Error('boom'); }\nboom();\n",
    );
    let (_stdout, stderr, code) = run(&script, &[]);
    assert_ne!(code, 0, "un error sin capturar debe salir con código ≠ 0");
    assert!(
        stderr.contains("boom.js:2"),
        "el aviso debe decir el archivo y la línea: {stderr}"
    );
    assert!(
        stderr.contains("function boom() { throw new Error('boom'); }"),
        "el aviso debe repetir la línea de código: {stderr}"
    );
    assert!(
        stderr.contains('^'),
        "el aviso debe señalar la columna con un `^`: {stderr}"
    );
    assert!(
        stderr.contains("Error: boom"),
        "el aviso debe llevar el error: {stderr}"
    );
    assert!(
        stderr.contains("at boom"),
        "el aviso debe llevar la traza: {stderr}"
    );
}

#[test]
fn a_required_plain_module_keeps_its_own_line_and_text() {
    let dir = tempfile::tempdir().unwrap();
    // Line 1, column 40: the loader must compile the file as it is, not the
    // transpiler's re-print of it (which would say line 3, column 8).
    let lib = "                                       throw new Error('kaput');\n";
    write_script(dir.path(), "lib.js", lib);
    let script = write_script(dir.path(), "main.js", "require('./lib.js');\n");
    let (_stdout, stderr, code) = run(&script, &[]);
    assert_ne!(code, 0, "un error sin capturar debe salir con código ≠ 0");
    assert!(
        stderr.contains("lib.js:1"),
        "el aviso debe dar la línea del archivo: {stderr}"
    );
    assert!(
        stderr.contains("throw new Error('kaput');"),
        "el aviso debe repetir la línea tal como está escrita: {stderr}"
    );
    assert!(
        stderr.contains("lib.js:1:46"),
        "la traza debe dar la línea y la columna del archivo: {stderr}"
    );
}

#[test]
fn emit_warning_prints_on_stderr_with_the_type_of_the_warning() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        "warn.js",
        "process.emitWarning('hello', 'DeprecationWarning');\n",
    );
    let (_stdout, stderr, _code) = run(&script, &[]);
    assert!(
        stderr.contains("DeprecationWarning: hello"),
        "el aviso debe imprimirse con su tipo: {stderr}"
    );
}

#[test]
fn emit_warning_with_a_code_prints_the_code() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        "warn2.js",
        "process.emitWarning('hello', { type: 'DeprecationWarning', code: 'DEP0005' });\n",
    );
    let (_stdout, stderr, _code) = run(&script, &[]);
    assert!(
        stderr.contains("[DEP0005] DeprecationWarning: hello"),
        "el aviso debe imprimirse con su código: {stderr}"
    );
}

#[test]
fn no_warnings_silences_the_print_but_not_the_event() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        "warn3.js",
        "process.on('warning', (w) => console.log('EVENT ' + w.name + ' ' + w.message));\n\
         process.emitWarning('hello', 'DeprecationWarning');\n",
    );
    let (stdout, stderr, _code) = run(&script, &["--no-warnings"]);
    assert!(
        !stderr.contains("DeprecationWarning: hello"),
        "con `--no-warnings` el aviso no se imprime: {stderr}"
    );
    assert!(
        stdout.contains("EVENT DeprecationWarning hello"),
        "con `--no-warnings` el evento sigue llegando: {stdout}"
    );
}
