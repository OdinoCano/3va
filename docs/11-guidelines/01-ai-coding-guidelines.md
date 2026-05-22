# 01 - DEVELOPMENT GUIDELINES FOR AI (AI GUIDELINES)

Any Artificial Intelligence system (LLMs, Code Assistants) or human developer generating code for the **3va** project MUST strictly adhere to the following rules, designed to comply with the European **EN 301 549** Accessibility directive.

## 1. Golden Rule: Terminal Accessibility by Default

The `3va` runtime is used by developers with visual impairments who depend on *Braille Displays* and *Screen Readers*. The CLI output must respect the global accessibility module `crates/cli/src/accessibility.rs`.

### 1.1 Excessive ASCII Art Prohibited
If the CLI needs to print boxes, tables or decorative elements, the AI MUST wrap such logic in a check of the `--accessible` flag.
Characters like `┌`, `─`, `│`, `└` overwhelm Braille hardware, which will read them literally.
- **Incorrect:** Always print tables drawn with bars.
- **Correct:** Check `accessibility::is_accessible_mode()` and if true, print a flat list.

### 1.2 Careful with Animations (Spinners/Progress bars)
When creating commands that take time (e.g. `3va install`, `3va bundle`), it is forbidden to send animations that depend on continuous carriage returns (`\r`) if accessible mode is active.
- A continuous carriage return freezes the Braille line.
- In accessible mode, print static messages: `INFO: Downloading...` and `INFO: Download complete`.

### 1.3 Colors are NOT Exclusive Semantics
The logging system (`tracing`) is already configured to disable ANSI codes (`NO_COLOR`) in accessible mode. Therefore, the AI must never use "colors" as the only way to denote a status (e.g. painting text red assuming the user will understand it is an error).
- **Mandatory:** Always prepend clear text prefixes like `ERROR:`, `WARN:`, `SUCCESS:`.

## 2. Implementation Example

When adding logic to `main.rs` or CLI commands:

```rust
// CORRECT
let is_accessible = accessibility::is_accessible_mode(cli.accessible);

if is_accessible {
    println!("INFO: Compiling project...");
} else {
    // Here you can use spinners, indicatif crate, or ASCII tables
    start_spinner("Compiling...");
}
```

> **AI Attention**: If the user asks you to "make the console prettier" or "add a progress bar", you **must** respect these guidelines and include the `if` branch for accessible mode.
