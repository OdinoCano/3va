/// TypeScript type stripper — pure Rust, zero extra dependencies.
///
/// This is a best-effort state-machine based transpiler. It handles the most
/// common TypeScript patterns but is not a full parser. It works by processing
/// the source character-by-character, tracking context (strings, template
/// literals, comments, brace/bracket/paren depth) and removing type-level
/// constructs.
///
/// Handles:
/// - `import type { ... } from '...'` and `export type { ... }` statements
/// - `declare` keyword statements (whole line/block)
/// - `interface Name { ... }` blocks (multiline, tracks brace depth)
/// - `type Name = ...;` alias declarations at statement level
/// - Inline type annotations: `const x: string` → `const x`
/// - `as TypeName` casts: `x as string` → `x`
/// - Access modifiers in class bodies: `public`, `private`, `protected`, `readonly`
/// - `!` non-null assertions: `x!.foo` → `x.foo`
/// - Generic type parameters from function/class signatures: `function f<T>(x: T)` → `function f(x)`
/// - Preserves ternary colons, object literal colons, `case x:` labels

pub fn transpile(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;

    // Track whether we're at start-of-statement (after newline / semicolon)
    // for detecting top-level `interface`, `type`, `declare` keywords.
    let mut at_stmt_start = true;
    // brace depth so we can track class bodies
    let mut brace_depth: i32 = 0;

    while i < len {
        // --- Single-line comment ---
        if chars[i] == '/' && i + 1 < len && chars[i + 1] == '/' {
            // Pass through the rest of the line unchanged
            while i < len && chars[i] != '\n' {
                out.push(chars[i]);
                i += 1;
            }
            at_stmt_start = true;
            continue;
        }

        // --- Multi-line comment ---
        if chars[i] == '/' && i + 1 < len && chars[i + 1] == '*' {
            out.push('/');
            out.push('*');
            i += 2;
            while i < len {
                if chars[i] == '*' && i + 1 < len && chars[i + 1] == '/' {
                    out.push('*');
                    out.push('/');
                    i += 2;
                    break;
                }
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }

        // --- String literals ---
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            out.push(chars[i]);
            i += 1;
            while i < len {
                if chars[i] == '\\' && i + 1 < len {
                    out.push(chars[i]);
                    out.push(chars[i + 1]);
                    i += 2;
                } else if chars[i] == quote {
                    out.push(chars[i]);
                    i += 1;
                    break;
                } else {
                    out.push(chars[i]);
                    i += 1;
                }
            }
            at_stmt_start = false;
            continue;
        }

        // --- Template literal ---
        if chars[i] == '`' {
            out.push('`');
            i += 1;
            let mut depth = 1i32;
            while i < len && depth > 0 {
                if chars[i] == '\\' && i + 1 < len {
                    out.push(chars[i]);
                    out.push(chars[i + 1]);
                    i += 2;
                } else if chars[i] == '`' {
                    out.push('`');
                    i += 1;
                    depth -= 1;
                } else if chars[i] == '$' && i + 1 < len && chars[i + 1] == '{' {
                    out.push('$');
                    out.push('{');
                    i += 2;
                    depth += 1;
                } else {
                    out.push(chars[i]);
                    i += 1;
                }
            }
            at_stmt_start = false;
            continue;
        }

        // --- Statement-level keyword detection ---
        if at_stmt_start {
            // Skip leading whitespace for keyword detection
            if chars[i].is_whitespace() {
                out.push(chars[i]);
                if chars[i] == '\n' {
                    at_stmt_start = true;
                }
                i += 1;
                continue;
            }

            // Try to read a keyword (identifier chars)
            let kw_start = i;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            let kw: String = chars[kw_start..i].iter().collect();

            // `import type { ... } from '...'`
            if kw == "import" || kw == "export" {
                // skip whitespace
                let saved_i = i;
                while i < len && chars[i] == ' ' {
                    i += 1;
                }
                // check for "type"
                let maybe_type_start = i;
                while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let next_kw: String = chars[maybe_type_start..i].iter().collect();

                if next_kw == "type" {
                    // This is `import type` or `export type` — skip until end of statement
                    // (could be on single line ending with ';' or newline, or spanning braces)
                    skip_to_statement_end(&chars, &mut i);
                    // Emit a semicolon placeholder to not break statement parsing
                    out.push('\n');
                    at_stmt_start = true;
                    continue;
                } else {
                    // Not a type import — restore and emit keyword
                    i = saved_i;
                    out.push_str(&kw);
                    at_stmt_start = false;
                    continue;
                }
            }

            // `declare ...` — skip entire statement/block
            if kw == "declare" {
                skip_to_statement_end(&chars, &mut i);
                out.push('\n');
                at_stmt_start = true;
                continue;
            }

            // `interface Name { ... }` — skip entire block
            if kw == "interface" {
                skip_interface_or_type_block(&chars, &mut i);
                out.push('\n');
                at_stmt_start = true;
                continue;
            }

            // `type Name = ...;` — skip type alias (only at statement level)
            if kw == "type" {
                // Make sure the next non-space char isn't `(` (type assertion in JS `type(...)`)
                // and that it looks like a type alias: `type IDENT ...`
                let saved_i = i;
                while i < len && chars[i] == ' ' {
                    i += 1;
                }
                if i < len && (chars[i].is_alphabetic() || chars[i] == '_') {
                    // It's a type alias declaration — skip it
                    skip_to_statement_end(&chars, &mut i);
                    out.push('\n');
                    at_stmt_start = true;
                    continue;
                } else {
                    // Not a type alias (e.g. `type` used as identifier) — restore
                    i = saved_i;
                    out.push_str(&kw);
                    at_stmt_start = false;
                    continue;
                }
            }

            // `abstract class` — strip `abstract` keyword
            if kw == "abstract" {
                // just drop the keyword and the following space
                while i < len && chars[i] == ' ' {
                    i += 1;
                }
                // don't push kw, continue without setting at_stmt_start = false
                // so the next keyword (class) is seen at stmt start
                continue;
            }

            // Access modifiers in class bodies: public/private/protected/readonly/override
            // These appear at statement start inside class bodies
            if (kw == "public" || kw == "private" || kw == "protected"
                || kw == "readonly" || kw == "override")
                && i < len
                && (chars[i] == ' ' || chars[i] == '\t' || chars[i] == '\n')
            {
                // Drop the modifier and continue parsing at stmt start
                while i < len && (chars[i] == ' ' || chars[i] == '\t') {
                    i += 1;
                }
                // Stay at at_stmt_start = true so the next token is parsed correctly
                continue;
            }

            // Emit regular keyword
            out.push_str(&kw);
            if kw.is_empty() {
                // Non-ident character — handle below
                i = kw_start;
            } else {
                at_stmt_start = false;
                continue;
            }
        }

        let ch = chars[i];

        // Track brace depth
        if ch == '{' {
            brace_depth += 1;
            out.push(ch);
            i += 1;
            at_stmt_start = true;
            continue;
        }
        if ch == '}' {
            brace_depth -= 1;
            out.push(ch);
            i += 1;
            at_stmt_start = true;
            continue;
        }

        if ch == ';' {
            out.push(ch);
            i += 1;
            at_stmt_start = true;
            continue;
        }
        if ch == '\n' {
            out.push(ch);
            i += 1;
            at_stmt_start = true;
            continue;
        }

        // --- `!` non-null assertion stripping ---
        // `x!.foo` or `x!` — if `!` appears after an identifier/`)` and before `.`, `[`, `(`, `;`, `,`, `)`, whitespace
        if ch == '!' {
            // Check if this looks like a non-null assertion (not logical not `!value`)
            // Heuristic: if the previous non-space output char was an ident char or `)` or `]`
            let prev = last_non_space_char(&out);
            let next = if i + 1 < len { chars[i + 1] } else { '\0' };
            if (prev.is_alphanumeric() || prev == '_' || prev == ')' || prev == ']')
                && (next == '.' || next == '[' || next == '(' || next == ';' || next == ',' || next == ')' || next == ']' || next == '\n' || next == ' ' || next == '\0')
            {
                // Non-null assertion — skip `!`
                i += 1;
                at_stmt_start = false;
                continue;
            }
            out.push(ch);
            i += 1;
            at_stmt_start = false;
            continue;
        }

        // --- Generic type parameters stripping ---
        // `function f<T, U>(` or `class Foo<T>` — strip `<...>` before `(`
        if ch == '<' {
            // Only strip if previous token was an identifier (function/class generic)
            let prev = last_non_space_char(&out);
            if prev.is_alphanumeric() || prev == '_' {
                // Try to consume a generic parameter block `<...>`
                // Must be balanced and contain only type-looking content
                if let Some(end) = find_closing_angle(&chars, i) {
                    // Skip the generic block
                    i = end + 1;
                    at_stmt_start = false;
                    continue;
                }
            }
            out.push(ch);
            i += 1;
            at_stmt_start = false;
            continue;
        }

        // --- Type annotation stripping: `: TypeExpr` ---
        // After `:`, if we determine this is a type annotation (not object key: value, not
        // ternary ? b : c, not case label), strip until end of annotation.
        if ch == ':' {
            // Look at context to determine if this is a type annotation
            // Heuristics:
            // - If previous meaningful char is `)` — function return type: ): Type
            // - If inside a parameter position (tracking parens)
            // - Pattern: identifer `:` — could be annotation or object key
            //
            // We use a simple heuristic: check if after `:` the content looks like a type
            // (starts with uppercase, or known primitive keywords, or `{`, `[`, `(`)
            // and the colon is NOT preceded by `?` (which would be end of ternary true branch).
            //
            // This is necessarily approximate.
            let prev = last_non_space_char(&out);

            // Definitely NOT a type annotation if prev is `?` (ternary) or nothing
            // Also not type annotation for `case X:` — prev would typically be identifier but
            // we handle that specially
            if prev == '?' {
                // ternary colon
                out.push(ch);
                i += 1;
                at_stmt_start = false;
                continue;
            }

            // Check for `case X:` — scan back in output for "case"
            if is_case_colon(&out) {
                out.push(ch);
                i += 1;
                at_stmt_start = false;
                continue;
            }

            // Check what follows the colon (skip spaces)
            let mut j = i + 1;
            while j < len && chars[j] == ' ' {
                j += 1;
            }

            // If we're at the top level of an object literal `{ key: value }`, don't strip.
            // This is very hard to distinguish from type annotations without full parsing.
            // Our heuristic: if the colon is at brace_depth > 0 and the brace was opened
            // by `{` after `=`, `(`, `return`, then it's an object. Otherwise it's a type.
            //
            // Since we can't track that precisely here, we use the simpler heuristic:
            // strip the annotation only if the next content looks like a type (starts with
            // uppercase or is a known type keyword), AND the previous char was ident/`)`/`]`.

            if j < len {
                let next_ch = chars[j];
                let looks_like_type = is_type_start(next_ch, &chars, j);

                if (prev.is_alphanumeric() || prev == '_' || prev == ')' || prev == ']')
                    && looks_like_type
                {
                    // Strip the `: TypeExpr`
                    // Skip spaces after `:`
                    i += 1; // skip `:`
                    while i < len && chars[i] == ' ' {
                        i += 1;
                    }
                    skip_type_expr(&chars, &mut i);
                    at_stmt_start = false;
                    continue;
                }
            }

            out.push(ch);
            i += 1;
            at_stmt_start = false;
            continue;
        }

        // --- `as TypeExpr` cast stripping ---
        // Identifier `as` followed by a type expression
        // Detect: we just finished reading an expression, see `as` keyword
        if ch == ' ' || ch == '\t' {
            // Peek ahead for `as` keyword
            if i + 3 < len && chars[i + 1] == 'a' && chars[i + 2] == 's' && chars[i + 3] == ' ' {
                // Ensure `as` is at a word boundary
                // Skip the space + `as` + space + TypeExpr
                let saved_i = i;
                i += 4; // skip ` as `
                // skip type expr
                skip_type_expr(&chars, &mut i);
                // Check if what we skipped actually looks like a type
                // (to avoid mangling `as` used as property name etc.)
                // We already consumed it — this is fine for common cases
                let _ = saved_i;
                at_stmt_start = false;
                continue;
            }
            out.push(ch);
            i += 1;
            continue;
        }

        // --- Access modifiers in class bodies: public/private/protected/readonly ---
        // These appear at statement start within class bodies
        // We handle them by stripping the modifier keyword when followed by a space + identifier
        // This is handled via identifier recognition below (not at_stmt_start path)
        if ch.is_alphabetic() || ch == '_' || ch == '$' {
            let id_start = i;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            let ident: String = chars[id_start..i].iter().collect();

            // Strip access modifiers in class bodies
            if (ident == "public" || ident == "private" || ident == "protected" || ident == "readonly")
                && i < len
                && (chars[i] == ' ' || chars[i] == '\t' || chars[i] == '\n')
            {
                // Skip the modifier and following whitespace
                while i < len && (chars[i] == ' ' || chars[i] == '\t') {
                    i += 1;
                }
                at_stmt_start = false;
                continue;
            }

            // `override` keyword (TS 4.3+)
            if ident == "override"
                && i < len
                && (chars[i] == ' ' || chars[i] == '\t')
            {
                while i < len && (chars[i] == ' ' || chars[i] == '\t') {
                    i += 1;
                }
                at_stmt_start = false;
                continue;
            }

            out.push_str(&ident);
            at_stmt_start = false;
            continue;
        }

        // Default: emit character
        out.push(ch);
        at_stmt_start = false;
        i += 1;
    }

    out
}

