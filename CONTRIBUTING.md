# Contributing to 3va

## First-time setup

```sh
git clone <repo>
cd 3va
./scripts/dev-setup.sh   # installs git hooks, verifies tools
```

This installs pre-commit hooks that run `cargo fmt --check` and `cargo clippy` before every commit, and `cargo test` before every push.

## Development workflow

```sh
cargo test          # run full test suite
cargo fmt           # format code
cargo clippy        # check lints
cargo deny check    # check dependencies for CVEs and license issues
```

## CI gates — every PR must pass

| Check | Blocks merge |
|---|---|
| `cargo fmt --check` | Yes |
| `cargo clippy -D warnings` | Yes |
| `cargo test` | Yes |
| `cargo deny check` (advisories + licenses + bans) | Yes |
| Secret scanning (gitleaks) | Yes |
| Semgrep SAST (ERROR severity) | Yes |

**There is no way to bypass CI.** Branch protection on `main` and `develop` requires all status checks to pass and at least one maintainer approval before merge. Even maintainers cannot push directly to `main`.

## Security-sensitive areas

Changes to the following require maintainer review regardless of who authored them (enforced via `CODEOWNERS`):

- `crates/permissions/` — capability model
- `crates/js/src/builtins/` — JS API surface exposed to user code
- `crates/wasm/src/` — WASM sandbox
- `.github/` — CI and security pipelines
- `Cargo.toml`, `Cargo.lock`, `deny.toml` — dependency surface

## Reporting security vulnerabilities

Do **not** open a public issue. Email `security@sophava.com` with a description and reproduction steps.
