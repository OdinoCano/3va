use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{DecoratorOptions, JsxOptions, JsxRuntime, TransformOptions, Transformer};

/// TypeScript/TSX/JSX → JavaScript transpiler backed by the Oxc toolchain.
///
/// Strips TypeScript types, transforms JSX to React.createElement calls, and
/// down-levels modern syntax. On any parse failure the original source is
/// returned unchanged so callers never observe an error.
pub fn transpile(source: &str) -> String {
    transpile_inner(source, false, false)
}

/// Like `transpile` but also enables JSX transform.
pub fn transpile_jsx(source: &str) -> String {
    transpile_inner(source, true, false)
}

/// Try to transpile a `.js` or plain file that may contain JSX/Flow syntax.
///
/// First attempts plain JS parse; if that yields errors, retries with JSX
/// enabled. If both fail, strips Flow annotations and tries again.
pub fn transpile_js(source: &str) -> String {
    // Try plain JS first (handles modern syntax OXC understands).
    let plain = transpile_inner(source, false, false);
    if !plain.is_empty() && plain != source {
        return plain;
    }

    // Retry with JSX enabled — handles `.js` files that contain JSX.
    let with_jsx = transpile_inner(source, true, false);
    if !with_jsx.is_empty() && with_jsx != source {
        return with_jsx;
    }

    // If both failed, strip Flow annotations and try again with JSX.
    // OXC does not support Flow, so we remove common Flow constructs first.
    let flow_stripped = strip_flow(source);
    if flow_stripped != source {
        let flow_jsx = transpile_inner(&flow_stripped, true, false);
        if !flow_jsx.is_empty() && flow_jsx != flow_stripped {
            return flow_jsx;
        }
        // Try inline Flow stripping on the flow-stripped source
        let inlined = strip_inline_flow_types(&flow_stripped);
        if inlined != flow_stripped {
            return inlined;
        }
    }

    // Last resort: aggressively strip inline type annotations (Flow-specific syntax
    // that OXC cannot parse, like `const x: {[string]: boolean} = {}`).
    strip_inline_flow_types(source)
}

