# 08 — Bundler

Bundle a multi-file app into a single self-contained JavaScript file.

## Run

Bundle the project:

```bash
3va bundle src/index.js -o dist/bundle.js --minify
```

Run the bundle (works standalone — no node_modules needed at runtime):

```bash
3va run dist/bundle.js
```

Or serve it in the browser via the dev server:

```bash
3va dev
```

(with the bundled output at http://localhost:3000/bundle.js and the page at `public/index.html`)

## What it demonstrates

- Walks the real import graph (project files + `node_modules`, both ESM and CommonJS)
- Inlines `.json` and `.css` imports (`.css` injected as a `<style>` tag in the browser)
- `--minify` strips whitespace and shortens names
- Output runs standalone via `3va run` or as a browser `<script>`

> **Note:** `--source-map` and `--split` are not yet implemented for this path.