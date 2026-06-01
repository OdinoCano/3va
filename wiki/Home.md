# 3va Wiki

> *Veni, Vidi, Vici, Abiit — He came, he saw, he conquered, he left.*

**3va** is a secure-by-default JavaScript and TypeScript runtime written in Rust. It reimagines the runtime from the ground up with a deny-by-default permission model, post-quantum cryptography, and a sandboxed package manager.

---

## Navigation

| Page | Description |
|------|-------------|
| [[Installation]] | Build from source and install the binary |
| [[CLI Reference]] | All commands and flags |
| [[Permissions]] | Capability-based permission system |
| [[Architecture]] | Crate structure and responsibilities |
| [[Contributing]] | Development workflow and CI gates |
| [[Security]] | Reporting vulnerabilities |

---

## Core Philosophy

- **Deny by default.** No filesystem, network, env, or child-process access unless explicitly granted.
- **Capability-based permissions.** Every sensitive operation requires a flag, scoped to specific hosts, paths, or variables.
- **Untrusted dependencies.** Post-install scripts are never executed.
- **Post-quantum ready.** The `vvva_crypto` crate is built with post-quantum primitives (ML-KEM-768, ML-DSA).

---

## Quick Example

```bash
# Run a script with scoped permissions
3va run app.ts --allow-net=api.example.com --allow-read=/app/config
```

No flag = no access. Simple.
