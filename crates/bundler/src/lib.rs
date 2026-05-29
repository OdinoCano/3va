//! Module bundler — resolves imports, tree-shakes, and code-generates single-file JS/TS bundles.

pub mod generator;
pub mod resolver;
pub mod tree_shaker;

pub use generator::{BundlerOptions, Chunk, CodeGenerator, CodeSplitter, OutputFormat};
pub use resolver::{ModuleKey, ModuleResolver, ModuleType};
pub use tree_shaker::{DeadCodeEliminator, TreeShaker};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{JsxOptions, JsxRuntime, TransformOptions, Transformer};

#[derive(Debug, Clone)]
pub struct BundlerOutput {
    pub main: String,
    pub chunks: Vec<ChunkOutput>,
}

#[derive(Debug, Clone)]
pub struct ChunkOutput {
    pub name: String,
    pub filename: String,
    pub code: String,
}

pub struct Bundler {
    _resolver: ModuleResolver,
    tree_shaker: TreeShaker,
    code_gen: CodeGenerator,
    modules: HashMap<String, PathBuf>,
    module_deps: HashMap<String, Vec<String>>,
    /// Maps module name → set of export names imported by other modules.
    /// Populated during `bundle()` via AST analysis; used for tree shaking.
    used_exports: HashMap<String, HashSet<String>>,
}

impl Bundler {
    pub fn new(root: PathBuf) -> Self {
        Self {
            _resolver: ModuleResolver::new(root),
            tree_shaker: TreeShaker::new(vec![]),
            code_gen: CodeGenerator::new(BundlerOptions::default()),
            modules: HashMap::new(),
            module_deps: HashMap::new(),
            used_exports: HashMap::new(),
        }
    }

    pub fn with_options(mut self, options: BundlerOptions) -> Self {
        self.code_gen = CodeGenerator::new(options);
        self
    }

    pub fn add_entry(&mut self, entry: &str) -> anyhow::Result<()> {
        let path = PathBuf::from(entry);
        if path.is_file() {
            self.modules.insert(entry.to_string(), path);
            self.tree_shaker.add_entry_point(entry);
        } else {
            anyhow::bail!("Entry file not found: {}", entry);
        }
        Ok(())
    }

    pub fn bundle(&mut self) -> anyhow::Result<String> {
        // Pass 1: process all modules into source code.
        let mut processed: Vec<(String, String)> = Vec::new();
        for (name, path) in &self.modules {
            let code = self.process_module(path)?;
            processed.push((name.clone(), code));
        }

        // Pass 2: analyze named imports across all modules to build used_exports.
        // Key: the raw import path string (e.g. "./utils"); matches module names
        // that were added with the same path via add_entry().
        for (_name, code) in &processed {
            let named = self.tree_shaker.analyze_named_imports(code);
            for (module_path, import_names) in named {
                self.used_exports
                    .entry(module_path)
                    .or_default()
                    .extend(import_names);
            }
        }

        // Pass 3: shake each module and register with the code generator.
        for (name, code) in &processed {
            let deps = self.extract_imports(code);
            self.module_deps.insert(name.clone(), deps);

            let final_code = if let Some(used) = self.used_exports.get(name) {
                self.tree_shaker.shake(name, code, used)
            } else {
                code.clone()
            };
            self.code_gen.add_module(name.clone(), final_code);
        }

        let options = self.code_gen.get_options();
        if options.splitting && self.modules.len() > 1 {
            return self.bundle_with_splitting();
        }

        Ok(self.code_gen.generate())
    }

    pub fn bundle_with_sourcemap(&mut self) -> anyhow::Result<(String, Option<String>)> {
        let mut processed: Vec<(String, String)> = Vec::new();
        for (name, path) in &self.modules {
            let code = self.process_module(path)?;
            processed.push((name.clone(), code));
        }

        for (_name, code) in &processed {
            let named = self.tree_shaker.analyze_named_imports(code);
            for (module_path, import_names) in named {
                self.used_exports
                    .entry(module_path)
                    .or_default()
                    .extend(import_names);
            }
        }

        for (name, code) in &processed {
            let deps = self.extract_imports(code);
            self.module_deps.insert(name.clone(), deps);

            let final_code = if let Some(used) = self.used_exports.get(name) {
                self.tree_shaker.shake(name, code, used)
            } else {
                code.clone()
            };
            self.code_gen.add_module(name.clone(), final_code);
        }

        let (code, map) = self.code_gen.generate_with_sourcemap();
        Ok((code, map))
    }

