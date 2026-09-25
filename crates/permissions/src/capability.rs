// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

use crate::audit::{AuditEvent, AuditLog};
use crate::scope::{self, ROOT_SCOPE};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Un permiso explícito para realizar una operación específica sobre un recurso.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Permite leer archivos en el path especificado (soporta prefijos).
    FileRead(PathBuf),
    /// Permite escribir archivos en el path especificado (soporta prefijos).
    FileWrite(PathBuf),
    /// Permite conexiones de red al host especificado (soporta wildcard `*.host`).
    Network(String),
    /// Permite crear procesos hijos.
    SpawnProcess,
    /// Permite leer todas las variables de entorno (equivale a --allow-env sin scope).
    EnvAccess,
    /// Permite leer una variable de entorno específica (--allow-env=VAR).
    /// `EnvAccess` (todas) cubre cualquier `EnvVar`; `EnvVar(a)` solo cubre `EnvVar(a)`.
    EnvVar(String),
    /// Permite llamadas FFI a librerías nativas en el path especificado.
    /// `FFI(PathBuf::from("/"))` equivale a `--allow-ffi` sin restricción de path.
    FFI(PathBuf),
}

use std::io::Write;
use std::sync::RwLock;

/// Estado de permisos del proceso, conforme al modelo deny-by-default.
///
/// Algoritmo de verificación:
/// 1. Si `deny_all_<tipo>` es true → DENY
/// 2. Si la capability está en `denied`  → DENY
/// 3. Si la capability está en `granted` → ALLOW
/// 4. Si `interactive` es true → PROMPT AL USUARIO
/// 5. Por defecto                        → DENY
#[derive(Debug, Default)]
pub struct PermissionState {
    /// Capabilities concedidas explícitamente por el usuario.
    pub granted: RwLock<HashSet<Capability>>,
    /// Capabilities denegadas explícitamente (tienen precedencia sobre granted).
    pub denied: RwLock<HashSet<Capability>>,

    /// Grants scoped to a specific dependency (`package.json["3va"].permissions.<name>`),
    /// keyed by package name. Checked in addition to (never instead of) the
    /// global `granted`/`denied` sets above — see [`crate::scope`] for how the
    /// "current" scope is determined at check() time.
    scoped_granted: RwLock<HashMap<String, HashSet<Capability>>>,
    /// Scoped denies — win over both scoped and global grants, same as the
    /// global `denied` set does.
    scoped_denied: RwLock<HashMap<String, HashSet<Capability>>>,

    /// Si está activado, lanza un prompt en consola cuando se detecta un permiso no configurado.
    pub interactive: bool,

    // Flags de denegación global por categoría
    deny_all_fs: bool,
    deny_all_net: bool,
    deny_all_env: bool,
    deny_all_process: bool,

    /// Shared audit log; when Some, every check() call appends an AuditEvent.
    pub audit_log: Option<Arc<Mutex<AuditLog>>>,
    /// When true, only denied checks are logged; when false, all checks are logged.
    pub audit_denied_only: bool,

    /// Every capability that was refused, with how often.
    ///
    /// A denial is the most useful thing a sandbox produces and the one it was
    /// worst at surfacing: the script throws, the run fails, and the user is
    /// left to work backwards from a stack trace to the flag they needed. This
    /// makes the run able to end with "you needed these three grants".
    tally: Mutex<DenialTally>,
    /// Local bind hosts that were refused, kept apart from outbound denials:
    /// "connect to 0.0.0.0" is not what went wrong, and
    /// `--allow-net=0.0.0.0` is not the fix (see [`Self::check_bind`]).
    denied_binds: Mutex<Vec<String>>,
    /// Print each denial as it happens, for when the summary is not enough.
    pub trace_denials: bool,
}

