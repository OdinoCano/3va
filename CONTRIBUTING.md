# Contributing to 3va

Thanks for helping. This guide covers how to build the project, what a pull
request needs before it can be merged, and how it gets reviewed. Everyone
taking part agrees to follow the [Code of Conduct](CODE_OF_CONDUCT.md). How
the project is run is described in [GOVERNANCE.md](GOVERNANCE.md).

## Prerequisites

- Rust stable, 1.85 or newer (the workspace uses edition 2024), installed with [rustup](https://rustup.rs), plus the `rustfmt` and `clippy` components
- A C/C++ toolchain: `build-essential` on Debian/Ubuntu, Xcode Command Line Tools on macOS, or MSVC Build Tools on Windows
- `git`
- Network access on the first build: the `v8` crate downloads a prebuilt static V8 library
- FIPS builds only (`--features fips`): Go and CMake, which AWS-LC needs
- Optional: `cargo-deny`, `cargo-vet`, `cargo-llvm-cov` and `gitleaks`, to run the CI checks locally

## First-time setup

```sh
git clone https://github.com/OdinoCano/3va.git
cd 3va
./scripts/dev-setup.sh   # installs git hooks, verifies tools
cargo build              # debug build → target/debug/3va
```

This installs git hooks:

- **pre-commit** runs `cargo fmt --check`, `cargo clippy` and `gitleaks`.
- **commit-msg** rejects commits without a DCO sign-off.
- **pre-push** runs `cargo test`.

## Development workflow

```sh
cargo test          # run the full test suite (unit, integration and doc tests)
cargo fmt           # format code
cargo clippy        # check lints
cargo deny check    # check dependencies for CVEs and license issues
```

**Tests are required.** Every PR that adds functionality or fixes a bug must
add automated tests covering it: unit tests next to the code, or integration
tests under `crates/*/tests/`. A PR without tests for new behavior will not be
merged.

### When tests run

| When | What runs |
|------|-----------|
| Every `git push` (local hook) | `cargo test` |
| Every push and pull request to `main` (CI, [ci.yml](.github/workflows/ci.yml)) | Every gate listed below, plus a coverage report (`cargo llvm-cov`, report only) |
| Every push and pull request (CI, [security.yml](.github/workflows/security.yml)) | Semgrep SAST and the permission sandbox tests |
| Weekly (CI) | Fuzzing, ASan + UBSan sanitizer runs, and a full dependency audit |

## Developer Certificate of Origin (DCO)

Every commit must be signed off to certify that you wrote it, or otherwise
have the right to submit it under the project's MIT license, as stated in the
[Developer Certificate of Origin 1.1](https://developercertificate.org):

```sh
git commit -s -m "fix(fs): ..."
```

This adds a `Signed-off-by: Your Name <you@example.com>` line that matches your git
identity. The `commit-msg` hook and the **DCO sign-off** CI job reject commits
that don't have one. To fix an existing branch, run `git rebase --signoff main`.

## CI gates: every PR must pass

| Check | Blocks merge |
|---|---|
| `cargo fmt --check` | Yes |
| `cargo clippy -D warnings` | Yes |
| `cargo test` | Yes |
| `cargo deny check` (advisories + licenses + bans + sources) | Yes |
| Secret scanning (gitleaks) | Yes |
| Semgrep SAST (ERROR severity) | Yes |
| CodeQL | Yes, for high and critical alerts |
| `cargo vet` (supply-chain audits) | Yes |
| FIPS build + crypto/TLS tests (`--features fips`) | Yes |
| DCO sign-off on every commit | Yes |

A ruleset on `main` requires a pull request, every status check above, and
an approval from someone other than the author. Direct pushes and force
pushes are blocked. What these checks enforce, and the thresholds they use, is
described in [SECURITY.md § Security findings policy](SECURITY.md#security-findings-policy).

## Code review

Every pull request is reviewed by a maintainer before it is merged. The reviewer checks:

1. **Purpose.** The change is worthwhile and within the project's scope ([roadmap](docs/12-roadmap/01-roadmap.md)).
2. **Correctness.** The logic is right, including edge cases and error paths, and tests cover the new or fixed behavior.
3. **Security.** The change doesn't weaken the deny-by-default permission model. Untrusted input is validated. No new `unsafe` code without a justification. No secrets.
4. **Dependencies.** Any new or updated crate follows the dependency policy below.
5. **Style and docs.** The code reads like the surrounding code. User-facing changes update the docs and `docs/CHANGELOG.md`.

A PR is accepted when every CI gate passes, every review comment is resolved, and it has at least one approving review from someone other than the author.

### Security-sensitive areas

Changes to the following paths need the project lead's review, whoever wrote them (enforced via `CODEOWNERS`):

- `crates/permissions/`: the capability model
- `crates/js/src/builtins/`: the JS API surface exposed to user code
- `crates/wasm/src/`: the WASM sandbox
- `.github/`: CI and security pipelines
- `Cargo.toml`, `Cargo.lock`, `deny.toml`: the dependency surface

## Dependencies

How the project selects, obtains and tracks its dependencies:

- **Selection.** Prefer the standard library, then crates that are already in the tree. A new crate must be actively maintained, carry a license on the `deny.toml` allowlist, and be justified in the PR description.
- **Obtaining.** Dependencies come only from crates.io through Cargo (`cargo-deny` `[sources]` rejects anything else). Versions are pinned in the committed `Cargo.lock`, and release builds use `--locked`. The only patched copies are `vendor/inferno` and `vendor/quick-xml`, declared in `[patch.crates-io]`.
- **Tracking.** `cargo-deny` checks every PR against the RustSec advisory database. `cargo-vet` requires every new or updated crate to be audited or explicitly exempted (`supply-chain/`). Dependabot opens update PRs, and every release ships a CycloneDX SBOM.

## Reporting bugs

Open a [GitHub issue](https://github.com/OdinoCano/3va/issues/new/choose)
using the bug report template. Include the 3va version (`3va --version`), your
OS, the smallest script that reproduces the problem, the command you ran
(including any `--allow-*` flags), and what you expected to happen compared
with what happened.

## Reporting security vulnerabilities

Do **not** open a public issue. Report the vulnerability privately through
[GitHub Security Advisories](https://github.com/OdinoCano/3va/security/advisories/new).
If you can't use GitHub, email `edgarcano.166@gmail.com` instead. See [SECURITY.md](SECURITY.md).
