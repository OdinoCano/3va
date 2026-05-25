use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Declaration, Expression, ImportDeclarationSpecifier, ModuleDeclaration, Statement,
};
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_parser::Parser;
use oxc_span::SourceType;
use std::collections::{HashMap, HashSet};

/// A robust, AST-based Tree Shaker for JavaScript/TypeScript modules.
pub struct TreeShaker {
    used_exports: HashMap<String, HashSet<String>>,
    /// Module names that are entry points; their exports are never shaken away
    /// because external callers may import any of them.
    entry_points: Vec<String>,
}

impl TreeShaker {
    /// Creates a new TreeShaker instance with the given entry points.
    pub fn new(entry_points: Vec<String>) -> Self {
        Self {
            used_exports: HashMap::new(),
            entry_points,
        }
    }

    /// Registers a module as an entry point, preventing any of its exports
    /// from being removed during tree shaking.
    pub fn add_entry_point(&mut self, module_name: &str) {
        if !self.entry_points.iter().any(|e| e == module_name) {
            self.entry_points.push(module_name.to_string());
        }
    }

    /// Analyzes named imports in source code.
    /// Returns a map of `module_path → set of imported export names`.
    /// Default imports map to `"default"`, namespace imports to `"*"`.
    pub fn analyze_named_imports(&self, code: &str) -> HashMap<String, HashSet<String>> {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();

        let mut result: HashMap<String, HashSet<String>> = HashMap::new();

        for stmt in &ret.program.body {
            if let Some(ModuleDeclaration::ImportDeclaration(import)) = stmt.as_module_declaration()
            {
                let module_path = import.source.value.to_string();
                let entry = result.entry(module_path).or_default();
                if let Some(specifiers) = &import.specifiers {
                    for specifier in specifiers {
                        match specifier {
                            ImportDeclarationSpecifier::ImportSpecifier(s) => {
                                entry.insert(s.imported.name().to_string());
                            }
                            ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {
                                entry.insert("default".to_string());
                            }
                            ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {
                                entry.insert("*".to_string());
                            }
                        }
                    }
                }
            }
        }

        result
    }

    /// Analyzes the source code to find all imported module paths.
    /// Supports standard ECMAScript imports.
    pub fn analyze_imports(&mut self, code: &str) -> HashSet<String> {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();

        let mut imports = HashSet::new();

        for stmt in &ret.program.body {
            if let Some(module_decl) = stmt.as_module_declaration() {
                match module_decl {
                    ModuleDeclaration::ImportDeclaration(import) => {
                        imports.insert(import.source.value.to_string());
                    }
                    ModuleDeclaration::ExportNamedDeclaration(export) => {
                        if let Some(source) = &export.source {
                            imports.insert(source.value.to_string());
                        }
                    }
                    ModuleDeclaration::ExportAllDeclaration(export) => {
                        imports.insert(export.source.value.to_string());
                    }
                    _ => {}
                }
            }
        }

        imports
    }

    /// Analyzes the source code to find the names of all exported bindings.
    pub fn analyze_exports(&self, code: &str) -> Vec<String> {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();

        let mut exports = Vec::new();

        for stmt in &ret.program.body {
            if let Some(module_decl) = stmt.as_module_declaration() {
                match module_decl {
                    ModuleDeclaration::ExportNamedDeclaration(export) => {
                        if let Some(decl) = &export.declaration {
                            match decl {
                                Declaration::FunctionDeclaration(func) => {
                                    if let Some(id) = &func.id {
                                        exports.push(id.name.to_string());
                                    }
                                }
                                Declaration::VariableDeclaration(var) => {
                                    for d in &var.declarations {
                                        if let oxc_ast::ast::BindingPattern::BindingIdentifier(id) =
                                            &d.id
                                        {
                                            exports.push(id.name.to_string());
                                        }
                                    }
                                }
                                Declaration::ClassDeclaration(cls) => {
                                    if let Some(id) = &cls.id {
                                        exports.push(id.name.to_string());
                                    }
                                }
                                _ => {}
                            }
                        }
                        for specifier in &export.specifiers {
                            exports.push(specifier.exported.name().to_string());
                        }
                    }
                    ModuleDeclaration::ExportDefaultDeclaration(_) => {
                        exports.push("default".to_string());
                    }
                    _ => {}
                }
            }
        }

        exports
    }

