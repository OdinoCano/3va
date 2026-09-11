# 06 — Dev Server with HMR

A tiny task-list app served by 3va's built-in dev server.

## Run

```bash
3va dev --port 3000
```

Open http://localhost:3000 in your browser.

## What it demonstrates

- On-demand ESM serving (Vite-style): `index.html` + `<script type="module">`
- JSX/TS transpiled per request
- Full-page HMR via Server-Sent Events: edit `src/main.js` and save to see it reload
- Files under `node_modules` resolve via `/@fs/<path>`
- `.css` imports are style-injected automatically