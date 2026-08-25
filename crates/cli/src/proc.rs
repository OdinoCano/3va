use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn default_instances() -> u32 {
    1
}

fn default_max_restarts() -> u32 {
    15
}

fn default_autorestart() -> bool {
    true
}

/// Base delay (milliseconds) for the first automatic restart after a crash.
const BACKOFF_BASE_MS: u64 = 500;

/// Upper bound (milliseconds) on the delay between consecutive restarts, so a
/// process stuck in a crash loop never burns the supervisor's CPU or floods
/// its log faster than this — 30 s is the "give it a while, maybe the
/// dependency came back" ceiling.
const BACKOFF_MAX_MS: u64 = 30_000;

/// Exponential backoff delay before restart number `restarts` (1-based).
///
/// `delay(n) = min(BASE · 2^(n−1), MAX)` with BASE = 500 ms, MAX = 30 s, so the
/// schedule is 500 ms → 1 s → 2 s → 4 s → 8 s → 16 s → 30 s → 30 s → … .
///
/// Design rationale (vs. the fixed/linear alternatives):
/// - A single spurious crash (e.g. an OOM-killed worker) recovers in 500 ms,
///   the same ballpark as pm2's default `restart_delay`.
/// - A sustained crash loop reaches the 30 s ceiling after 7 consecutive
///   crashes, bounding worst-case CPU/log burn — the explicit requirement
///   here — which a fixed delay can't do without being slow for the common
///   one-off case, and a linear ramp (the previous 300 ms·n schedule, capped
///   at 3 s) grows too slowly to bound a long crash loop.
/// - No jitter: one supervisor supervises one process (unlike a fleet-wide
///   orchestration tier where synchronized restarts thunder-herd), and jitter
///   would make the restart schedule nondeterministic and hard to test.
pub fn backoff_delay(restarts: u32) -> std::time::Duration {
    if restarts == 0 {
        return std::time::Duration::from_millis(BACKOFF_BASE_MS);
    }
    // `min(16)` keeps the shift well inside the cap (500 ms · 2^16 is already
    // hours — the 30 s ceiling is hit at 2^6) while avoiding u64 overflow.
    let exp = (restarts - 1).min(16);
    let delay = BACKOFF_BASE_MS.saturating_mul(1u64 << exp);
    std::time::Duration::from_millis(delay.min(BACKOFF_MAX_MS))
}

/// Managed process metadata.
///
/// `pid` is always the *supervisor* process — the long-lived process that
/// owns the app instance(s) and restarts them on crash — not an app instance
/// itself. `instance_pids` holds the actual worker PIDs (more than one only
/// in cluster mode, `instances > 1`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub name: String,
    pub entry: PathBuf,
    pub pid: u32,
    pub cwd: PathBuf,
    pub log_path: PathBuf,
    pub status: String,
    pub started_at: u64,
    pub restarts: u32,
    pub args: Vec<String>,
    pub port: Option<u16>,
    #[serde(default = "default_instances")]
    pub instances: u32,
    #[serde(default = "default_max_restarts")]
    pub max_restarts: u32,
    /// Whether the supervisor should respawn the app when it dies
    /// unexpectedly. `3va start --no-autorestart` sets this to false; a manual
    /// `3va restart` re-applies it. Defaults to true for pre-existing
    /// `~/.3va/processes/*.json` files (missing field).
    #[serde(default = "default_autorestart")]
    pub autorestart: bool,
    #[serde(default)]
    pub instance_pids: Vec<u32>,
}

fn processes_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".3va").join("processes")
}

fn process_path(name: &str) -> PathBuf {
    processes_dir().join(format!("{}.json", name))
}

fn log_path(name: &str) -> PathBuf {
    processes_dir().join(format!("{}.log", name))
}