impl PermissionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Activa el modo interactivo para preguntar al usuario al vuelo.
    pub fn set_interactive(&mut self, interactive: bool) {
        self.interactive = interactive;
    }

    /// Concede una capability. No la agrega si ya existe.
    pub fn grant(&self, cap: Capability) {
        self.granted.write().unwrap().insert(cap);
    }

    /// Deniega una capability explícita (tiene precedencia sobre `grant`).
    pub fn deny(&self, cap: Capability) {
        self.denied.write().unwrap().insert(cap);
    }

    /// Concede una capability solo cuando el código que ejecuta pertenece al
    /// scope dado (nombre de paquete, o [`ROOT_SCOPE`] para el código de la
    /// app). No amplía el scope global — un grant aquí para `"axios"` no
    /// aplica a ningún otro paquete.
    pub fn grant_scoped(&self, scope: &str, cap: Capability) {
        if scope == ROOT_SCOPE {
            return self.grant(cap);
        }
        self.scoped_granted
            .write()
            .unwrap()
            .entry(scope.to_string())
            .or_default()
            .insert(cap);
    }

    /// Deniega una capability solo dentro de un scope específico. Gana sobre
    /// cualquier grant (global o del mismo scope), igual que `deny`.
    pub fn deny_scoped(&self, scope: &str, cap: Capability) {
        if scope == ROOT_SCOPE {
            return self.deny(cap);
        }
        self.scoped_denied
            .write()
            .unwrap()
            .entry(scope.to_string())
            .or_default()
            .insert(cap);
    }

    /// Deniega toda la categoría de filesystem (lectura y escritura).
    pub fn deny_all_fs(&mut self) {
        self.deny_all_fs = true;
    }

    /// Deniega todo acceso de red.
    pub fn deny_all_net(&mut self) {
        self.deny_all_net = true;
    }

    /// Deniega todo acceso a variables de entorno.
    pub fn deny_all_env(&mut self) {
        self.deny_all_env = true;
    }

    /// Deniega la creación de cualquier proceso hijo.
    pub fn deny_all_process(&mut self) {
        self.deny_all_process = true;
    }

    /// Attach a shared AuditLog. Every subsequent check() call appends an event.
    /// Set `denied_only = true` to only record checks that were denied.
    pub fn enable_audit(&mut self, log: Arc<Mutex<AuditLog>>, denied_only: bool) {
        self.audit_log = Some(log);
        self.audit_denied_only = denied_only;
    }

    /// Retorna una copia de todas las capabilities concedidas actualmente.
    pub fn list_granted(&self) -> Vec<Capability> {
        self.granted.read().unwrap().iter().cloned().collect()
    }

    /// Verifica si una operación está permitida.
    ///
    /// Para paths de archivo y hosts de red, el matching es por prefijo/subdominio,
    /// no por igualdad exacta, reflejando el comportamiento documentado.
    pub fn check(&self, required: &Capability) -> bool {
        let result = self.check_inner(required);
        if !result {
            self.record_denial(required);
        }
        self.record_audit(required, result);
        result
    }

    /// Like [`check`], but never appends to the audit log.
    ///
    /// Used for low-level runtime housekeeping that should not count as
    /// user-visible capability usage — most notably pre-filtering the
    /// `process.env` object at engine init, which probes every variable just
    /// to build the object and would otherwise swamp `3va permissions learn`
    /// with the entire host environment instead of the handful of variables
    /// the script actually read.
    pub fn check_quiet(&self, required: &Capability) -> bool {
        self.check_inner(required)
    }

    /// Record a capability check to the audit log without any side effect of
    /// [`check`]: no interactive prompt, no deny-by-default recording, no
    /// change in behavior for the caller. Only used when an audit log is
    /// attached (e.g. `3va permissions learn`).
    ///
    /// This backs the `process.env` Proxy's `__envAudit` hook: the object
    /// handed to scripts already contains only permitted variables, so reads
    /// are never re-gated — we only want the read to show up in the audit log
    /// so `learn` reports the variables a script actually touched instead of
    /// the full host environment.
    pub fn audit_env_read(&self, variable: &str) {
        if self.audit_log.is_none() || self.audit_denied_only {
            return;
        }
        let cap = Capability::EnvVar(variable.to_string());
        let allowed = self.check_inner(&cap);
        self.record_audit(&cap, allowed);
    }

    /// A script read `process.env[variable]` and the variable isn't in the
    /// pre-filtered env object. Returns its value if the read is allowed now
    /// (e.g. just granted at the interactive prompt). Otherwise it goes
    /// through [`Self::check`] like any other refusal: audited, tallied in
    /// the run's denial summary, and remembered by the prompt. Variables the
    /// host doesn't define hide nothing, so they are not checked at all.
    pub fn request_env_var(&self, variable: &str) -> Option<String> {
        let value = std::env::var(variable).ok()?;
        self.check(&Capability::EnvVar(variable.to_string()))
            .then_some(value)
    }

    fn check_inner(&self, required: &Capability) -> bool {
        // Paso 1: deny_all global por categoría
        match required {
            Capability::FileRead(_) | Capability::FileWrite(_) if self.deny_all_fs => {
                return false;
            }
            Capability::Network(_) if self.deny_all_net => {
                return false;
            }
            Capability::EnvAccess | Capability::EnvVar(_) if self.deny_all_env => {
                return false;
            }
            Capability::SpawnProcess if self.deny_all_process => {
                return false;
            }
            _ => {}
        }

        // Which package's code is currently executing (set by the JS engine's
        // require() wrapper; "." / ROOT_SCOPE for the app's own code, or when
        // no scoped grants were ever declared for this project).
        let active_scope = scope::current_scope();

        // Paso 2: deny-list explícita — global primero, luego la del scope activo.
        {
            let denied = self.denied.read().unwrap();
            if denied.iter().any(|d| caps_match(d, required)) {
                return false;
            }
        }
        if active_scope != ROOT_SCOPE {
            let scoped_denied = self.scoped_denied.read().unwrap();
            if let Some(set) = scoped_denied.get(&active_scope)
                && set.iter().any(|d| caps_match(d, required))
            {
                return false;
            }
        }

        // Paso 3: granted-list — global primero, luego la del scope activo.
        {
            let granted = self.granted.read().unwrap();
            if granted.iter().any(|g| caps_match(g, required)) {
                return true;
            }
        }
        if active_scope != ROOT_SCOPE {
            let scoped_granted = self.scoped_granted.read().unwrap();
            if let Some(set) = scoped_granted.get(&active_scope)
                && set.iter().any(|g| caps_match(g, required))
            {
                return true;
            }
        }

        // Paso 4: Modo interactivo
        if self.interactive {
            return self.prompt_user(required);
        }

        false
    }

    /// Check permission to *bind/listen* a local server on `host`, as opposed
    /// to connecting out to it.
    ///
    /// `http.createServer().listen(port)` and `net.createServer().listen(port)`
    /// default their bind host to `0.0.0.0` (all interfaces) regardless of
    /// which specific remote host the user wrote in `allow-net` — so a plain
    /// `check(&Capability::Network(host))` almost always fails even when the
    /// user clearly intended to let the script run its own server (they
    /// granted `allow-net: ["127.0.0.1"]` or some other host, not literally
    /// `"0.0.0.0"`). Running a server you wrote is a different risk than
    /// connecting out to arbitrary hosts, so any existing network grant is
    /// treated as authorization to bind on a local/wildcard address.
    ///
    /// This must never be reused for outbound checks (fetch/http.request/tcp
    /// connect) — there, the granted host must still match the real
    /// destination, or granting `allow-net: ["api.example.com"]` would also
    /// silently permit SSRF-style requests to `127.0.0.1`.
    pub fn check_bind(&self, host: &str) -> bool {
        let required = Capability::Network(host.to_string());

        if self.deny_all_net {
            self.record_bind_denial(host);
            self.record_audit(&required, false);
            return false;
        }
        {
            let denied = self.denied.read().unwrap();
            if denied.iter().any(|d| caps_match(d, &required)) {
                drop(denied);
                self.record_bind_denial(host);
                self.record_audit(&required, false);
                return false;
            }
        }
        if is_local_bind_host(host) {
            let granted = self.granted.read().unwrap();
            if granted.iter().any(|g| matches!(g, Capability::Network(_))) {
                drop(granted);
                self.record_audit(&required, true);
                return true;
            }
        }
        // Not `self.check()`: a refused bind is not an outbound refusal, and
        // reporting it as one would tell the user to grant a destination the
        // script never connects to.
        let allowed = self.check_inner(&required);
        if !allowed {
            self.record_bind_denial(host);
        }
        self.record_audit(&required, allowed);
        allowed
    }

    /// Local bind hosts that were refused during this run, deduplicated.
    pub fn denied_bind_hosts(&self) -> Vec<String> {
        self.denied_binds.lock().unwrap().clone()
    }

    fn record_bind_denial(&self, host: &str) {
        let mut binds = self.denied_binds.lock().unwrap();
        if binds.iter().any(|h| h == host) {
            return;
        }
        let first = binds.is_empty();
        binds.push(host.to_string());
        drop(binds);
        if first && self.trace_denials {
            eprintln!("  [denied] bind a local server on {host}");
        }
    }

    /// Every capability refused during this run, deduplicated and in the order
    /// first refused.
    pub fn denials(&self) -> Vec<Capability> {
        self.tally.lock().unwrap().order.clone()
    }

    /// Every capability refused during this run with its hit count, in the
    /// order first refused. The count separates "touched one file it was not
    /// allowed to read" from "polled a file it was never allowed to read,
    /// four hundred times while retrying" — the second is a bug in the script.
    pub fn denial_counts(&self) -> Vec<(Capability, usize)> {
        let tally = self.tally.lock().unwrap();
        tally
            .order
            .iter()
            .map(|c| (c.clone(), tally.counts[c]))
            .collect()
    }

    fn record_denial(&self, cap: &Capability) {
        let first = {
            let mut tally = self.tally.lock().unwrap();
            let hits = tally.counts.entry(cap.clone()).or_insert(0);
            let first = *hits == 0;
            *hits += 1;
            if first {
                tally.order.push(cap.clone());
            }
            first
        };
        // Repeats are counted but not printed: a retry loop would otherwise
        // bury every other line in the run.
        if first && self.trace_denials {
            eprintln!("  [denied] {}", describe_capability(cap));
        }
    }

    fn record_audit(&self, cap: &Capability, allowed: bool) {
        let log = match &self.audit_log {
            Some(l) => l,
            None => return,
        };
        if self.audit_denied_only && allowed {
            return;
        }
        let event = match cap {
            Capability::FileRead(p) => AuditEvent::FileAccess {
                timestamp: Utc::now(),
                path: p.clone(),
                operation: "read".to_string(),
                allowed,
            },
            Capability::FileWrite(p) => AuditEvent::FileAccess {
                timestamp: Utc::now(),
                path: p.clone(),
                operation: "write".to_string(),
                allowed,
            },
            Capability::Network(h) => AuditEvent::NetworkAccess {
                timestamp: Utc::now(),
                host: h.clone(),
                port: 0,
                allowed,
            },
            Capability::SpawnProcess => AuditEvent::ProcessSpawn {
                timestamp: Utc::now(),
                command: "*".to_string(),
                allowed,
            },
            Capability::EnvAccess => AuditEvent::EnvAccess {
                timestamp: Utc::now(),
                variable: "*".to_string(),
                allowed,
            },
            Capability::EnvVar(v) => AuditEvent::EnvAccess {
                timestamp: Utc::now(),
                variable: v.clone(),
                allowed,
            },
            Capability::FFI(p) => AuditEvent::PermissionDenied {
                timestamp: Utc::now(),
                capability: "FFI".to_string(),
                resource: p.display().to_string(),
                reason: if allowed {
                    "allowed".to_string()
                } else {
                    "not granted".to_string()
                },
            },
        };
        if let Ok(mut l) = log.lock() {
            l.add_event(event);
        }
    }

    fn prompt_user(&self, required: &Capability) -> bool {
        use std::io::IsTerminal;

        // Non-interactive contexts (pipes, CI, integration tests) have no TTY.
        // Blocking for input would hang indefinitely — deny silently instead.
        if !std::io::stdin().is_terminal() {
            self.deny(required.clone());
            return false;
        }

        let msg = match required {
            Capability::FileRead(p) => format!("read file '{}'", p.display()),
            Capability::FileWrite(p) => format!("write file '{}'", p.display()),
            Capability::Network(h) => format!("connect to network host '{h}'"),
            Capability::SpawnProcess => "spawn child processes".to_string(),
            Capability::EnvAccess => "access all environment variables".to_string(),
            Capability::EnvVar(v) => format!("access environment variable '{v}'"),
            Capability::FFI(p) => format!("call native library '{}'", p.display()),
        };

        eprint!(
            "\n[!] The script is requesting permission to {msg}.\nAllow? [y (once) / N (deny) / A (always)] "
        );
        let _ = std::io::stdout().flush();

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() {
            let choice = input.trim();
            if choice.eq_ignore_ascii_case("y") {
                return true;
            } else if choice == "A" {
                self.grant(required.clone());
                return true;
            }
        }

        // Anything else (N or enter) is deny
        self.deny(required.clone());
        false
    }
}

