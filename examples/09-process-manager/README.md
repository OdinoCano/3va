# 09 — Process Manager

Run a Node-style HTTP server as a background daemon with 3va's built-in process manager — no pm2.

## Start (background daemon with auto-restart)

Permissions for a managed process come from `package.json` (there are no `--allow-*` flags on `3va start`), so this example ships a `package.json` that grants `allow-net`:

```bash
3va start server.js --name demo-api
```

## Manage it

```bash
3va status                 # list all managed processes
3va status demo-api        # one process
3va logs demo-api          # tail logs
3va restart demo-api
3va stop demo-api          # SIGTERM → SIGKILL after 1.5s
3va delete demo-api        # stop + remove logs
```

## Test auto-restart

`server.js` intentionally crashes every 30 requests. Watch the supervisor respawn it with exponential backoff (500 ms → 1 s → 2 s → … → capped 30 s):

```bash
for i in $(seq 1 35); do curl -s localhost:4000; echo; done
```

For a one-shot process that never respawns: add `--no-autorestart`.

## What it demonstrates

- Built-in process manager (no pm2, no external daemon)
- Auto-restart with exponential backoff
- Logs and metadata in `~/.3va/processes/`
- Managed processes keep the exact permissions they were started with