fn ensure_dir() -> std::io::Result<()> {
    let dir = processes_dir();
    fs::create_dir_all(&dir)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        // Send signal 0 to check if process exists without actually signaling it
        let result = unsafe { libc::kill(pid as i32, 0) };
        result == 0
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new("tasklist")
            .args(["/FO", "CSV", "/NH", "/FI", &format!("PID eq {}", pid)])
            .output()
            .map(|o| {
                let out = String::from_utf8_lossy(&o.stdout);
                // CSV format: "image","pid","session","session#","mem"
                // Exact match on the PID column (2nd quoted field)
                out.split('"')
                    .nth(3)
                    .map(|s| s.trim() == pid.to_string())
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }
}

fn save_process(info: &ProcessInfo) -> std::io::Result<()> {
    ensure_dir()?;
    let path = process_path(&info.name);
    let json = serde_json::to_string_pretty(info)?;
    fs::write(&path, json)
}

fn load_process(name: &str) -> std::io::Result<ProcessInfo> {
    let path = process_path(name);
    let json = fs::read_to_string(&path)?;
    let info: ProcessInfo = serde_json::from_str(&json)?;
    Ok(info)
}

fn delete_process_file(name: &str) -> std::io::Result<()> {
    let path = process_path(name);
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

fn list_all_processes() -> Vec<ProcessInfo> {
    let dir = processes_dir();
    if !dir.exists() {
        return Vec::new();
    }
    let mut processes = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(info) = fs::read_to_string(&path).and_then(|s| {
                    serde_json::from_str::<ProcessInfo>(&s)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
                }) {
                    processes.push(info);
                }
            }
        }
    }
    processes
}

/// Resolve `entry` to an absolute path against `cwd`. The app always runs
/// from `cwd` itself — same as `npm run <script>`/`node file.js`, where the
/// child's `process.cwd()` is wherever the command was invoked from, never
/// derived from the resolved binary's own location. That distinction matters
/// once `entry` comes from `resolve_start_entry()` resolving a package.json
/// script through a `node_modules/.bin` symlink (e.g. `android` →
/// `node_modules/react-native/cli.js`): using the entry's *own* directory as
/// cwd used to make a spawned Metro report its project root as
/// `node_modules/react-native` instead of the real project root, so
/// `react-native run-android`'s own dev-server check saw a mismatched
/// `X-React-Native-Project-Root` and treated an already-running Metro as a
/// conflicting process on the port rather than recognizing it. `abs_entry`
/// is already absolute, so the spawned process finds it regardless of cwd.
fn resolve_entry_and_run_dir(entry: &Path, cwd: &Path) -> (PathBuf, PathBuf) {
    let abs_entry = if entry.is_absolute() {
        entry.to_path_buf()
    } else {
        cwd.join(entry)
    };
    (abs_entry, cwd.to_path_buf())
}

/// Everything the supervisor needs to spawn and keep an app cohort alive:
/// the entry, its arguments, and the restart policy. Shared by the detached
/// supervisor (`3va start` → `__supervise`), the foreground `3va start
/// --attach` supervisor, and `3va restart`, so they can never drift apart on
/// which policy is in effect.
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub name: String,
    pub entry: PathBuf,
    pub args: Vec<String>,
    pub port: Option<u16>,
    pub instances: u32,
    /// Give up restarting after this many consecutive crashes.
    pub max_restarts: u32,
    /// Whether an unexpected exit should be respawned (`--no-autorestart`
    /// disables this). `3va stop` always wins regardless of this value.
    pub autorestart: bool,
    /// Starting value for the restart counter — 0 for a fresh `3va start`,
    /// `info.restarts + 1` for `3va restart`, so the count survives
    /// supervisor generations without racing the on-disk state.
    pub start_restarts: u32,
}

