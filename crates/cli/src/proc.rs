use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Managed process metadata.
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

/// Spawn a process in the background with logging.
pub fn start_process(
    name: &str,
    entry: &Path,
    cwd: &Path,
    args: &[String],
) -> anyhow::Result<ProcessInfo> {
    ensure_dir()?;

    let log_file = log_path(name);
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)?;

    // Use 3va run to execute the entry file
    let bin = std::env::current_exe()?;

    let mut cmd = std::process::Command::new(&bin);
    cmd.arg("run")
        .arg(entry)
        .args(args)
        .current_dir(cwd)
        .stdout(log.try_clone()?)
        .stderr(log)
        .stdin(std::process::Stdio::null());

    // Start a new process group so the child survives the parent's exit
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

    // Detach — don't wait for the child
    // On Unix, because we created a new session with setsid(), the child continues
    // even when the parent exits. The child handle is dropped to avoid zombies.
    // The child's stdio is connected to the log file, not the terminal.
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
    };

    save_process(&info)?;
    Ok(info)
}

/// Stop a managed process by name.
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

    // Try graceful shutdown (SIGTERM), then force (SIGKILL)
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        // Wait a bit for graceful shutdown
        std::thread::sleep(std::time::Duration::from_millis(1500));
        if is_pid_alive(pid) {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }

    #[cfg(not(unix))]
    {
        // Try graceful shutdown first (WM_CLOSE), then force
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string()])
            .status();
        std::thread::sleep(std::time::Duration::from_millis(1500));
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
    let mut result = start_process(name, &entry, &cwd, &args)?;
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