    fn bundle_with_splitting(&self) -> anyhow::Result<String> {
        let entries: Vec<String> = self.modules.keys().cloned().collect();
        let mut splitter = CodeSplitter::new();
        let chunks = splitter.split(&entries, &self.module_deps);

        let mut output = String::new();
        let format = self.code_gen.get_options().format;

        if format == OutputFormat::Esm {
            for chunk in &chunks {
                let mut chunk_code = String::new();
                for module in &chunk.modules {
                    if let Some(code) = self.code_gen.get_module(module) {
                        chunk_code.push_str(code);
                        chunk_code.push('\n');
                    }
                }
                let filename = format!("{}.js", chunk.name);
                output.push_str(&format!(
                    "// Chunk: {} ({})\nimport './{}';\n\n",
                    chunk.name, filename, filename
                ));
            }
        } else {
            for chunk in &chunks {
                let mut chunk_code = String::new();
                for module in &chunk.modules {
                    if let Some(code) = self.code_gen.get_module(module) {
                        chunk_code.push_str(code);
                        chunk_code.push('\n');
                    }
                }
                output.push_str(&format!(
                    "// ===== Chunk: {} =====\n(function() {{\n{}}})();\n\n",
                    chunk.name, chunk_code
                ));
            }
        }

        Ok(output)
    }

    fn extract_imports(&self, code: &str) -> Vec<String> {
        let mut deps = Vec::new();
        let import_regex = regex_lite::Regex::new(r#"import\s+.*?from\s+['"](.+?)['"]"#).ok();

        if let Some(re) = import_regex {
            for cap in re.captures_iter(code) {
                if let Some(m) = cap.get(1) {
                    deps.push(m.as_str().to_string());
                }
            }
        }

        let require_regex = regex_lite::Regex::new(r#"require\s*\(\s*['"](.+?)['"]\s*\)"#).ok();

        if let Some(re) = require_regex {
            for cap in re.captures_iter(code) {
                if let Some(m) = cap.get(1) {
                    deps.push(m.as_str().to_string());
                }
            }
        }

        deps
    }

    fn process_module(&self, path: &Path) -> anyhow::Result<String> {
        let content = std::fs::read_to_string(path)?;

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        match ext {
            "ts" => Ok(self.strip_types(&content, false)),
            "tsx" => Ok(self.strip_types(&content, true)),
            "js" | "jsx" => Ok(content),
            "json" => Ok(format!("module.exports = {};", content)),
            _ => Ok(content),
        }
    }

    fn strip_types(&self, code: &str, jsx: bool) -> String {
        let allocator = Allocator::default();
        let source_type = if jsx {
            SourceType::tsx()
        } else {
            SourceType::mjs().with_typescript(true)
        };
        let parsed = Parser::new(&allocator, code, source_type).parse();
        if !parsed.errors.is_empty() && parsed.program.body.is_empty() {
            return code.to_string();
        }
        let mut program = parsed.program;
        let scoping = SemanticBuilder::new()
            .build(&program)
            .semantic
            .into_scoping();
        let mut options = TransformOptions::default();
        if jsx {
            options.jsx = JsxOptions {
                jsx_plugin: true,
                runtime: JsxRuntime::Classic,
                pragma: Some("React.createElement".to_string()),
                pragma_frag: Some("React.Fragment".to_string()),
                ..JsxOptions::default()
            };
        }
        let ret = Transformer::new(&allocator, Path::new("input.tsx"), &options)
            .build_with_scoping(scoping, &mut program);
        if !ret.errors.is_empty() && program.body.is_empty() {
            return code.to_string();
        }
        Codegen::new().build(&program).code
    }
}

pub fn bundle_file(
    input: &str,
    output: &str,
    options: Option<BundlerOptions>,
) -> anyhow::Result<()> {
    let root = PathBuf::from(".");
    let mut bundler = Bundler::new(root);

    let sourcemap = options.as_ref().map(|o| o.sourcemap).unwrap_or(false);

    if let Some(opts) = options {
        bundler = bundler.with_options(opts);
    }

    bundler.add_entry(input)?;

    let (code, map) = bundler.bundle_with_sourcemap()?;

    if sourcemap {
        if let Some(map_json) = map {
            let map_path = format!("{}.map", output);
            std::fs::write(&map_path, &map_json)?;
            // Append sourceMappingURL comment to bundle
            let code = format!("{}\n//# sourceMappingURL={}.map\n", code, output);
            std::fs::write(output, code)?;
            tracing::info!("Source map written to {}", map_path);
        } else {
            std::fs::write(output, code)?;
        }
    } else {
        std::fs::write(output, code)?;
    }

    tracing::info!("Bundled {} -> {}", input, output);

    Ok(())
}

/// Start a file-watching build loop. Bundles `input` → `output` immediately,
/// then re-bundles whenever a `.js`, `.ts`, `.jsx`, or `.tsx` file changes
/// under the input's parent directory. Blocks until the process is killed.
pub fn start_watch_mode(
    input: &Path,
    output: &Path,
    options: Option<BundlerOptions>,
) -> anyhow::Result<()> {
    use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    let watch_dir = input.parent().unwrap_or(Path::new("."));

    println!(
        "[bundler] Watch mode: {} → {}",
        input.display(),
        output.display()
    );
    println!("[bundler] Watching: {}", watch_dir.display());

    // Initial build
    do_bundle(input, output, options.clone())?;

    let (tx, rx) = mpsc::channel::<Result<Event, notify::Error>>();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default().with_poll_interval(Duration::from_millis(500)),
    )?;
    watcher.watch(watch_dir, RecursiveMode::Recursive)?;

    let mut last_build = Instant::now();
    let debounce = Duration::from_millis(300);

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(event)) => {
                let is_source = event.paths.iter().any(|p| {
                    matches!(
                        p.extension().and_then(|e| e.to_str()),
                        Some("js" | "ts" | "jsx" | "tsx" | "mjs" | "cjs")
                    )
                });
                let is_modify = matches!(
                    event.kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                );
                if is_source && is_modify && last_build.elapsed() > debounce {
                    println!("\n[bundler] Change detected: {:?}", event.paths);
                    match do_bundle(input, output, options.clone()) {
                        Ok(()) => println!("[bundler] ✓ Rebuilt {}", output.display()),
                        Err(e) => eprintln!("[bundler] ✗ Build error: {}", e),
                    }
                    last_build = Instant::now();
                }
            }
            Ok(Err(e)) => eprintln!("[bundler] Watch error: {}", e),
            Err(_) => {} // recv timeout, keep looping
        }
    }
}