/// Spawn the managed daemon in the background: launches `3va __supervise`
/// (setsid-detached, like the old direct-`3va run` daemon) instead of the app
/// itself, so a supervisor process — not just the app — outlives this CLI
/// invocation and can restart the app on crash. `3va stop` still just sends
/// SIGTERM to `info.pid`; the supervisor traps it, drains its children, and
/// exits without respawning (see `run_supervisor`).
///
/// `cfg.autorestart == false` (from `--no-autorestart`) tells the supervisor
/// to mark the process `error` and exit when the app dies unexpectedly,
/// instead of respawning it.
pub fn start_managed(cfg: &SupervisorConfig, cwd: &Path) -> anyhow::Result<ProcessInfo> {
    ensure_dir()?;

    let log_file = log_path(&cfg.name);
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)?;

    let bin = std::env::current_exe()?;
    let (abs_entry, run_dir) = resolve_entry_and_run_dir(&cfg.entry, cwd);

    let mut cmd = std::process::Command::new(&bin);
    cmd.arg("__supervise")
        .arg("--name")
        .arg(&cfg.name)
        .arg("--instances")
        .arg(cfg.instances.to_string())
        .arg("--max-restarts")
        .arg(cfg.max_restarts.to_string())
        .arg("--start-restarts")
        .arg(cfg.start_restarts.to_string());
    if !cfg.autorestart {
        cmd.arg("--no-autorestart");
    }
    if let Some(p) = cfg.port {
        cmd.arg("--port").arg(p.to_string());
    }
    cmd.arg(&abs_entry);
    if !cfg.args.is_empty() {
        cmd.arg("--").args(&cfg.args);
    }
    cmd.current_dir(&run_dir)
        .stdout(log.try_clone()?)
        .stderr(log)
        .stdin(std::process::Stdio::null());

    // Start a new process group so the supervisor survives the parent's exit.
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            let ret = libc::setsid();
            if ret == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn process '{}': {}", cfg.name, e))?;

    let pid = child.id();

    // Detach — don't wait for the child. The supervisor is responsible for
    // its own children and for updating the saved ProcessInfo as it runs.
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    let info = ProcessInfo {
        name: cfg.name.clone(),
        entry: cfg.entry.clone(),
        pid,
        cwd: cwd.to_path_buf(),
        log_path: log_file,
        status: "running".to_string(),
        started_at: now(),
        restarts: 0,
        args: cfg.args.clone(),
        port: cfg.port,
        instances: cfg.instances,
        max_restarts: cfg.max_restarts,
        autorestart: cfg.autorestart,
        instance_pids: vec![],
    };

    save_process(&info)?;
    Ok(info)
}

/// Spawn one app instance (`3va run <entry>`) as a child of the supervisor.
///
/// `cluster` sets `VVVA_CLUSTER=1` so the HTTP server binds with
/// `SO_REUSEPORT`, letting `instances > 1` share the same port. `inherit_stdio`
/// is true only for `3va start --attach`, where the app's output should go
/// straight to the foreground terminal/container log instead of a file.
fn spawn_app_instance(
    entry: &Path,
    cwd: &Path,
    args: &[String],
    port: Option<u16>,
    cluster: bool,
    inherit_stdio: bool,
    log_file_path: &Path,
) -> anyhow::Result<tokio::process::Child> {
    let bin = std::env::current_exe()?;
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.arg("run");
    if let Some(p) = port {
        cmd.arg("--port").arg(p.to_string());
    }
    // See the note in the old start_process: CI=true plus --allow-env=CI
    // keeps dev-server CLIs (Vite, nodemon, ...) from treating a closed
    // stdin as "my terminal died" and exiting before they bind their port.
    cmd.env("CI", "true");
    cmd.arg("--allow-env=CI");
    if cluster {
        cmd.env("VVVA_CLUSTER", "1");
    }
    cmd.arg(entry);
    if !args.is_empty() {
        cmd.arg("--").args(args);
    }
    cmd.current_dir(cwd).stdin(std::process::Stdio::null());

    if inherit_stdio {
        cmd.stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
    } else {
        let log_out = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file_path)?;
        let log_err = log_out.try_clone()?;
        cmd.stdout(log_out).stderr(log_err);
    }

    cmd.spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn app instance: {}", e))
}

/// Resolves as soon as any one of `children` exits. Each child's `wait()` is
/// polled directly (not moved into a separate task) so the caller keeps
/// ownership and can still kill every child — including the ones that
/// haven't exited — once this returns.
async fn wait_for_any_exit(children: &mut [tokio::process::Child]) {
    let waits: Vec<_> = children
        .iter_mut()
        .map(|c| {
            Box::pin(c.wait())
                as std::pin::Pin<Box<dyn std::future::Future<Output = _> + Send + '_>>
        })
        .collect();
    let _ = futures::future::select_all(waits).await;
}

