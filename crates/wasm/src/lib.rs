// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! WebAssembly execution engine — WASI-compatible Wasm runtime for 3va.

#![forbid(unsafe_code)]

pub mod engine;

pub use engine::WasmEngine;
