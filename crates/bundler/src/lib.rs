pub mod generator;
pub mod resolver;
pub mod tree_shaker;

pub use generator::{BundlerOptions, Chunk, CodeGenerator, CodeSplitter, OutputFormat};
pub use resolver::{ModuleKey, ModuleResolver, ModuleType};
pub use tree_shaker::{DeadCodeEliminator, TreeShaker};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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
    #[allow(dead_code)]
    resolver: ModuleResolver,
    #[allow(dead_code)]
    tree_shaker: TreeShaker,
    code_gen: CodeGenerator,
    modules: HashMap<String, PathBuf>,
    module_deps: HashMap<String, Vec<String>>,
    #[allow(dead_code)]
    used_exports: HashMap<String, HashSet<String>>,
}

impl Bundler {
    pub fn new(root: PathBuf) -> Self {
        Self {
            resolver: ModuleResolver::new(root),
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
        } else {
            anyhow::bail!("Entry file not found: {}", entry);
        }
        Ok(())
    }

    pub fn bundle(&mut self) -> anyhow::Result<String> {
        for (name, path) in &self.modules {
            let code = self.process_module(path)?;
            let deps = self.extract_imports(&code);
            self.module_deps.insert(name.clone(), deps);
            self.code_gen.add_module(name.clone(), code);
        }

        let options = self.code_gen.get_options();

        if options.splitting && self.modules.len() > 1 {
            return self.bundle_with_splitting();
        }

        Ok(self.code_gen.generate())
    }

    pub fn bundle_with_sourcemap(&mut self) -> anyhow::Result<(String, Option<String>)> {
        for (name, path) in &self.modules {
            let code = self.process_module(path)?;
            let deps = self.extract_imports(&code);
            self.module_deps.insert(name.clone(), deps);
            self.code_gen.add_module(name.clone(), code);
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
            "ts" | "tsx" => Ok(self.strip_types(&content)),
            "js" | "jsx" => Ok(content),
            "json" => Ok(format!("module.exports = {};", content)),
            _ => Ok(content),
        }
    }

    fn strip_types(&self, code: &str) -> String {
        let mut result = String::new();

        for line in code.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("interface ")
                || trimmed.starts_with("type ")
                || trimmed.contains(": string")
                || trimmed.contains(": number")
                || trimmed.contains(": boolean")
                || trimmed.contains(": void")
                || trimmed.starts_with("// @ts-")
            {
                continue;
            }

            result.push_str(line);
            result.push('\n');
        }

        result
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
pub fn start_watch_mode(input: &Path, output: &Path, options: Option<BundlerOptions>) -> anyhow::Result<()> {
    use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    let watch_dir = input.parent().unwrap_or(Path::new("."));

    println!("[bundler] Watch mode: {} → {}", input.display(), output.display());
    println!("[bundler] Watching: {}", watch_dir.display());

    // Initial build
    do_bundle(input, output, options.clone())?;

    let (tx, rx) = mpsc::channel::<Result<Event, notify::Error>>();
    let mut watcher = RecommendedWatcher::new(
        move |res| { let _ = tx.send(res); },
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

    #[test]
    fn test_bundler_creation() {
        let bundler = Bundler::new(PathBuf::from("."));
        assert!(bundler.modules.is_empty());
    }

    #[test]
    fn test_bundler_options_default() {
        let options = BundlerOptions::default();
        assert!(!options.minify);
        assert_eq!(options.format, OutputFormat::Iife);
    }
}
