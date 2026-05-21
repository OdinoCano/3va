#![no_main]

use libfuzzer_sys::fuzz_target;
use vvva_bundler::{BundlerOptions, CodeGenerator, OutputFormat};

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else { return };

    // Fuzz IIFE format
    let mut gen = CodeGenerator::new(BundlerOptions::default());
    gen.add_module("main".to_string(), input.to_string());
    let _ = gen.generate();

    // Fuzz ESM format
    let mut gen_esm = CodeGenerator::new(BundlerOptions {
        format: OutputFormat::Esm,
        minify: true,
        splitting: true,
        ..BundlerOptions::default()
    });
    gen_esm.add_module("entry".to_string(), input.to_string());
    let _ = gen_esm.generate();
});