impl Clone for PermissionState {
    fn clone(&self) -> Self {
        Self {
            granted: RwLock::new(self.granted.read().unwrap().clone()),
            denied: RwLock::new(self.denied.read().unwrap().clone()),
            scoped_granted: RwLock::new(self.scoped_granted.read().unwrap().clone()),
            scoped_denied: RwLock::new(self.scoped_denied.read().unwrap().clone()),
            interactive: self.interactive,
            deny_all_fs: self.deny_all_fs,
            deny_all_net: self.deny_all_net,
            deny_all_env: self.deny_all_env,
            deny_all_process: self.deny_all_process,
            audit_log: self.audit_log.clone(),
            audit_denied_only: self.audit_denied_only,
            // A clone reports only its own denials: the summary describes the
            // run that is finishing, not every check made since process start.
            tally: Mutex::new(DenialTally::default()),
            denied_binds: Mutex::new(Vec::new()),
            trace_denials: self.trace_denials,
        }
    }
}

// Evalúa si la capability `granted` cubre a `required`.
// - FileRead/FileWrite: el path requerido debe comenzar con el path concedido.
// - Network: el host requerido debe coincidir exactamente o por wildcard *.host.
// - El resto: igualdad exacta.

// Strip Windows \\?\ extended-length path prefix so comparisons work
// regardless of whether the path was produced by canonicalize() or not.
#[cfg(windows)]
fn normalize_path(p: &std::path::Path) -> std::borrow::Cow<'_, std::path::Path> {
    let s = p.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        std::borrow::Cow::Owned(std::path::PathBuf::from(stripped))
    } else {
        std::borrow::Cow::Borrowed(p)
    }
}
#[cfg(not(windows))]
fn normalize_path(p: &std::path::Path) -> std::borrow::Cow<'_, std::path::Path> {
    std::borrow::Cow::Borrowed(p)
}