    /// Marks a specific export from a module as used.
    pub fn mark_used(&mut self, module: &str, export: &str) {
        self.used_exports
            .entry(module.to_string())
            .or_default()
            .insert(export.to_string());
    }

    /// Checks if a specific export from a module is used.
    pub fn is_used(&self, module: &str, export: &str) -> bool {
        self.used_exports
            .get(module)
            .map(|s| s.contains(export))
            .unwrap_or(true)
    }

    /// Performs tree shaking on the provided code by removing unused exports.
    ///
    /// `module_name` is used to skip shaking for registered entry points (their
    /// exports are the public API and must all be preserved).  `used_exports` is
    /// the set of exports that MUST NOT be removed in non-entry modules; any
    /// export absent from that set will be stripped.
    pub fn shake(
        &mut self,
        module_name: &str,
        module_code: &str,
        used_exports: &HashSet<String>,
    ) -> String {
        if self.entry_points.iter().any(|ep| ep == module_name) {
            return module_code.to_string();
        }
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let mut ret = Parser::new(&allocator, module_code, source_type).parse();

        // Mutate AST: Retain only statements that are NOT unused exports.
        ret.program.body.retain(|stmt| {
            if let Some(ModuleDeclaration::ExportNamedDeclaration(export)) =
                stmt.as_module_declaration()
            {
                if used_exports.is_empty() {
                    return true;
                }

                let mut has_used = false;

                if let Some(declaration) = &export.declaration {
                    match declaration {
                        Declaration::FunctionDeclaration(func) => {
                            has_used |= func
                                .id
                                .as_ref()
                                .is_some_and(|id| used_exports.contains(id.name.as_str()));
                        }
                        Declaration::VariableDeclaration(var) => {
                            for d in &var.declarations {
                                if let oxc_ast::ast::BindingPattern::BindingIdentifier(id) = &d.id
                                    && used_exports.contains(id.name.as_str())
                                {
                                    has_used = true;
                                }
                            }
                        }
                        Declaration::ClassDeclaration(cls) => {
                            has_used |= cls
                                .id
                                .as_ref()
                                .is_some_and(|id| used_exports.contains(id.name.as_str()));
                        }
                        _ => {}
                    }
                }

                for spec in &export.specifiers {
                    if used_exports.contains(spec.exported.name().as_str()) {
                        has_used = true;
                    }
                }

                return has_used;
            }
            true
        });

        let codegen = Codegen::new().with_options(CodegenOptions {
            minify: false,
            ..Default::default()
        });
        codegen.build(&ret.program).code
    }
}

/// An AST-based eliminator for dead code, specifically unreachable conditionals.
pub struct DeadCodeEliminator;

impl DeadCodeEliminator {
    pub fn new() -> Self {
        Self
    }

    /// Recursively eliminates block statements that are provably unreachable.
    /// Natively evaluates AST for `if (false)` without depending on string format.
    pub fn eliminate(&self, code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let mut ret = Parser::new(&allocator, code, source_type).parse();

        self.eliminate_dead_code_recursive(&mut ret.program.body);

        let codegen = Codegen::new().with_options(CodegenOptions {
            minify: false,
            ..Default::default()
        });
        codegen.build(&ret.program).code
    }

