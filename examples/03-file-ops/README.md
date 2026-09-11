# 03 — File Operations

Read and write files using Node's `fs` module with scoped permissions.

## Run

```bash
3va run app.js --allow-read=./data.json --allow-write=./data.json
```

Run it multiple times to see the data accumulate.

## What it demonstrates

- `--allow-read` and `--allow-write` scoped to a specific file
- Without the flags, any `fs.readFileSync` / `fs.writeFileSync` call is blocked
- Path scoping: only `./data.json` is accessible, not the entire filesystem
