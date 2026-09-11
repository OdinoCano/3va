# 02 — HTTP Server

A minimal HTTP server using Node's built-in `http` module.

## Run

```bash
3va run server.js --allow-net=localhost
```

Then open http://localhost:3000 in your browser.

## Endpoints

| Path | Description |
|------|-------------|
| `/` | Returns a JSON greeting |
| `/time` | Returns current ISO timestamp |
| `*` | 404 |

## What it demonstrates

- Using `--allow-net=localhost` to grant network permission scoped to localhost
- Node.js `http` module compatibility
- Without `--allow-net`, the server would be blocked at runtime