/// Resolve symlinks for path comparison — falls back to the original path when
/// canonicalize fails (e.g. path doesn't exist yet).
fn canon_path(p: &std::path::Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Make a relative path absolute against the process cwd, leaving absolute
/// paths untouched. Like `canon_path` this falls back to the literal path when
/// the cwd is unavailable — the caller keeps the raw path in that case, so
/// matching simply behaves as it did before (fails closed rather than wide).
fn absolutize(p: &std::path::Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}

/// Check if `target` is covered by `allowed`, resolving symlinks on both sides
/// so that `/lib64` (symlink → `/usr/lib64`) matches `--allow-read=/lib64`.
///
/// Both sides are absolutized against the process cwd first, so a grant from a
/// CLI flag (`--allow-read=./config` resolved to `$CWD/config`) matches an
/// application request regardless of whether that request was made with a
/// relative or absolute path.
fn path_covered_by(target: &std::path::Path, allowed: &std::path::Path) -> bool {
    let norm_t = normalize_path(target);
    let norm_a = normalize_path(allowed);
    let abs_t = absolutize(norm_t.as_ref());
    let abs_a = absolutize(norm_a.as_ref());
    if abs_t.starts_with(&abs_a) {
        return true;
    }
    // Re-check after resolving symlinks on both sides.
    canon_path(&abs_t).starts_with(canon_path(&abs_a))
}

/// Wildcard/loopback addresses a server binds to when the app didn't ask for
/// a specific host — these describe *this machine*, not a remote target.
fn is_local_bind_host(host: &str) -> bool {
    matches!(host, "0.0.0.0" | "::" | "127.0.0.1" | "::1" | "localhost")
}

fn caps_match(granted: &Capability, required: &Capability) -> bool {
    match (granted, required) {
        (Capability::FileRead(allowed), Capability::FileRead(target)) => {
            path_covered_by(target, allowed)
        }
        (Capability::FileWrite(allowed), Capability::FileWrite(target)) => {
            path_covered_by(target, allowed)
        }
        (Capability::Network(allowed), Capability::Network(target)) => {
            host_matches(allowed, target)
        }
        // EnvAccess (all) covers both EnvAccess and any specific EnvVar.
        (Capability::EnvAccess, Capability::EnvAccess) => true,
        (Capability::EnvAccess, Capability::EnvVar(_)) => true,
        (Capability::EnvVar(a), Capability::EnvVar(b)) => a == b,
        // FFI: el path requerido debe comenzar con el path concedido (con symlinks).
        (Capability::FFI(allowed), Capability::FFI(target)) => path_covered_by(target, allowed),
        // Capabilities sin parámetros: igualdad de variante
        (a, b) => a == b,
    }
}

/// Refused capabilities for one run: which, in what order, and how often.
#[derive(Debug, Default)]
struct DenialTally {
    order: Vec<Capability>,
    counts: HashMap<Capability, usize>,
}

/// The CLI flag that would grant this capability, as the user should write it.
///
/// A denial is only actionable if the answer says what to do, so this returns
/// the flag rather than a description: `--allow-net=api.example.com:8080` is
/// something you can paste, "network access to api.example.com:8080" is not.
pub fn grant_flag(cap: &Capability) -> String {
    match cap {
        Capability::FileRead(p) => format!("--allow-read={}", p.display()),
        Capability::FileWrite(p) => format!("--allow-write={}", p.display()),
        Capability::Network(h) => format!("--allow-net={h}"),
        Capability::EnvVar(v) => format!("--allow-env={v}"),
        Capability::EnvAccess => "--allow-env".to_string(),
        Capability::SpawnProcess => "--allow-child-process".to_string(),
        Capability::FFI(p) => format!("--allow-ffi={}", p.display()),
    }
}

/// A one-line description of a denied capability, for `--trace-denials`.
pub fn describe_capability(cap: &Capability) -> String {
    match cap {
        Capability::FileRead(p) => format!("read {}", p.display()),
        Capability::FileWrite(p) => format!("write {}", p.display()),
        Capability::Network(h) => format!("connect to {h}"),
        Capability::EnvVar(v) => format!("read environment variable {v}"),
        Capability::EnvAccess => "read environment variables".to_string(),
        Capability::SpawnProcess => "spawn a child process".to_string(),
        Capability::FFI(p) => format!("load native library {}", p.display()),
    }
}

/// The `package.json` `3va.permissions` key a capability belongs under.
///
/// The manifest deliberately uses the same kebab-case spelling as the CLI
/// flags (`--allow-net` -> `"allow-net"`), so a grant can be moved between the
/// two without translation. `learn --write` and this table must agree with
/// `read_package_json_permissions`; a key spelled differently is silently
/// ignored, which would make a written grant useless.
pub fn manifest_key(cap: &Capability) -> &'static str {
    match cap {
        Capability::FileRead(_) => "allow-read",
        Capability::FileWrite(_) => "allow-write",
        Capability::Network(_) => "allow-net",
        Capability::EnvVar(_) | Capability::EnvAccess => "allow-env",
        Capability::SpawnProcess => "allow-child-process",
        Capability::FFI(_) => "allow-ffi",
    }
}

