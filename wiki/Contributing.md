# Contributing

## First-time setup

```bash
git clone https://github.com/OdinoCano/3va.git
cd 3va
./scripts/dev-setup.sh   # installs git hooks, verifies tools
```

This installs pre-commit hooks that run `cargo fmt --check` and `cargo clippy` before every commit, and `cargo test` before every push.

## Development workflow

```bash
cargo test          # run full test suite
cargo fmt           # format code
cargo clippy        # check lints
cargo deny check    # check dependencies for CVEs and license issues
```

## CI gates

Every PR must pass all of the following before it can be merged:

| Check | Blocks merge |
|-------|-------------|
| `cargo fmt --check` | Yes |
| `cargo clippy -D warnings` | Yes |
| `cargo test` | Yes |
| `cargo deny check` (advisories + licenses + bans) | Yes |
| Secret scanning (gitleaks) | Yes |
| Semgrep SAST (ERROR severity) | Yes |

**There is no way to bypass CI.** Branch protection on `main` and `develop` requires all status checks to pass and at least one maintainer approval. Even maintainers cannot push directly to `main`.

## Security-sensitive areas

Changes to the following require maintainer review regardless of author (enforced via `CODEOWNERS`):

- `crates/permissions/` — capability model
- `crates/js/src/builtins/` — JS API surface exposed to user code
- `crates/wasm/src/` — WASM sandbox
- `.github/` — CI and security pipelines
- `Cargo.toml`, `Cargo.lock`, `deny.toml` — dependency surface

## Submitting a PR

1. Fork the repository and create a branch from `develop`
2. Make your changes and ensure all CI checks pass locally
3. Open a pull request against `develop` (not `main`)
4. Fill out the PR template

See [[Security]] for reporting vulnerabilities.
