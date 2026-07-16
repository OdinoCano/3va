# Android arm64 fallback branch — AI handoff prompt

This branch (`android-pre-v8`) is a snapshot of `main` at commit `fda229f`,
the last commit **before** the JS engine migration from rquickjs (QuickJS) to
V8 (`c763958`, "Migrate JS engine from rquickjs to V8..."). It exists purely
as a fallback/reference point for Android arm64 support, which worked under
QuickJS and does not work under V8. It is not meant to receive ongoing
feature development — `main` (V8) is the active line.

Paste the section below into a fresh AI coding session to bring it up to
speed on why this branch exists and what's known.

---

## Prompt: catch up on the 3va Android arm64 build situation

You're working on **3va**, a secure-by-default JavaScript/TypeScript runtime
(Rust workspace, binary name `3va`, 11 crates under `crates/`). I need you to
understand a specific, already-diagnosed platform-support problem before
doing anything else.

**Background:** `main` migrated 3va's JS engine from rquickjs (QuickJS) to
V8 in commit `c763958`. That migration was never validated against a full
6-platform release build until the `v2.4.0` release attempt (2026-07-16),
which revealed it silently broke two targets in
`.github/workflows/release.yml`'s build matrix:

1. **`aarch64-unknown-linux-gnu` (cross-compiled via `cross`)** — FIXED on
   `main`. Root cause: `cross`'s default Docker image for this target pins
   an old glibc lacking `memfd_create` (needed by V8's static lib, which
   assumes glibc 2.27+). Fix: a `Cross.toml` at the repo root pinning the
   image to the `:main` tag (a recent rolling build with modern glibc)
   instead of the stale default. Verified working via a `workflow_dispatch`
   test run (`test_only: true` input added to `release.yml` specifically so
   this could be tested without cutting a real release).

2. **`aarch64-linux-android`** — NOT fixable on the V8 side without patching
   third-party source. Two independent problems stack here:
   - `denoland/rusty_v8` (V8's Rust binding) ships no prebuilt Android
     binaries at all (see `rusty_v8#1640`) — dropped, not planned to return.
   - The fallback, building V8 from source via `V8_FROM_SOURCE=1` under
     `cargo-ndk`, was tried and fails with a **genuine bug in the `v8` crate
     itself**: its vendored/trimmed copy of Chromium's
     `build/config/android/config.gni` never declares `android_ndk_version`
     via `declare_args()`, but `build/config/android/BUILD.gn:37` references
     it — a GN "Undefined identifier" error, unconditional, not something
     fixable by passing GN args from our side (GN requires a variable to be
     `declare_args()`-declared somewhere before it can be referenced or
     overridden). This is upstream incompleteness in `rusty_v8`'s vendored
     build files, not a 3va bug and not a config mistake in our workflow.

   Full session diagnosis is in the memory file `v8_migration_platform_gaps`
   (if you have access to project memory) and in `main`'s git history around
   the `v2.4.0` release attempt (commits touching `release.yml`,
   `Cross.toml`, `crates/js/src/builtins/os_info.rs`'s Windows
   `gethostname` fix, and the `v8` crate lockfile pin to `150.0.0`).

**Why this branch exists:** before the V8 migration, Android arm64 built
fine under QuickJS — see commit `41c4174` ("fix: add pre-built QuickJS
bindings for aarch64-linux-android") on this branch's own history. This
branch (`android-pre-v8`, at `fda229f`) is a preserved fallback snapshot in
case a decision is made to special-case Android on QuickJS while the rest of
the platform matrix stays on V8, or to reference exactly how QuickJS's
Android build was configured (NDK setup, `cross`/`cargo-ndk` steps, patched
`rquickjs-sys`/`vvva-rquickjs` vendored crates under whatever path they lived
in at that commit).

**What is NOT decided yet:** whether to (a) patch/vendor a fixed copy of the
`v8` crate's `build/config/android/config.gni` to add the missing
`declare_args()` block, (b) drop `aarch64-linux-android` from the release
matrix on `main` as a documented, deliberate gap, or (c) maintain Android
specifically off this QuickJS-era branch as a separate release channel. The
last explicit direction from the project owner, mid-investigation, was to
preserve this branch before making that call — no path has been chosen yet.

**Your task:** pick up from here. Read `docs/12-roadmap/06-pm-feature-parity.md`
and any memory files under the project's memory directory for full context on
concurrent work (a large PM feature-parity effort landed on `main` around the
same time and is unrelated to this Android issue but shares the same release
cycle). Do not merge this branch into `main` or make irreversible decisions
(deleting this branch, dropping Android from the matrix, tagging a release)
without explicit confirmation — this mirrors the standing operating principle
for this project: hard-to-reverse or shared-state actions require a
confirmation step first.
