# 03 - REPL Plugin API

## 3.1 Overview

v2.0.0 introduces a plugin system for the sandbox REPL. Plugins can add custom dot-commands, tab-completion providers, and startup banners.

---

## 3.2 Loading Plugins

```bash
3va sandbox --plugin ./my-plugin.ts
3va sandbox --plugin ./inspect-plugin.ts --plugin ./history-plugin.ts
```

Multiple `--plugin` flags are allowed. Plugins are loaded in order; conflicting command names are rejected with an error at startup.

---

## 3.3 Plugin Interface

```ts
// Plugin contract (TypeScript)
export interface ReplContext {
  eval(code: string): Promise<unknown>;
  print(text: string): void;
  permissions: {
    grantRead(path: string): void;
    grantNet(host: string): void;
    list(): string[];
  };
}

export interface ReplPlugin {
  /** Commands registered as .name in the REPL */
  commands?: Record<string, (args: string, ctx: ReplContext) => void | Promise<void>>;

  /** Tab-completion providers. Return candidate completions for the current line. */
  completers?: Array<(line: string, ctx: ReplContext) => string[]>;

  /** Text printed after the built-in REPL banner */
  banner?: string;
}

/** Default export must be the plugin object or a factory function */
export default {
  commands: {
    'hello': (_args, ctx) => ctx.print('Hello from my plugin!'),
  },
  banner: 'my-plugin loaded',
} satisfies ReplPlugin;
```

---

## 3.4 Built-in Plugins

Two plugins are bundled with 3va and available without a path:

### `--plugin=inspect`

Overrides the default value printer with a colorized, recursive object inspector (similar to Node.js `util.inspect`). Circular references are detected and printed as `[Circular]`.

```
3va sandbox --plugin=inspect
> { a: 1, b: [1, 2, 3], c: { nested: true } }
{ a: 1, b: [ 1, 2, 3 ], c: { nested: true } }
```

### `--plugin=history`

Saves REPL input history to `~/.3va/repl_history` (one entry per line, newest at bottom). History is loaded on startup and navigable with arrow keys. The file is capped at 10,000 lines.

```bash
3va sandbox --plugin=history
```

---

## 3.5 Permissions

Plugins are loaded under the same sandbox as the REPL itself. A plugin cannot grant itself permissions beyond what was passed to `3va sandbox` on the command line. The `ctx.permissions.grantRead` / `ctx.permissions.grantNet` methods mirror the existing `.allow-read` / `.allow-net` dot-commands. 

To prevent silent sandbox escapes, any programmatic call to `ctx.permissions.grant*` that is not already covered by the CLI startup flags **must trigger the standard interactive TTY confirmation prompt** to the user. A plugin cannot silently acquire privileges.

---

## 3.6 Implementation Notes

- Plugins are loaded as ES modules via the existing `EsmLoader`.
- Plugin file paths require `--allow-read=<path>` if outside the current directory.
- The `ReplContext` object is a Rust-backed JS object; its methods call into `PermissionState` directly.
- Plugin execution errors (at load time or command invocation time) are caught and printed without crashing the REPL.