/// Format a host and port the way a grant must spell them.
///
/// An IPv6 literal is bracketed, because `::1:443` cannot be split back into
/// an address and a port — a bare `::1` would read as host `:` on port `1`.
/// Callers building a check request must use this rather than `format!` so
/// the two sides of a comparison are always spelled the same way.
pub fn authority(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// The port a spec *writes*, as text, without validating it — `None` when the
/// spec names no port. `Some("99999")` means the spec does name a port, just
/// not a valid one, which callers need to tell apart from "no port at all".
fn written_port(spec: &str) -> Option<&str> {
    if let Some(rest) = spec.strip_prefix('[') {
        return rest
            .find(']')
            .and_then(|close| rest[close + 1..].strip_prefix(':'));
    }
    if spec.matches(':').count() > 1 {
        // Bare IPv6 literal: every colon belongs to the address.
        return None;
    }
    spec.rsplit_once(':')
        .map(|(_, port)| port)
        .filter(|port| !port.is_empty())
}

/// Split `host`, `host:port` or `[::1]:port` into its parts.
///
/// The port is optional and its presence is what makes a grant meaningful:
/// `allow-net: ["api.example.com:443"]` is a promise about one service, not
/// about the whole host.
pub fn split_host_port(spec: &str) -> (&str, Option<u16>) {
    // Bracketed IPv6: `[::1]:443` — the last colon after the bracket is the
    // port separator, colons inside the brackets are part of the address.
    if let Some(rest) = spec.strip_prefix('[')
        && let Some(close) = rest.find(']')
    {
        let host = &rest[..close];
        return match rest[close + 1..].strip_prefix(':') {
            Some(p) => (host, p.parse().ok()),
            None => (host, None),
        };
    }

    // An unbracketed value with more than one colon is a bare IPv6 literal, so
    // any colon in it belongs to the address, not to a port.
    if spec.matches(':').count() > 1 {
        return (spec, None);
    }
    match spec.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => (host, port.parse().ok()),
        _ => (spec, None),
    }
}

