// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Re-exports vvva_cli::main so this crate can be published on crates.io
// under a name that doesn't start with a digit.
fn main() {
    vvva_cli::main();
}