/// Strip common Flow type annotations that OXC cannot parse.
/// Best-effort, no-regex implementation using simple string matching.
fn strip_flow(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut i = 0;
    let bytes = source.as_bytes();
    let len = bytes.len();

    while i < len {
        // Skip '@flow' and '@format' in comments (approximate: skip lines containing @flow)
        if bytes[i] == b'@' && i + 4 < len && &bytes[i..i + 5] == b"@flow" {
            // Skip to end of line
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'@' && i + 7 < len && &bytes[i..i + 8] == b"@format" {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Remove `import typeof * as X from '...'` or `import typeof * as X from "..."`
        if i + 14 < len && bytes[i..i + 14].eq_ignore_ascii_case(b"import typeof ") {
            // Skip to end of semicolon or newline at statement level
            let mut j = i;
            while j < len && bytes[j] != b';' && bytes[j] != b'\n' {
                j += 1;
            }
            if j < len && bytes[j] == b';' {
                j += 1; // include semicolon
            }
            i = j;
            continue;
        }

        // Remove `import type {` ... `} from '...'`
        if i + 11 < len && bytes[i..i + 12].eq_ignore_ascii_case(b"import type {") {
            let mut j = i;
            while j < len && bytes[j] != b';' && bytes[j] != b'\n' {
                j += 1;
            }
            if j < len && bytes[j] == b';' {
                j += 1;
            }
            i = j;
            continue;
        }

        // Remove `import type X from '...'`
        if i + 11 < len && bytes[i..i + 12].eq_ignore_ascii_case(b"import type ") {
            let mut j = i;
            while j < len && bytes[j] != b';' && bytes[j] != b'\n' {
                j += 1;
            }
            if j < len && bytes[j] == b';' {
                j += 1;
            }
            i = j;
            continue;
        }

        result.push(bytes[i] as char);
        i += 1;
    }

    result
}

/// Aggressively strip inline Flow type annotations that OXC can't parse.
/// Works line-by-line, matching simple patterns with character-level scanning.
/// Handles:
///   `const x: Type = val` → `const x = val`
///   `function f(p: T): R {` → `function f(p) {`
fn strip_inline_flow_types(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim();

        // Process variable declarations: const/let/var name: Type = value
        if trimmed.starts_with("const ")
            || trimmed.starts_with("let ")
            || trimmed.starts_with("var ")
        {
            let kw_len = if trimmed.starts_with("const ") { 6 } else { 4 };
            let after_kw = &trimmed[kw_len..];
            // Find first colon (type annotation marker)
            if let Some(cp) = after_kw.find(':') {
                let before_colon = &after_kw[..cp];
                // Ensure colon is not after `=` (already past value) or `?` (ternary)
                if !before_colon.contains('=') && !before_colon.contains(')') && cp > 0 {
                    // Find `=` after the colon, respecting brace depth
                    let after_colon = &after_kw[cp + 1..];
                    let mut depth = 0u32;
                    let mut eq_pos = None;
                    for (k, ch) in after_colon.char_indices() {
                        match ch {
                            '{' | '[' | '(' => depth += 1,
                            '}' | ']' | ')' if depth > 0 => depth -= 1,
                            '=' if depth == 0 => {
                                eq_pos = Some(k);
                                break;
                            }
                            _ => {}
                        }
                    }
                    if let Some(ep) = eq_pos {
                        result.push_str(&trimmed[..kw_len + cp]);
                        result.push_str(&after_colon[ep..]);
                        result.push('\n');
                        continue;
                    }
                }
            }
        }

        // Process function declarations: remove param types and return types
        if trimmed.starts_with("function ") {
            let paren = trimmed.find('(');
            if let Some(po) = paren {
                let mut clean = String::with_capacity(trimmed.len());
                clean.push_str(&trimmed[..=po]); // up to and including '('
                let params = &trimmed[po + 1..];
                let mut depth = 0u32;
                let mut i = 0;
                let bytes = params.as_bytes();
                // Build clean params skipping type annotations
                while i < bytes.len() {
                    let ch = bytes[i] as char;
                    match ch {
                        '(' => {
                            depth += 1;
                            clean.push(ch);
                            i += 1;
                        }
                        ')' if depth == 0 => {
                            // End of params — check for return type `: ... {`
                            clean.push(')');
                            i += 1;
                            // Skip return type annotation: `: whatever {`
                            if i < bytes.len() && bytes[i] == b':' {
                                // Find the opening brace of function body
                                let rest = &params[i..];
                                if let Some(brace) = rest.find('{') {
                                    clean.push_str(&rest[brace..]);
                                    i = bytes.len();
                                }
                            } else {
                                clean.push_str(&params[i..]);
                                i = bytes.len();
                            }
                        }
                        ')' => {
                            depth -= 1;
                            clean.push(ch);
                            i += 1;
                        }
                        ':' if depth == 0 && i > 0 && bytes[i - 1].is_ascii_alphabetic() => {
                            // Type annotation colon — skip to next `,` or `)`
                            i += 1;
                            let mut inner_depth = 0u32;
                            while i < bytes.len() {
                                let c = bytes[i] as char;
                                match c {
                                    '{' | '[' | '(' => inner_depth += 1,
                                    '}' | ']' | ')' if inner_depth > 0 => inner_depth -= 1,
                                    ',' | ')' if inner_depth == 0 => {
                                        clean.push(c);
                                        i += 1;
                                        break;
                                    }
                                    _ => {}
                                }
                                i += 1;
                            }
                        }
                        _ => {
                            clean.push(ch);
                            i += 1;
                        }
                    }
                }
                result.push_str(&clean);
                result.push('\n');
                continue;
            }
        }

        result.push_str(line);
        result.push('\n');
    }
    result
}

fn transpile_inner(source: &str, jsx: bool, _flow: bool) -> String {
    try_transpile(source, jsx).unwrap_or_else(|_| source.to_string())
}

fn try_transpile(source: &str, jsx: bool) -> Result<String, ()> {
    let allocator = Allocator::default();

    let source_type = if jsx {
        SourceType::tsx()
    } else {
        SourceType::mjs().with_typescript(true)
    };

    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.errors.is_empty() && parsed.program.body.is_empty() {
        return Err(());
    }

    let mut program = parsed.program;

    let scoping = SemanticBuilder::new()
        .with_enum_eval(true)
        .build(&program)
        .semantic
        .into_scoping();

    // Enable TypeScript legacy decorators (experimentalDecorators + emitDecoratorMetadata)
    // Required by TypeORM, MikroORM, tsyringe, routing-controllers, etc.
    let jsx_opts = if jsx {
        // Use Classic runtime: transforms <Foo /> → React.createElement(Foo, null)
        // This is what React Native and legacy React use.
        JsxOptions {
            jsx_plugin: true,
            runtime: JsxRuntime::Classic,
            pragma: Some("React.createElement".to_string()),
            pragma_frag: Some("React.Fragment".to_string()),
            ..JsxOptions::default()
        }
    } else {
        JsxOptions::default()
    };
    let options = TransformOptions {
        decorator: DecoratorOptions {
            legacy: true,
            emit_decorator_metadata: true,
        },
        jsx: jsx_opts,
        ..TransformOptions::default()
    };

    let ret = Transformer::new(&allocator, std::path::Path::new("input.tsx"), &options)
        .build_with_scoping(scoping, &mut program);

    if !ret.errors.is_empty() && program.body.is_empty() {
        return Err(());
    }

    Ok(Codegen::new().build(&program).code)
}

/// Heuristic: does this source look like it contains JSX elements?
/// Looks for `<Identifier` or `<identifier` not preceded by `<` or `=` (to avoid
/// template literals, generics, and comparison operators).
pub fn looks_like_jsx(source: &str) -> bool {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'<' && i + 1 < len {
            let next = bytes[i + 1];
            // Skip `<<` (bitshift), `<=` (comparison), `<!` (HTML comment, not JSX)
            if next == b'<' || next == b'=' {
                i += 2;
                continue;
            }
            // `</` starts a closing tag — that's JSX
            if next == b'/' {
                return true;
            }
            // `<Letter` — opening JSX tag
            if next.is_ascii_alphabetic() {
                return true;
            }
        }
        // Skip string literals to avoid false positives
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let q = bytes[i];
            i += 1;
            while i < len && bytes[i] != q {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
        }
        i += 1;
    }
    false
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
        let input = "interface User { name: string; age: number; }\nconst x = 1;";
        let output = transpile(input);
        assert!(!output.contains("interface"), "got: {output}");
        assert!(output.contains("const x = 1"), "got: {output}");
    }

    #[test]
    fn test_jsx_classic_transform() {
        let input = "const el = <div className=\"foo\">hello</div>;";
        let output = transpile_jsx(input);
        assert!(
            output.contains("React.createElement"),
            "should use React.createElement, got: {output}"
        );
        assert!(
            !output.contains("<div"),
            "should not contain JSX, got: {output}"
        );
        assert!(
            output.contains("\"foo\""),
            "should keep props, got: {output}"
        );
    }

    #[test]
    fn test_jsx_component_transform() {
        let input = "const el = <MyComponent name=\"test\" />;";
        let output = transpile_jsx(input);
        assert!(output.contains("React.createElement"), "got: {output}");
        assert!(output.contains("MyComponent"), "got: {output}");
    }

    #[test]
    fn test_tsx_strips_types_and_jsx() {
        let input = "const el: JSX.Element = <div id={foo as string}>hi</div>;";
        let output = transpile_jsx(input);
        assert!(output.contains("React.createElement"), "got: {output}");
        assert!(!output.contains(": JSX.Element"), "got: {output}");
    }

    #[test]
    fn test_looks_like_jsx_positive() {
        assert!(looks_like_jsx("<View style={styles.container}>"));
        assert!(looks_like_jsx("return <div />;"));
        assert!(looks_like_jsx("return </div>;"));
        assert!(looks_like_jsx("const x = <MyComp foo={1} />;"));
    }

    #[test]
    fn test_looks_like_jsx_negative() {
        assert!(!looks_like_jsx("const x = a < b ? 1 : 0;"));
        assert!(!looks_like_jsx("const x = a <= b;"));
        assert!(!looks_like_jsx("const s = \"<not jsx>\";"));
    }

    #[test]
    fn test_type_alias_removal() {
        let input = "type StringOrNumber = string | number;\nconst y = 2;";
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
    fn test_preserve_object_literal() {
        let input = "const obj = { key: 'value', count: 42 };";
        let output = transpile(input);
        assert!(output.contains("key"), "got: {output}");
        assert!(output.contains("value"), "got: {output}");
    }

    #[test]
    fn test_fallback_on_invalid_source() {
        let input = "this is not valid ts or js @@##";
        let output = transpile(input);
        assert!(!output.is_empty());
    }

    #[test]
    fn test_import_type_removal() {
        let input = "import type { User } from './types';\nconst x = 1;";
        let output = transpile(input);
        assert!(!output.contains("User"), "got: {output}");
        assert!(output.contains("const x = 1"), "got: {output}");
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
        assert!(output.contains("import"), "got: {output}");
        assert!(output.contains("foo"), "got: {output}");
    }
}