/// The supervisor loop: spawns `instances` app processes, waits for either a
/// shutdown signal or any instance exiting unexpectedly, and in the latter
/// case kills the remaining siblings and restarts the whole cohort together
/// with exponential backoff (`backoff_delay`), up to `max_restarts` times.
///
/// With `autorestart: false` (`3va start --no-autorestart`), an unexpected
/// exit is *not* respawned: the supervisor marks the process `error` and exits
/// (rather than lingering as a live-but-idle supervisor), so `3va status`
/// reflects reality and the user can still `3va restart` it manually.
///
/// `start_restarts` seeds the restart counter (0 for `3va start`, the persisted
/// count + 1 for `3va restart`) so it keeps accumulating across supervisor
/// generations instead of resetting to zero — see `restart_process`.
///
/// Runs either as the long-lived body of the detached `3va __supervise`
/// process (background `3va start`) or in-process as `3va start --attach`,
/// in which case it IS the foreground process a container should run as PID 1.
pub async fn run_supervisor(cfg: &SupervisorConfig, inherit_stdio: bool) -> anyhow::Result<()> {
    ensure_dir()?;
    let instances = cfg.instances.max(1);
    let cwd = std::env::current_dir()?;
    let log_file_path = log_path(&cfg.name);

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    let mut restarts = cfg.start_restarts;

    loop {
        let mut children = Vec::with_capacity(instances as usize);
        for _ in 0..instances {
            children.push(spawn_app_instance(
                &cfg.entry,
                &cwd,
                &cfg.args,
                cfg.port,
                instances > 1,
                inherit_stdio,
                &log_file_path,
            )?);
        }
        let pids: Vec<u32> = children.iter().filter_map(|c| c.id()).collect();

        {
            let mut info = load_process(&cfg.name).unwrap_or_else(|_| ProcessInfo {
                name: cfg.name.clone(),
                entry: cfg.entry.clone(),
                pid: std::process::id(),
                cwd: cwd.clone(),
                log_path: log_file_path.clone(),
                status: "running".to_string(),
                started_at: now(),
                restarts,
                args: cfg.args.clone(),
                port: cfg.port,
                instances: cfg.instances,
                max_restarts: cfg.max_restarts,
                autorestart: cfg.autorestart,
                instance_pids: vec![],
            });
            info.pid = std::process::id();
            info.status = "running".to_string();
            info.restarts = restarts;
            info.autorestart = cfg.autorestart;
            info.instance_pids = pids.clone();
            let _ = save_process(&info);
        }

        // `children` stays owned by this loop (not moved into spawned tasks) so
        // that whichever branch below fires, we can still reach in and kill the
        // survivors directly — moving each Child into its own task that awaits
        // `child.wait()` would leave no way to interrupt that wait from here.
        #[cfg(unix)]
        let shutdown = tokio::select! {
            _ = wait_for_any_exit(&mut children) => false,
            _ = sigterm.recv() => true,
            _ = tokio::signal::ctrl_c() => true,
        };
        #[cfg(not(unix))]
        let shutdown = tokio::select! {
            _ = wait_for_any_exit(&mut children) => false,
            _ = tokio::signal::ctrl_c() => true,
        };

        // Either an instance exited unexpectedly or we're shutting down —
        // either way, stop the remaining siblings so the cohort moves together.
        for child in &mut children {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }

        if shutdown {
            if let Ok(mut info) = load_process(&cfg.name) {
                info.status = "stopped".to_string();
                let _ = save_process(&info);
            }
            return Ok(());
        }

        // `3va stop`/`3va delete` may have raced the crash — don't respawn
        // something the user just asked to stop.
        if let Ok(info) = load_process(&cfg.name) {
            if info.status == "stopped" {
                return Ok(());
            }
        }

        // Autorestart disabled (`3va start --no-autorestart`): the app died
        // and we are explicitly told not to bring it back. Mark it `error`
        // (dead, intentionally not respawned) and exit the supervisor so
        // `3va status` doesn't keep reporting a live supervisor for a process
        // that will never be respawned. A manual `3va restart` still works.
        if !cfg.autorestart {
            if let Ok(mut info) = load_process(&cfg.name) {
                info.status = "error".to_string();
                let _ = save_process(&info);
            }
            return Ok(());
        }

        restarts += 1;
        if restarts > cfg.max_restarts {
            if let Ok(mut info) = load_process(&cfg.name) {
                info.status = "crashed".to_string();
                let _ = save_process(&info);
            }
            anyhow::bail!(
                "'{}' exited {} times in a row — giving up (--max-restarts {})",
                cfg.name,
                restarts,
                cfg.max_restarts
            );
        }
        // Exponential backoff: 500 ms, 1 s, 2 s, … up to a 30 s ceiling. See
        // `backoff_delay` for the rationale (bounds crash-loop CPU/log burn).
        tokio::time::sleep(backoff_delay(restarts)).await;
    }
}