/// Find the position of the closing `>` for a generic type parameter block `<...>`.
/// Returns None if the content doesn't look like a type parameter (e.g., a comparison).
fn find_closing_angle(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    let len = chars.len();

    // Quick sanity check: generics can't span very large blocks
    // and should only contain type-like content
    let mut suspicious = false;

    while i < len {
        match chars[i] {
            '<' => { depth += 1; i += 1; }
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return if suspicious { None } else { Some(i) };
                }
                i += 1;
            }
            // Things that shouldn't appear in generic type parameters
            '=' | '+' | '-' | '*' | '/' | '!' | '%' | '^' | '&' | '|' | '?' | '\n' => {
                suspicious = true;
                i += 1;
            }
            _ => { i += 1; }
        }
        // If we've gone too far without closing, give up
        if i - start > 256 {
            return None;
        }
    }

    None
}

/// Skip to the end of a TypeScript statement (until `;` or matching `}` for blocks).
fn skip_to_statement_end(chars: &[char], i: &mut usize) {
    let len = chars.len();
    let mut depth = 0i32;

    while *i < len {
        match chars[*i] {
            '{' => { depth += 1; *i += 1; }
            '}' => {
                if depth > 0 {
                    depth -= 1;
                    *i += 1;
                    if depth == 0 {
                        return;
                    }
                } else {
                    // Don't consume the closing brace of an outer block
                    return;
                }
            }
            ';' => {
                if depth == 0 {
                    *i += 1;
                    return;
                }
                *i += 1;
            }
            '\n' => {
                if depth == 0 {
                    *i += 1;
                    return;
                }
                *i += 1;
            }
            '"' | '\'' => {
                let q = chars[*i];
                *i += 1;
                while *i < len {
                    if chars[*i] == '\\' { *i += 2; }
                    else if chars[*i] == q { *i += 1; break; }
                    else { *i += 1; }
                }
            }
            _ => { *i += 1; }
        }
    }
}

