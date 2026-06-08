// Re-export everything from the patched rquickjs-core.
// The dep key "rquickjs-core" (pointing to vvva-rquickjs-core) ensures that
// `rquickjs-macro`-generated code referencing `::rquickjs_core::` paths
// resolves against our patched fork.
pub use rquickjs_core::*;

// Re-export the derive macros from rquickjs-macro.
pub use rquickjs_macro::*;