/// Stop a managed process by name.
///
/// Sends SIGTERM (Unix) or `taskkill /PID` (Windows) and then **polls** every
/// 200 ms until either the process exits or 30 s have elapsed.  Only then is
/// SIGKILL (Unix) / `taskkill /F` (Windows) sent.
///
/// The polling approach — rather than a fixed 1.5 s sleep — means a process
/// that drains its WebSocket connections and exits in under a second will not
/// incur unnecessary latency, while long-lived drains (e.g. 1000 clients × 500 ms
/// jitter) are still given time to complete gracefully.
pub fn stop_process(name: &str) -> anyhow::Result<()> {
    let info = load_process(name)?;
    let pid = info.pid;

    if pid == 0 {
        anyhow::bail!("Process '{}' has invalid PID 0", name);
    }

    if !is_pid_alive(pid) {
        // Already dead, just clean up
        let mut updated = info;
        updated.status = "stopped".to_string();
        save_process(&updated)?;
        return Ok(());
    }

    // Try graceful shutdown (SIGTERM), poll for exit, then force-kill if needed.
    // Polling instead of a fixed sleep lets a fast-exiting process skip the wait
    // while still giving slow drainers up to 30 s before SIGKILL.
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let poll = std::time::Duration::from_millis(200);
        while is_pid_alive(pid) && std::time::Instant::now() < deadline {
            std::thread::sleep(poll);
        }
        if is_pid_alive(pid) {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string()])
            .status();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let poll = std::time::Duration::from_millis(200);
        while is_pid_alive(pid) && std::time::Instant::now() < deadline {
            std::thread::sleep(poll);
        }
        if is_pid_alive(pid) {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .status();
        }
    }

    let mut updated = info;
    updated.status = "stopped".to_string();
    save_process(&updated)?;
    Ok(())
}

/// Restart a managed process.
pub fn restart_process(name: &str) -> anyhow::Result<ProcessInfo> {
    let info = load_process(name)?;
    let restarts = info.restarts + 1;
    let cwd = info.cwd.clone();
    let cfg = SupervisorConfig {
        name: name.to_string(),
        entry: info.entry.clone(),
        args: info.args.clone(),
        port: info.port,
        instances: info.instances,
        max_restarts: info.max_restarts,
        autorestart: info.autorestart,
        start_restarts: restarts,
    };

    // Stop (ignore error if already stopped)
    let _ = stop_process(name);

    // Start again, passing the bumped counter straight to the new supervisor
    // (instead of racing the on-disk state after spawn), so the count keeps
    // accumulating across manual restarts instead of resetting to 0.
    let mut result = start_managed(&cfg, &cwd)?;
    result.restarts = restarts;
    save_process(&result)?;
    Ok(result)
}

/// Get status of a managed process.
pub fn status_process(name: &str) -> anyhow::Result<ProcessInfo> {
    let mut info = load_process(name)?;

    // Refresh status based on PID liveness
    if info.status == "running" && !is_pid_alive(info.pid) {
        info.status = "error".to_string();
        save_process(&info)?;
    }

    Ok(info)
}

/// List all managed processes with live status.
pub fn list_processes() -> Vec<ProcessInfo> {
    let mut processes = list_all_processes();

    // Refresh statuses
    for p in &mut processes {
        if p.status == "running" && !is_pid_alive(p.pid) {
            p.status = "error".to_string();
            let _ = save_process(p);
        }
    }

    processes
}

/// Print logs for a managed process (last N lines).
pub fn print_logs(name: &str, tail_lines: usize) -> anyhow::Result<()> {
    let info = load_process(name)?;
    let log_path = &info.log_path;

    if !log_path.exists() {
        println!("No logs yet for '{}'.", name);
        return Ok(());
    }

    let file = fs::File::open(log_path)?;
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

    let total = lines.len();
    let start = total.saturating_sub(tail_lines);

    for line in &lines[start..] {
        println!("{}", line);
    }

    if total == 0 {
        println!("(empty log file)");
    }

    Ok(())
}

/// Print log file path.
pub fn log_path_for(name: &str) -> anyhow::Result<PathBuf> {
    let info = load_process(name)?;
    Ok(info.log_path)
}