/// Do the two host specs describe the same host, and does the grant cover the
/// destination's port?
///
/// Rules, in the order they matter for safety:
///
/// * `*` covers everything, as it always has.
/// * A grant **with** a port covers only that port. `api.example.com:443` does
///   not authorise a connection to `api.example.com:8080` — an app server
///   listening on an admin port is exactly what a scoped grant should not
///   reach.
/// * A grant **without** a port covers any port on that host, which is what
///   `allow-net: ["api.example.com"]` has always meant.
/// * When the grant names a port but the destination reports none, the grant
///   does not apply: a port we cannot see is a port we did not authorise.
pub fn host_matches(pattern: &str, host: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    // A pattern that names a port it cannot spell (`api.example.com:https`,
    // `api.example.com:99999`) is a typo in a grant. Treating it as portless
    // would silently widen a narrow promise to every port on that host, which
    // is the one outcome the user did not ask for — so it matches nothing.
    if let Some(written) = written_port(pattern)
        && written.parse::<u16>().is_err()
    {
        return false;
    }
    let (pattern_host, pattern_port) = split_host_port(pattern);
    let (target_host, target_port) = split_host_port(host);

    if let Some(wanted) = pattern_port {
        // A pattern that names a port must be answered by a destination that
        // names the same one.
        if target_port != Some(wanted) {
            return false;
        }
    }

    if pattern_host == target_host {
        return true;
    }
    if let Some(suffix) = pattern_host.strip_prefix("*.") {
        return target_host.ends_with(suffix)
            && target_host.len() > suffix.len()
            && target_host.as_bytes()[target_host.len() - suffix.len() - 1] == b'.';
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denials_are_recorded_once_per_shape() {
        let state = PermissionState::new();
        state.grant(Capability::Network("api.example.com:443".into()));
        assert!(state.check(&Capability::Network("api.example.com:443".into())));
        // Same shape, different value: reported once, not four times.
        for _ in 0..4 {
            assert!(!state.check(&Capability::Network("other.example.com:8443".into())));
        }
        assert!(!state.check(&Capability::FileRead(PathBuf::from("/etc/shadow"))));
        let denials = state.denials();
        assert_eq!(denials.len(), 2, "{denials:?}");
        // Repeated refusals are counted, not re-listed, and order is stable.
        let counts = state.denial_counts();
        assert_eq!(
            counts[0],
            (Capability::Network("other.example.com:8443".into()), 4)
        );
        assert_eq!(
            counts[1],
            (Capability::FileRead(PathBuf::from("/etc/shadow")), 1)
        );
    }

    #[test]
    fn a_run_with_no_denials_reports_none() {
        let state = PermissionState::new();
        state.grant(Capability::EnvVar("NODE_ENV".into()));
        assert!(state.check(&Capability::EnvVar("NODE_ENV".into())));
        assert!(state.denials().is_empty());
        assert!(state.denial_counts().is_empty());
    }

    #[test]
    fn every_denial_carries_the_flag_that_would_grant_it() {
        // A denial is only useful if it says what to do.
        assert_eq!(
            grant_flag(&Capability::Network("api.example.com:8080".into())),
            "--allow-net=api.example.com:8080"
        );
        assert_eq!(
            grant_flag(&Capability::FileRead(PathBuf::from("/etc/hosts"))),
            "--allow-read=/etc/hosts"
        );
        assert_eq!(
            grant_flag(&Capability::EnvVar("SECRET".into())),
            "--allow-env=SECRET"
        );
        assert_eq!(
            grant_flag(&Capability::SpawnProcess),
            "--allow-child-process"
        );
        assert_eq!(
            describe_capability(&Capability::Network("a.b:1".into())),
            "connect to a.b:1"
        );
        // The manifest mirrors the CLI flag spelling; a key spelled
        // differently would be written and then ignored at load time.
        assert_eq!(
            manifest_key(&Capability::FileWrite(PathBuf::from("/tmp"))),
            "allow-write"
        );
        assert_eq!(manifest_key(&Capability::Network("*".into())), "allow-net");
    }

    #[test]
    fn a_refused_bind_is_reported_as_a_bind_not_a_destination() {
        // `server.listen(8080)` binds 0.0.0.0. Suggesting `--allow-net=0.0.0.0`
        // would send the user after a destination they never connect to; the
        // actual cause is that no network grant exists at all.
        let state = PermissionState::new();
        assert!(!state.check_bind("0.0.0.0"));
        assert!(
            state.denials().is_empty(),
            "a bind is not an outbound denial"
        );
        assert_eq!(state.denied_bind_hosts(), vec!["0.0.0.0".to_string()]);

        // Any network grant authorizes a local bind, and then there is nothing
        // to report at all.
        let state = PermissionState::new();
        state.grant(Capability::Network("api.example.com".into()));
        assert!(state.check_bind("0.0.0.0"));
        assert!(state.denied_bind_hosts().is_empty());
        assert!(state.denials().is_empty());
    }

    #[test]
    fn an_unspellable_port_matches_nothing() {
        // Fail closed: a typo must not widen a narrow grant into "any port".
        assert!(!host_matches(
            "api.example.com:https",
            "api.example.com:443"
        ));
        assert!(!host_matches("api.example.com:https", "api.example.com"));
        assert!(!host_matches(
            "api.example.com:99999",
            "api.example.com:99999"
        ));
        // A bracketed literal with a bad port behaves the same way.
        assert!(!host_matches("[::1]:https", "[::1]:443"));
        // ...and a host with no port in it is unaffected.
        assert!(host_matches("api.example.com", "api.example.com:443"));
        // Bare IPv6 is not mistaken for host:port.
        assert!(host_matches("::1", "::1"));
        // ...and a port-scoped IPv6 grant needs the bracketed spelling, which
        // is what `authority` produces for every request it builds.
        assert_eq!(authority("::1", 443), "[::1]:443");
        assert_eq!(authority("example.com", 443), "example.com:443");
        assert!(host_matches("[::1]:443", &authority("::1", 443)));
        assert!(!host_matches("[::1]:443", &authority("::1", 8443)));
    }

    #[test]
    fn a_port_scoped_grant_covers_only_that_port() {
        // The point of `host:port`: allowing the HTTPS API must not also allow
        // an admin server on the same machine.
        assert!(host_matches("api.example.com:443", "api.example.com:443"));
        assert!(!host_matches("api.example.com:443", "api.example.com:8080"));
        assert!(!host_matches("api.example.com:443", "api.example.com"));
        assert!(!host_matches(
            "api.example.com:443",
            "other.example.com:443"
        ));
    }

    #[test]
    fn a_portless_grant_still_covers_every_port() {
        // This is what `allow-net: ["api.example.com"]` has always meant, and
        // existing manifests depend on it.
        assert!(host_matches("api.example.com", "api.example.com:443"));
        assert!(host_matches("api.example.com", "api.example.com:8080"));
        assert!(host_matches("api.example.com", "api.example.com"));
    }

    #[test]
    fn the_wildcard_forms_keep_working() {
        assert!(host_matches("*", "anything.example.com:443"));
        assert!(host_matches("*.example.com", "api.example.com:443"));
        assert!(!host_matches("*.example.com", "example.com:443"));
        assert!(!host_matches("*.example.com", "evil.com:443"));
    }

    #[test]
    fn a_scoped_wildcard_with_a_port_matches_only_that_port() {
        assert!(host_matches("*.example.com:443", "api.example.com:443"));
        assert!(!host_matches("*.example.com:443", "api.example.com:80"));
    }

    #[test]
    fn ipv6_literals_split_correctly() {
        assert_eq!(split_host_port("[::1]:443"), ("::1", Some(443)));
        assert_eq!(split_host_port("[::1]"), ("::1", None));
        // A bare IPv6 address has several colons and no port.
        assert_eq!(split_host_port("::1"), ("::1", None));
        assert_eq!(split_host_port("fe80::1"), ("fe80::1", None));
        assert_eq!(
            split_host_port("api.example.com:8080"),
            ("api.example.com", Some(8080))
        );
        assert_eq!(
            split_host_port("api.example.com"),
            ("api.example.com", None)
        );
    }

    #[test]
    fn a_check_against_a_port_scoped_grant_accepts_only_the_named_port() {
        let state = PermissionState::new();
        state.grant(Capability::Network("api.example.com:443".to_string()));
        assert!(state.check(&Capability::Network("api.example.com:443".into())));
        assert!(!state.check(&Capability::Network("api.example.com:8080".into())));
        // A grant with no port remains a whole-host grant.
        let whole = PermissionState::new();
        whole.grant(Capability::Network("api.example.com".to_string()));
        assert!(whole.check(&Capability::Network("api.example.com:8080".into())));
    }

    #[test]
    fn check_bind_allows_wildcard_bind_host_when_any_net_grant_exists() {
        // The common real-world case: package.json grants allow-net for a
        // specific host, but http.createServer().listen(port) with no host
        // defaults to binding "0.0.0.0" — that must still be allowed to run
        // the server the user clearly intended to permit.
        let state = PermissionState::new();
        state.grant(Capability::Network("127.0.0.1".to_string()));
        assert!(state.check_bind("0.0.0.0"));
        assert!(state.check_bind("127.0.0.1"));
        assert!(state.check_bind("localhost"));
    }

    #[test]
    fn check_bind_denies_without_any_net_grant() {
        let state = PermissionState::new();
        assert!(!state.check_bind("0.0.0.0"));
    }

    #[test]
    fn check_bind_respects_explicit_deny_net() {
        let state = PermissionState::new();
        state.grant(Capability::Network("127.0.0.1".to_string()));
        state.deny(Capability::Network("0.0.0.0".to_string()));
        assert!(!state.check_bind("0.0.0.0"));
    }

    #[test]
    fn check_bind_does_not_relax_outbound_connect_checks() {
        // Granting allow-net for one host must never let an *outbound*
        // connection reach a different host, even a loopback one (SSRF).
        // check_bind's relaxation is bind-only — plain check() must stay strict.
        let state = PermissionState::new();
        state.grant(Capability::Network("api.example.com".to_string()));
        assert!(!state.check(&Capability::Network("127.0.0.1".to_string())));
        assert!(!state.check(&Capability::Network("0.0.0.0".to_string())));
    }

    #[test]
    fn denied_env_reads_are_tallied_and_granted_reads_return_the_value() {
        // PATH is defined in every test environment.
        let state = PermissionState::new();
        assert_eq!(state.request_env_var("PATH"), None);
        assert_eq!(
            state.denials(),
            vec![Capability::EnvVar("PATH".to_string())]
        );
        // Variables the host doesn't define hide nothing.
        assert_eq!(state.request_env_var("__3VA_SURELY_UNSET__"), None);
        assert_eq!(
            state.denials(),
            vec![Capability::EnvVar("PATH".to_string())]
        );

        let granted = PermissionState::new();
        granted.grant(Capability::EnvVar("PATH".to_string()));
        assert!(granted.request_env_var("PATH").is_some());
    }

    #[test]
    fn deny_by_default() {
        let state = PermissionState::new();
        assert!(!state.check(&Capability::EnvAccess));
        assert!(!state.check(&Capability::FileRead(PathBuf::from("/etc"))));
    }

    #[test]
    fn grant_allows() {
        let state = PermissionState::new();
        state.grant(Capability::EnvAccess);
        assert!(state.check(&Capability::EnvAccess));
    }

    #[test]
    fn deny_overrides_grant() {
        let state = PermissionState::new();
        state.grant(Capability::EnvAccess);
        state.deny(Capability::EnvAccess);
        assert!(!state.check(&Capability::EnvAccess));
    }

    #[test]
    fn deny_all_overrides_grant() {
        let mut state = PermissionState::new();
        state.grant(Capability::FileRead(PathBuf::from("/")));
        state.deny_all_fs();
        assert!(!state.check(&Capability::FileRead(PathBuf::from("/app"))));
    }

    #[test]
    fn path_prefix_matching() {
        let state = PermissionState::new();
        state.grant(Capability::FileRead(PathBuf::from("/app")));
        assert!(state.check(&Capability::FileRead(PathBuf::from("/app/config.json"))));
        assert!(!state.check(&Capability::FileRead(PathBuf::from("/etc/passwd"))));
    }

    #[test]
    fn wildcard_host_matching() {
        let state = PermissionState::new();
        state.grant(Capability::Network("*.example.com".to_string()));
        assert!(state.check(&Capability::Network("api.example.com".to_string())));
        assert!(!state.check(&Capability::Network("evil.com".to_string())));
        assert!(!state.check(&Capability::Network("example.com".to_string())));
    }

    // ── scoped (per-package) grants ────────────────────────────────────────────
    // These tests set the thread-local "current scope" directly, mirroring
    // what the JS engine's require() wrapper does before a package touches a
    // capability-gated builtin. Serialized via a mutex since thread-locals are
    // per-thread but `cargo test` may reuse the test-runner thread across
    // `#[test]` fns run serially in the same binary — reset scope on exit.

    #[test]
    fn scoped_grant_does_not_leak_to_other_scopes() {
        let state = PermissionState::new();
        state.grant_scoped("axios", Capability::Network("api.example.com".to_string()));

        crate::scope::set_current_scope("axios");
        assert!(state.check(&Capability::Network("api.example.com".to_string())));

        crate::scope::set_current_scope("express");
        assert!(!state.check(&Capability::Network("api.example.com".to_string())));

        crate::scope::set_current_scope(ROOT_SCOPE);
        assert!(!state.check(&Capability::Network("api.example.com".to_string())));
    }

    #[test]
    fn scoped_deny_overrides_global_grant_for_that_scope_only() {
        let state = PermissionState::new();
        state.grant(Capability::Network("*".to_string()));
        state.deny_scoped(
            "sketchy-pkg",
            Capability::Network("internal.corp".to_string()),
        );

        crate::scope::set_current_scope("sketchy-pkg");
        assert!(!state.check(&Capability::Network("internal.corp".to_string())));
        // The wildcard grant still covers a different host in that same scope.
        assert!(state.check(&Capability::Network("other.example.com".to_string())));

        crate::scope::set_current_scope("some-other-pkg");
        assert!(state.check(&Capability::Network("internal.corp".to_string())));

        crate::scope::set_current_scope(ROOT_SCOPE);
    }

    #[test]
    fn root_scope_grant_is_stored_globally_not_as_a_scope_entry() {
        let state = PermissionState::new();
        state.grant_scoped(ROOT_SCOPE, Capability::EnvAccess);
        assert!(
            state
                .granted
                .read()
                .unwrap()
                .contains(&Capability::EnvAccess)
        );
        assert!(state.scoped_granted.read().unwrap().is_empty());
    }
}
