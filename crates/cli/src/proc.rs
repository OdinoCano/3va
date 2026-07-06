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

/// Resolve `entry` to an absolute path against `cwd`, and the directory the
/// app should run from — otherwise a relative entry (e.g. "index.js") no
/// longer points at the right file once the child's cwd changes.
fn resolve_entry_and_run_dir(entry: &Path, cwd: &Path) -> (PathBuf, PathBuf) {
    let abs_entry = if entry.is_absolute() {
        entry.to_path_buf()
    } else {
        cwd.join(entry)
    };
    let run_dir = abs_entry
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| cwd.to_path_buf());
    (abs_entry, run_dir)
}

/// Spawn the managed daemon in the background: launches `3va __supervise`
/// (setsid-detached, like the old direct-`3va run` daemon) instead of the app
/// itself, so a supervisor process — not just the app — outlives this CLI
/// invocation and can restart the app on crash. `3va stop` still just sends
/// SIGTERM to `info.pid`; the supervisor traps it, drains its children, and
/// exits without respawning (see `run_supervisor`).
pub fn start_managed(
    name: &str,
    entry: &Path,
    cwd: &Path,
    args: &[String],
    port: Option<u16>,
    instances: u32,
    max_restarts: u32,
) -> anyhow::Result<ProcessInfo> {
    ensure_dir()?;

    let log_file = log_path(name);
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)?;

    let bin = std::env::current_exe()?;
    let (abs_entry, run_dir) = resolve_entry_and_run_dir(entry, cwd);

    let mut cmd = std::process::Command::new(&bin);
    cmd.arg("__supervise")
        .arg("--name")
        .arg(name)
        .arg("--instances")
        .arg(instances.to_string())
        .arg("--max-restarts")
        .arg(max_restarts.to_string());
    if let Some(p) = port {
        cmd.arg("--port").arg(p.to_string());
    }
    cmd.arg(&abs_entry);
    if !args.is_empty() {
        cmd.arg("--").args(args);
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
        .map_err(|e| anyhow::anyhow!("Failed to spawn process '{}': {}", name, e))?;

    let pid = child.id();

    // Detach — don't wait for the child. The supervisor is responsible for
    // its own children and for updating the saved ProcessInfo as it runs.
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    let info = ProcessInfo {
        name: name.to_string(),
        entry: entry.to_path_buf(),
        pid,
        cwd: cwd.to_path_buf(),
        log_path: log_file,
        status: "running".to_string(),
        started_at: now(),
        restarts: 0,
        args: args.to_vec(),
        port,
        instances,
        max_restarts,
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
    cmd.arg(entry)
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null());

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
/// (with linear backoff), up to `max_restarts` times.
///
/// Runs either as the long-lived body of the detached `3va __supervise`
/// process (background `3va start`) or in-process as `3va start --attach`,
/// in which case it IS the foreground process a container should run as PID 1.
pub async fn run_supervisor(
    name: &str,
    entry: &Path,
    args: &[String],
    port: Option<u16>,
    instances: u32,
    max_restarts: u32,
    inherit_stdio: bool,
) -> anyhow::Result<()> {
    ensure_dir()?;
    let instances = instances.max(1);
    let cwd = std::env::current_dir()?;
    let log_file_path = log_path(name);

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    let mut restarts = 0u32;

    loop {
        let mut children = Vec::with_capacity(instances as usize);
        for _ in 0..instances {
            children.push(spawn_app_instance(
                entry,
                &cwd,
                args,
                port,
                instances > 1,
                inherit_stdio,
                &log_file_path,
            )?);
        }
        let pids: Vec<u32> = children.iter().filter_map(|c| c.id()).collect();

        {
            let mut info = load_process(name).unwrap_or_else(|_| ProcessInfo {
                name: name.to_string(),
                entry: entry.to_path_buf(),
                pid: std::process::id(),
                cwd: cwd.clone(),
                log_path: log_file_path.clone(),
                status: "running".to_string(),
                started_at: now(),
                restarts,
                args: args.to_vec(),
                port,
                instances,
                max_restarts,
                instance_pids: vec![],
            });
            info.pid = std::process::id();
            info.status = "running".to_string();
            info.restarts = restarts;
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
            if let Ok(mut info) = load_process(name) {
                info.status = "stopped".to_string();
                let _ = save_process(&info);
            }
            return Ok(());
        }

        // `3va stop`/`3va delete` may have raced the crash — don't respawn
        // something the user just asked to stop.
        if let Ok(info) = load_process(name) {
            if info.status == "stopped" {
                return Ok(());
            }
        }

        restarts += 1;
        if restarts > max_restarts {
            if let Ok(mut info) = load_process(name) {
                info.status = "crashed".to_string();
                let _ = save_process(&info);
            }
            anyhow::bail!(
                "'{}' exited {} times in a row — giving up (--max-restarts {})",
                name,
                restarts,
                max_restarts
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(
            300 * restarts.min(10) as u64,
        ))
        .await;
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
    let entry = info.entry.clone();
    let args = info.args.clone();

    // Stop (ignore error if already stopped)
    let _ = stop_process(name);

    // Start again (resets restarts to 0), then restore count
    let mut result = start_managed(
        name,
        &entry,
        &cwd,
        &args,
        info.port,
        info.instances,
        info.max_restarts,
    )?;
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
}