/// Delete a managed process (stop if running, then remove files).
pub fn delete_process(name: &str) -> anyhow::Result<()> {
    // Stop if running
    if let Ok(info) = load_process(name) {
        if info.status == "running" && is_pid_alive(info.pid) {
            stop_process(name)?;
        }
    }

    delete_process_file(name)?;

    // Remove log file
    let log = log_path(name);
    if log.exists() {
        fs::remove_file(&log)?;
    }

    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_pid_alive ──────────────────────────────────────────────────────────

    #[test]
    fn is_pid_alive_true_for_self() {
        let pid = std::process::id();
        assert!(is_pid_alive(pid), "the current process should be alive");
    }

    #[test]
    fn is_pid_alive_false_for_zero() {
        assert!(
            !is_pid_alive(0),
            "PID 0 is never a valid user-space process"
        );
    }

    /// Spawn a no-op child, wait for it to exit (reaping the zombie), then verify
    /// `is_pid_alive` returns `false`.  This exercises the kernel path that
    /// `stop_process` relies on to exit its polling loop early.
    #[cfg(unix)]
    #[test]
    fn is_pid_alive_false_after_child_exits() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap(); // reaps the zombie
        assert!(!is_pid_alive(pid), "PID {pid} should be dead after wait()");
    }

    // ── polling loop timing ───────────────────────────────────────────────────

    /// Verify that `stop_process` returns quickly when the target is already dead
    /// at the time of the call (the "already dead" early-return path, not the poll
    /// loop).  This is a regression guard against re-introducing a fixed sleep.
    #[cfg(unix)]
    #[test]
    fn stop_process_returns_fast_when_pid_already_gone() {
        use std::time::Instant;

        // Build a ProcessInfo referencing a reaped PID and write it to disk.
        ensure_dir().unwrap();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();

        let info = ProcessInfo {
            name: "__test_dead__".to_string(),
            entry: std::path::PathBuf::from("/dev/null"),
            pid,
            cwd: std::path::PathBuf::from("/tmp"),
            log_path: std::path::PathBuf::from("/tmp/__test_dead__.log"),
            status: "running".to_string(),
            started_at: 0,
            restarts: 0,
            args: vec![],
            port: None,
            instances: 1,
            max_restarts: 15,
            autorestart: true,
            instance_pids: vec![],
        };
        save_process(&info).unwrap();

        let t = Instant::now();
        let _ = stop_process("__test_dead__");
        let elapsed = t.elapsed();

        // Should return well under 1 s — there is no reason to wait for a dead PID.
        assert!(
            elapsed.as_millis() < 500,
            "stop_process took {}ms for a dead PID — fixed sleep may have been reintroduced",
            elapsed.as_millis()
        );
    }

    // ── backoff_delay ─────────────────────────────────────────────────────────

    #[test]
    fn backoff_delay_grows_exponentially_and_caps() {
        assert_eq!(backoff_delay(0), std::time::Duration::from_millis(500));
        assert_eq!(backoff_delay(1), std::time::Duration::from_millis(500));
        assert_eq!(backoff_delay(2), std::time::Duration::from_millis(1_000));
        assert_eq!(backoff_delay(3), std::time::Duration::from_millis(2_000));
        assert_eq!(backoff_delay(4), std::time::Duration::from_millis(4_000));
        assert_eq!(backoff_delay(5), std::time::Duration::from_millis(8_000));
        assert_eq!(backoff_delay(6), std::time::Duration::from_millis(16_000));
        // 2^6·500 ms = 32 s → clamped to the 30 s ceiling, and stays there.
        assert_eq!(backoff_delay(7), std::time::Duration::from_millis(30_000));
        assert_eq!(backoff_delay(8), std::time::Duration::from_millis(30_000));
        assert_eq!(
            backoff_delay(10_000),
            std::time::Duration::from_millis(30_000)
        );
    }

    #[test]
    fn backoff_delay_never_overflows_on_huge_count() {
        let d = backoff_delay(u32::MAX);
        assert_eq!(d, std::time::Duration::from_millis(30_000));
    }

    // ── ProcessInfo serde backward compatibility ──────────────────────────────

    /// `~/.3va/processes/*.json` files written by versions before the
    /// `autorestart` field existed must still load, defaulting to `true` (the
    /// pre-existing supervised behavior).
    #[test]
    fn process_info_deserializes_without_autorestart_field() {
        let legacy = r#"{
            "name": "legacy",
            "entry": "/tmp/legacy.js",
            "pid": 1234,
            "cwd": "/tmp",
            "log_path": "/tmp/legacy.log",
            "status": "running",
            "started_at": 0,
            "restarts": 3,
            "args": [],
            "port": null,
            "instances": 1,
            "max_restarts": 15
        }"#;
        let info: ProcessInfo = serde_json::from_str(legacy).unwrap();
        assert!(info.autorestart, "missing field must default to true");
        assert_eq!(info.restarts, 3);
        assert_eq!(info.instances, 1);
        assert_eq!(info.max_restarts, 15);
    }
}