/// Skip an `interface` or standalone type block: reads past `{ ... }`.
fn skip_interface_or_type_block(chars: &[char], i: &mut usize) {
    let len = chars.len();
    let mut found_open = false;
    let mut depth = 0i32;

    while *i < len {
        match chars[*i] {
            '{' => {
                found_open = true;
                depth += 1;
                *i += 1;
            }
            '}' => {
                depth -= 1;
                *i += 1;
                if found_open && depth == 0 {
                    return;
                }
            }
            ';' if !found_open => {
                // interface on one line (rare)
                *i += 1;
                return;
            }
            '\n' if !found_open => {
                // no block started, end on newline
                *i += 1;
                return;
            }
            _ => { *i += 1; }
        }
    }
}

/// Skip a TypeScript type expression (after `:`).
/// Handles: primitives, `|`, `&`, `?`, `[]`, `<...>`, `(...)`, `{...}`, arrow functions.
fn skip_type_expr(chars: &[char], i: &mut usize) {
    let len = chars.len();
    let mut depth_paren = 0i32;
    let mut depth_brace = 0i32;
    let mut depth_bracket = 0i32;

    while *i < len {
        match chars[*i] {
            '(' => { depth_paren += 1; *i += 1; }
            ')' => {
                if depth_paren > 0 {
                    depth_paren -= 1;
                    *i += 1;
                } else {
                    break;
                }
            }
            '{' => { depth_brace += 1; *i += 1; }
            '}' => {
                if depth_brace > 0 {
                    depth_brace -= 1;
                    *i += 1;
                } else {
                    break;
                }
            }
            '[' => { depth_bracket += 1; *i += 1; }
            ']' => {
                if depth_bracket > 0 {
                    depth_bracket -= 1;
                    *i += 1;
                } else {
                    break;
                }
            }
            '<' => {
                // Try to consume angle bracket (generic)
                if let Some(end) = find_closing_angle(chars, *i) {
                    *i = end + 1;
                } else {
                    break;
                }
            }
            // Type union/intersection/optional — keep going
            '|' | '&' => { *i += 1; }
            // Fat arrow in type: `() => void` — keep going
            '=' if *i + 1 < len && chars[*i + 1] == '>' => {
                *i += 2;
            }
            // Stop at these when at top level
            ',' | ';' | '=' => {
                if depth_paren == 0 && depth_brace == 0 && depth_bracket == 0 {
                    break;
                }
                *i += 1;
            }
            '\n' => {
                if depth_paren == 0 && depth_brace == 0 && depth_bracket == 0 {
                    break;
                }
                *i += 1;
            }
            // Skip whitespace
            ' ' | '\t' => { *i += 1; }
            // Skip identifier characters (type names)
            c if c.is_alphanumeric() || c == '_' || c == '$' || c == '.' || c == '?' => {
                *i += 1;
            }
            _ => { break; }
        }
    }
    // Skip any trailing whitespace
    while *i < len && chars[*i] == ' ' {
        *i += 1;
    }
}