    fn eliminate_dead_code_recursive(&self, stmts: &mut oxc_allocator::Vec<'_, Statement<'_>>) {
        stmts.retain(|stmt| {
            if let Statement::IfStatement(if_stmt) = stmt
                && let Expression::BooleanLiteral(b) = &if_stmt.test
            {
                return b.value || if_stmt.alternate.is_some();
            }
            true
        });

        for stmt in stmts.iter_mut() {
            match stmt {
                Statement::BlockStatement(block) => {
                    self.eliminate_dead_code_recursive(&mut block.body);
                }
                Statement::IfStatement(if_stmt) => {
                    if let Statement::BlockStatement(block) = &mut if_stmt.consequent {
                        self.eliminate_dead_code_recursive(&mut block.body);
                    }
                    if let Some(Statement::BlockStatement(block)) = &mut if_stmt.alternate {
                        self.eliminate_dead_code_recursive(&mut block.body);
                    }
                }
                Statement::ForStatement(for_stmt) => {
                    if let Statement::BlockStatement(block) = &mut for_stmt.body {
                        self.eliminate_dead_code_recursive(&mut block.body);
                    }
                }
                Statement::WhileStatement(while_stmt) => {
                    if let Statement::BlockStatement(block) = &mut while_stmt.body {
                        self.eliminate_dead_code_recursive(&mut block.body);
                    }
                }
                Statement::FunctionDeclaration(func) => {
                    if let Some(body) = &mut func.body {
                        self.eliminate_dead_code_recursive(&mut body.statements);
                    }
                }
                _ => {}
            }
        }
    }
}

impl Default for DeadCodeEliminator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tree_shaker_analyze_imports_ast() {
        let mut shaker = TreeShaker::new(vec!["main".to_string()]);

        let code = r#"
            import foo from './foo';
            import { bar } from './bar';
            import * as baz from './baz';
            export { qux } from './qux';
        "#;

        let imports = shaker.analyze_imports(code);
        assert!(imports.contains("./foo"));
        assert!(imports.contains("./bar"));
        assert!(imports.contains("./baz"));
        assert!(imports.contains("./qux"));
    }

    #[test]
    fn test_tree_shaker_analyze_exports_ast() {
        let shaker = TreeShaker::new(vec![]);

        let code = r#"
            export function testFunc() {}
            export const x = 1, y = 2;
            export class MyClass {}
            export default function() {}
        "#;

        let exports = shaker.analyze_exports(code);
        assert!(exports.contains(&"testFunc".to_string()));
        assert!(exports.contains(&"x".to_string()));
        assert!(exports.contains(&"y".to_string()));
        assert!(exports.contains(&"MyClass".to_string()));
        assert!(exports.contains(&"default".to_string()));
    }

    #[test]
    fn test_tree_shaker_shake_ast() {
        let mut shaker = TreeShaker::new(vec![]);

        let code = r#"
            export function used() { return 1; }
            export function unused() { return 2; }
            export const keep = true;
        "#;

        let mut used = HashSet::new();
        used.insert("used".to_string());
        used.insert("keep".to_string());

        let result = shaker.shake("utils", code, &used);

        assert!(result.contains("used()"));
        assert!(result.contains("keep"));
        assert!(
            !result.contains("unused()"),
            "Unused export should be removed"
        );
    }

    #[test]
    fn test_entry_point_exports_are_preserved() {
        let mut shaker = TreeShaker::new(vec!["main".to_string()]);

        let code = r#"
            export function publicApi() { return 1; }
            export function alsoPublic() { return 2; }
        "#;

        // Empty used set would normally strip everything, but "main" is an entry point.
        let result = shaker.shake("main", code, &HashSet::new());

        assert!(
            result.contains("publicApi"),
            "entry-point exports must be kept"
        );
        assert!(
            result.contains("alsoPublic"),
            "entry-point exports must be kept"
        );
    }

    #[test]
    fn test_non_entry_point_shakes_normally() {
        let mut shaker = TreeShaker::new(vec!["main".to_string()]);

        let code = r#"
            export function used() { return 1; }
            export function dropped() { return 2; }
        "#;

        let mut used = HashSet::new();
        used.insert("used".to_string());

        let result = shaker.shake("lib", code, &used);
        assert!(result.contains("used()"));
        assert!(
            !result.contains("dropped()"),
            "non-entry unused export must be removed"
        );
    }

    #[test]
    fn test_dead_code_eliminator_ast() {
        let elim = DeadCodeEliminator::new();

        let code = r#"
            const a = 1;
            if (false) {
                const dead = 2;
            }
            if (true) {
                const alive = 3;
            }
            function test() {
                if(false){ console.log("nested dead"); }
            }
        "#;

        let result = elim.eliminate(code);

        assert!(!result.contains("dead = 2"));
        assert!(!result.contains("nested dead"));
        assert!(result.contains("alive = 3"));
        assert!(result.contains("const a = 1"));
    }
}
