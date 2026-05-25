use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{TransformOptions, Transformer};

/// TypeScript → JavaScript transpiler backed by the Oxc toolchain.
///
/// Parses TypeScript via Oxc, strips all type-level constructs structurally,
/// then regenerates clean JavaScript. On any parse failure the original source
/// is returned unchanged so callers never observe an error.
pub fn transpile(source: &str) -> String {
    try_transpile(source).unwrap_or_else(|_| source.to_string())
}

fn try_transpile(source: &str) -> Result<String, ()> {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs().with_typescript(true);

    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.errors.is_empty() && parsed.program.body.is_empty() {
        return Err(());
    }

    let mut program = parsed.program;

    let scoping = SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();

    let options = TransformOptions::default();
    let ret = Transformer::new(&allocator, std::path::Path::new("input.ts"), &options)
        .build_with_scoping(scoping, &mut program);

    if !ret.errors.is_empty() && program.body.is_empty() {
        return Err(());
    }

    Ok(Codegen::new().build(&program).code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variable_type_annotation() {
        let input = "const x: string = 'hello';";
        let output = transpile(input);
        assert!(
            !output.contains(": string"),
            "should strip type annotation, got: {output}"
        );
        assert!(
            output.contains("const x"),
            "should keep variable declaration"
        );
        assert!(output.contains("hello"), "should keep value");
    }

    #[test]
    fn test_function_param_types() {
        let input = "function greet(name: string, age: number): void { console.log(name); }";
        let output = transpile(input);
        assert!(!output.contains(": string"), "got: {output}");
        assert!(!output.contains(": number"), "got: {output}");
        assert!(!output.contains(": void"), "got: {output}");
        assert!(output.contains("function greet("), "got: {output}");
    }

    #[test]
    fn test_interface_removal() {
        let input = r#"
interface User {
    name: string;
    age: number;
}
const x = 1;
"#;
        let output = transpile(input);
        assert!(!output.contains("interface"), "got: {output}");
        assert!(!output.contains("User"), "got: {output}");
        assert!(output.contains("const x = 1"), "got: {output}");
    }

    #[test]
    fn test_type_alias_removal() {
        let input = r#"
type StringOrNumber = string | number;
const y = 2;
"#;
        let output = transpile(input);
        assert!(!output.contains("StringOrNumber"), "got: {output}");
        assert!(output.contains("const y = 2"), "got: {output}");
    }

    #[test]
    fn test_as_cast_stripping() {
        let input = "const x = value as string;";
        let output = transpile(input);
        assert!(!output.contains("as string"), "got: {output}");
        assert!(output.contains("const x = value"), "got: {output}");
    }

    #[test]
    fn test_preserve_object_literal() {
        let input = "const obj = { key: 'value', count: 42 };";
        let output = transpile(input);
        assert!(
            output.contains("key"),
            "should keep object key, got: {output}"
        );
        assert!(
            output.contains("value"),
            "should keep object value, got: {output}"
        );
    }

    #[test]
    fn test_preserve_ternary() {
        let input = "const result = a > 0 ? 'positive' : 'negative';";
        let output = transpile(input);
        assert!(output.contains("positive"), "got: {output}");
        assert!(output.contains("negative"), "got: {output}");
        assert!(output.contains("?"), "got: {output}");
    }

    #[test]
    fn test_generic_function() {
        let input = "function identity<T>(x: T): T { return x; }";
        let output = transpile(input);
        assert!(
            !output.contains("<T>"),
            "should strip generic, got: {output}"
        );
        assert!(output.contains("function identity("), "got: {output}");
    }

    #[test]
    fn test_import_type_removal() {
        let input = "import type { User } from './types';\nconst x = 1;";
        let output = transpile(input);
        assert!(!output.contains("User"), "got: {output}");
        assert!(output.contains("const x = 1"), "got: {output}");
    }

    #[test]
    fn test_export_type_removal() {
        let input = "export type { Foo, Bar } from './foo';\nconst z = 3;";
        let output = transpile(input);
        assert!(!output.contains("Foo"), "got: {output}");
        assert!(output.contains("const z = 3"), "got: {output}");
    }

    #[test]
    fn test_access_modifiers() {
        let input = "class Foo { public name: string; private age: number; }";
        let output = transpile(input);
        assert!(!output.contains("public "), "got: {output}");
        assert!(!output.contains("private "), "got: {output}");
        assert!(output.contains("name"), "got: {output}");
    }

    #[test]
    fn test_non_null_assertion() {
        let input = "const x = foo!.bar;";
        let output = transpile(input);
        assert!(!output.contains("!."), "got: {output}");
        assert!(output.contains("foo.bar"), "got: {output}");
    }

    #[test]
    fn test_declare_removal() {
        let input = "declare const x: string;\nconst y = 1;";
        let output = transpile(input);
        assert!(!output.contains("declare"), "got: {output}");
        assert!(output.contains("const y = 1"), "got: {output}");
    }

    #[test]
    fn test_regular_import_preserved() {
        let input = "import { foo } from './bar';\nconst x = foo();";
        let output = transpile(input);
        assert!(
            output.contains("import"),
            "should keep regular import, got: {output}"
        );
        assert!(output.contains("foo"), "got: {output}");
    }

    #[test]
    fn test_string_literals_preserved() {
        let input = r#"const s = "hello: world";"#;
        let output = transpile(input);
        assert!(output.contains("\"hello: world\""), "got: {output}");
    }

    #[test]
    fn test_comments_preserved() {
        let input = "// This is a comment: string\nconst x = 1;";
        let output = transpile(input);
        assert!(output.contains("// This is a comment"), "got: {output}");
        assert!(output.contains("const x = 1"), "got: {output}");
    }

    #[test]
    fn test_fallback_on_invalid_source() {
        let input = "this is not valid ts or js @@##";
        let output = transpile(input);
        assert!(!output.is_empty());
    }
}