/// Returns the last non-whitespace character from the output buffer.
fn last_non_space_char(s: &str) -> char {
    s.chars().rev().find(|c| !c.is_whitespace()).unwrap_or('\0')
}

/// Check if the colon at end of `out` is a `case X:` label.
fn is_case_colon(out: &str) -> bool {
    // Look backwards in output for "case" keyword after last newline/semicolon
    let trimmed = out.trim_end();
    // Find last statement boundary
    let last_boundary = trimmed.rfind(|c| c == '\n' || c == ';' || c == '{').unwrap_or(0);
    let stmt = &trimmed[last_boundary..].trim_start_matches(|c: char| c.is_whitespace());
    stmt.starts_with("case ")
}

/// Determine if the character at position `j` in `chars` looks like the start of a type expression.
fn is_type_start(ch: char, chars: &[char], j: usize) -> bool {
    // Known primitive type keywords
    let len = chars.len();

    if ch == '{' || ch == '[' || ch == '(' {
        return true;
    }

    // Read the word starting at j
    let mut end = j;
    while end < len && (chars[end].is_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    if end == j {
        return false;
    }

    let word: String = chars[j..end].iter().collect();

    // Type keywords
    matches!(
        word.as_str(),
        "string" | "number" | "boolean" | "void" | "null" | "undefined"
        | "never" | "any" | "unknown" | "object" | "symbol" | "bigint"
        | "Array" | "Promise" | "Record" | "Map" | "Set" | "Error"
        | "Function" | "RegExp" | "Date" | "HTMLElement" | "Event"
        | "ReadonlyArray" | "Partial" | "Required" | "Readonly"
        | "Pick" | "Omit" | "Exclude" | "Extract" | "NonNullable"
        | "ReturnType" | "InstanceType" | "Parameters" | "ConstructorParameters"
    ) || ch.is_uppercase()   // Capitalized identifier → probably a type
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(s: &str) -> String {
        // Normalize whitespace for comparison: collapse blank lines
        s.lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_variable_type_annotation() {
        let input = "const x: string = 'hello';";
        let output = transpile(input);
        assert!(!output.contains(": string"), "should strip type annotation, got: {output}");
        assert!(output.contains("const x"), "should keep variable declaration");
        assert!(output.contains("'hello'"), "should keep value");
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
        // Object literals should be preserved
        assert!(output.contains("key"), "should keep object key, got: {output}");
        assert!(output.contains("'value'"), "should keep object value, got: {output}");
    }

    #[test]
    fn test_preserve_ternary() {
        let input = "const result = a > 0 ? 'positive' : 'negative';";
        let output = transpile(input);
        assert!(output.contains("'positive'"), "got: {output}");
        assert!(output.contains("'negative'"), "got: {output}");
        assert!(output.contains("?"), "got: {output}");
    }

    #[test]
    fn test_generic_function() {
        let input = "function identity<T>(x: T): T { return x; }";
        let output = transpile(input);
        assert!(!output.contains("<T>"), "should strip generic, got: {output}");
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
        assert!(output.contains("import"), "should keep regular import, got: {output}");
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
}