fn do_bundle(input: &Path, output: &Path, options: Option<BundlerOptions>) -> anyhow::Result<()> {
    let input_str = input.to_string_lossy().to_string();
    let output_str = output.to_string_lossy().to_string();
    bundle_file(&input_str, &output_str, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, content: &str) -> String {
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        path.to_string_lossy().into_owned()
    }

    // add_entry rejects a path that does not exist.
    #[test]
    fn add_entry_nonexistent_path_returns_error() {
        let mut b = Bundler::new(PathBuf::from("."));
        assert!(b.add_entry("/no/such/file.js").is_err());
    }

    // add_entry accepts an existing file and registers it.
    #[test]
    fn add_entry_existing_file_registers_module() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "a.js", "const x = 1;");
        let mut b = Bundler::new(dir.path().to_path_buf());
        b.add_entry(&path).unwrap();
        assert!(!b.modules.is_empty());
    }

    // bundle() on a JS file produces non-empty output.
    #[test]
    fn bundle_single_module_produces_output() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "index.js", "var x = 42;");
        let mut b = Bundler::new(dir.path().to_path_buf());
        b.add_entry(&path).unwrap();
        let code = b.bundle().unwrap();
        assert!(!code.is_empty(), "bundle output must not be empty");
        assert!(code.contains("42"), "bundle must include source content");
    }

    // with_options(Cjs) changes the format used during generate().
    #[test]
    fn with_options_changes_output_format() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "index.js", "var x = 1;");

        let mut iife = Bundler::new(dir.path().to_path_buf());
        iife.add_entry(&path).unwrap();
        let iife_code = iife.bundle().unwrap();

        let mut cjs = Bundler::new(dir.path().to_path_buf()).with_options(BundlerOptions {
            format: OutputFormat::Cjs,
            ..BundlerOptions::default()
        });
        cjs.add_entry(&path).unwrap();
        let cjs_code = cjs.bundle().unwrap();

        assert!(
            iife_code.starts_with("(function"),
            "IIFE must start with (function"
        );
        assert!(
            !cjs_code.starts_with("(function"),
            "CJS must not start with (function"
        );
    }

    // extract_imports finds both ESM import and CJS require paths.
    #[test]
    fn extract_imports_finds_esm_and_cjs_deps() {
        let b = Bundler::new(PathBuf::from("."));
        let code = r#"
            import foo from './foo.js';
            const bar = require('./bar.js');
        "#;
        let deps = b.extract_imports(code);
        assert!(
            deps.contains(&"./foo.js".to_string()),
            "ESM import must be detected"
        );
        assert!(
            deps.contains(&"./bar.js".to_string()),
            "CJS require must be detected"
        );
    }

    // process_module wraps JSON as module.exports assignment.
    #[test]
    fn process_module_wraps_json_as_module_exports() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("data.json");
        std::fs::write(&path, r#"{"key":"val"}"#).unwrap();
        let b = Bundler::new(dir.path().to_path_buf());
        let out = b.process_module(&path).unwrap();
        assert!(
            out.starts_with("module.exports ="),
            "JSON must become module.exports"
        );
        assert!(out.contains("\"key\""), "JSON content must be preserved");
    }
}
