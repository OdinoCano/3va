// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Filesystem operations that can't be redirected between the permission
//! check and the syscall (VULN-01 TOCTOU).
//!
//! Checking a path and then letting the kernel resolve it again leaves a
//! window where a symlink anywhere along the path can be swapped (a worker
//! thread or an outside process). On Linux every operation here acts on an
//! object already pinned by a file descriptor whose real location
//! (`/proc/self/fd/N`) is what gets checked:
//!
//! - [`open`]: reads check the file actually opened; writes open inside a
//!   pinned, checked parent directory with `O_NOFOLLOW`.
//! - [`at`]: operations that don't follow the final component (unlink, rm,
//!   rename, mkdir, symlink, readlink, lchown, lutimes) run on
//!   `/proc/self/fd/<parent>/<name>`.
//! - [`object`]: operations that follow it (stat, readdir, realpath, chmod,
//!   chown, utimes) pin the object itself with `O_PATH` and check it.
//!
//! Elsewhere (or without `/proc`) they fall back to the plain check-then-act.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use vvva_permissions::{Capability, PermissionState};

fn cap(write: bool, p: PathBuf) -> Capability {
    if write {
        Capability::FileWrite(p)
    } else {
        Capability::FileRead(p)
    }
}

fn denied(p: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "EACCES: permission denied, {} resolves outside the granted paths",
            p.display()
        ),
    )
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    fn proc_available() -> bool {
        static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *OK.get_or_init(|| std::fs::read_link("/proc/self/exe").is_ok())
    }

    fn fd_path(f: &File) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", f.as_raw_fd()))
    }

    /// The real location of an open descriptor, checked against the grants.
    fn verify(perms: &PermissionState, f: &File, write: bool) -> io::Result<PathBuf> {
        let real = std::fs::read_link(fd_path(f))?;
        if perms.check(&cap(write, real.clone())) {
            Ok(real)
        } else {
            Err(denied(&real))
        }
    }

    fn o_path(p: &Path, extra: i32) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_CLOEXEC | extra)
            .open(p)
    }

    /// Parent directory pinned and checked; returns it with the final name.
    fn pin_parent(
        perms: &PermissionState,
        path: &Path,
        write: bool,
    ) -> io::Result<(File, std::ffi::OsString)> {
        let abs = std::path::absolute(path)?;
        let (Some(parent), Some(name)) = (abs.parent(), abs.file_name()) else {
            return Err(denied(path));
        };
        let dir = o_path(parent, libc::O_DIRECTORY)?;
        let real_parent = std::fs::read_link(fd_path(&dir))?;
        let target = real_parent.join(name);
        if !perms.check(&cap(write, target.clone())) {
            return Err(denied(&target));
        }
        Ok((dir, name.to_os_string()))
    }

    pub fn open(
        perms: &PermissionState,
        path: &Path,
        opts: &OpenOptions,
        write: bool,
    ) -> io::Result<File> {
        if !proc_available() {
            return opts.open(path);
        }
        if !write {
            let f = opts.open(path)?;
            verify(perms, &f, false)?;
            return Ok(f);
        }
        let mut path = path.to_path_buf();
        for _ in 0..8 {
            let (dir, name) = pin_parent(perms, &path, true)?;
            let anchored = fd_path(&dir).join(&name);
            let mut o = opts.clone();
            o.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            match o.open(&anchored) {
                Ok(f) => {
                    verify(perms, &f, true)?;
                    return Ok(f);
                }
                // Final component is a symlink: follow it by hand and
                // check where it points before touching anything.
                Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
                    let target = std::fs::read_link(&anchored)?;
                    path = std::fs::read_link(fd_path(&dir))?.join(target);
                }
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::from_raw_os_error(libc::ELOOP))
    }

    pub fn at<R>(
        perms: &PermissionState,
        path: &Path,
        write: bool,
        op: impl FnOnce(&Path) -> io::Result<R>,
    ) -> io::Result<R> {
        if !proc_available() {
            return op(path);
        }
        let (dir, name) = pin_parent(perms, path, write)?;
        let r = op(&fd_path(&dir).join(name));
        drop(dir);
        r
    }

    pub fn object<R>(
        perms: &PermissionState,
        path: &Path,
        write: bool,
        op: impl FnOnce(&Path, &Path) -> io::Result<R>,
    ) -> io::Result<R> {
        if !proc_available() {
            return op(path, path);
        }
        let f = o_path(path, 0)?;
        let real = verify(perms, &f, write)?;
        let r = op(&fd_path(&f), &real);
        drop(f);
        r
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::*;

    // ponytail: check-then-act elsewhere; macOS could pin with F_GETPATH.
    pub fn open(
        _perms: &PermissionState,
        path: &Path,
        opts: &OpenOptions,
        _write: bool,
    ) -> io::Result<File> {
        opts.open(path)
    }

    pub fn at<R>(
        _perms: &PermissionState,
        path: &Path,
        _write: bool,
        op: impl FnOnce(&Path) -> io::Result<R>,
    ) -> io::Result<R> {
        op(path)
    }

    pub fn object<R>(
        _perms: &PermissionState,
        path: &Path,
        _write: bool,
        op: impl FnOnce(&Path, &Path) -> io::Result<R>,
    ) -> io::Result<R> {
        op(path, path)
    }
}

/// Opens `path` so the file opened is the one the grants allow.
pub fn open(
    perms: &PermissionState,
    path: &Path,
    opts: &OpenOptions,
    write: bool,
) -> io::Result<File> {
    imp::open(perms, path, opts, write)
}

/// Runs `op` on `path` with its parent directory pinned and checked; for
/// operations that act on the final component itself.
pub fn at<R>(
    perms: &PermissionState,
    path: &Path,
    write: bool,
    op: impl FnOnce(&Path) -> io::Result<R>,
) -> io::Result<R> {
    imp::at(perms, path, write, op)
}

/// Pins the object `path` resolves to (following symlinks), checks its real
/// location, and runs `op(pinned_path, real_path)`.
pub fn object<R>(
    perms: &PermissionState,
    path: &Path,
    write: bool,
    op: impl FnOnce(&Path, &Path) -> io::Result<R>,
) -> io::Result<R> {
    imp::object(perms, path, write, op)
}

/// `create_dir_all` where every directory is created inside a pinned,
/// checked parent.
pub fn create_dir_all(perms: &PermissionState, path: &Path) -> io::Result<()> {
    let abs = std::path::absolute(path)?;
    let mut missing = Vec::new();
    let mut cur = abs.as_path();
    while !cur.exists() {
        missing.push(cur.to_path_buf());
        match cur.parent() {
            Some(p) => cur = p,
            None => break,
        }
    }
    for dir in missing.into_iter().rev() {
        match at(perms, &dir, true, |p| std::fs::create_dir(p)) {
            Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// Copies one file: the source as opened is checked for read, the
/// destination is created inside a checked parent. Keeps the mode bits.
pub fn copy_file(perms: &PermissionState, src: &Path, dst: &Path) -> io::Result<u64> {
    let mut from = open(perms, src, OpenOptions::new().read(true), false)?;
    let mut o = OpenOptions::new();
    o.write(true).create(true).truncate(true);
    let mut to = open(perms, dst, &o, true)?;
    let n = io::copy(&mut from, &mut to)?;
    to.set_permissions(from.metadata()?.permissions())?;
    Ok(n)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PathBuf, PermissionState) {
        let tmp = tempfile::tempdir().unwrap();
        let sandbox = tmp.path().join("sandbox");
        std::fs::create_dir_all(&sandbox).unwrap();
        std::fs::write(tmp.path().join("secret"), "s").unwrap();
        let perms = PermissionState::new();
        perms.grant(Capability::FileRead(sandbox.clone()));
        perms.grant(Capability::FileWrite(sandbox.clone()));
        (tmp, sandbox, perms)
    }

    #[test]
    fn a_symlink_swapped_in_after_the_check_is_caught_at_open() {
        // Simulates the race: the path is checked while it is a plain file,
        // then the file is replaced by a symlink pointing outside.
        let (tmp, sandbox, perms) = setup();
        let p = sandbox.join("f");
        std::fs::write(&p, "ok").unwrap();
        assert!(perms.check(&Capability::FileRead(p.clone())));
        std::fs::remove_file(&p).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("secret"), &p).unwrap();
        let err = open(&perms, &p, OpenOptions::new().read(true), false).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn writes_never_create_or_truncate_outside_the_grant() {
        let (tmp, sandbox, perms) = setup();
        std::os::unix::fs::symlink(tmp.path().join("secret"), sandbox.join("l")).unwrap();
        let mut o = OpenOptions::new();
        o.write(true).create(true).truncate(true);
        assert!(open(&perms, &sandbox.join("l"), &o, true).is_err());
        assert_eq!(std::fs::read(tmp.path().join("secret")).unwrap(), b"s");
        // A symlink inside the grant still works.
        std::fs::write(sandbox.join("real"), "x").unwrap();
        std::os::unix::fs::symlink(sandbox.join("real"), sandbox.join("inner")).unwrap();
        assert!(open(&perms, &sandbox.join("inner"), &o, true).is_ok());
        assert!(open(&perms, &sandbox.join("new"), &o, true).is_ok());
    }

    #[test]
    fn path_ops_run_in_the_checked_directory() {
        let (tmp, sandbox, perms) = setup();
        std::os::unix::fs::symlink(tmp.path(), sandbox.join("up")).unwrap();
        // Parent resolves outside: refused before the operation runs.
        assert!(
            at(&perms, &sandbox.join("up/secret"), true, |p| {
                std::fs::remove_file(p)
            })
            .is_err()
        );
        assert!(tmp.path().join("secret").exists());
        std::fs::write(sandbox.join("gone"), "").unwrap();
        at(&perms, &sandbox.join("gone"), true, |p| {
            std::fs::remove_file(p)
        })
        .unwrap();
        assert!(!sandbox.join("gone").exists());
    }
}

// The non-Linux fallback is a plain check-then-act passthrough (no /proc to pin
// through; macOS could pin with F_GETPATH). These tests pin down that the
// fallback still opens and operates on the given path — they only run where
// that fallback is actually used.
#[cfg(all(test, not(target_os = "linux")))]
mod fallback_tests {
    use super::*;

    fn sandbox() -> (tempfile::TempDir, PathBuf, PermissionState) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        (tmp, dir, PermissionState::new())
    }

    #[test]
    fn open_passes_through_to_the_plain_path() {
        let (tmp, dir, perms) = sandbox();
        let p = dir.join("f.txt");
        let mut o = OpenOptions::new();
        o.write(true).create_new(true);
        let f = open(&perms, &p, &o, true).unwrap();
        drop(f);
        assert!(
            p.exists(),
            "open() must create the file on the fallback path"
        );
        let _ = tmp;
    }

    #[test]
    fn read_open_returns_the_file_contents() {
        let (tmp, dir, perms) = sandbox();
        let p = dir.join("g.txt");
        std::fs::write(&p, "contents").unwrap();
        let f = open(&perms, &p, OpenOptions::new().read(true), false).unwrap();
        let mut s = String::new();
        use std::io::Read;
        (&f).read_to_string(&mut s).unwrap();
        assert_eq!(s, "contents");
        let _ = tmp;
    }

    #[test]
    fn at_and_object_delegate_to_the_given_path() {
        let (tmp, dir, perms) = sandbox();
        let p = dir.join("h.txt");
        std::fs::write(&p, "x").unwrap();
        let read = at(&perms, &p, false, |q| std::fs::read_to_string(q)).unwrap();
        assert_eq!(read, "x");
        let (a, b) = object(&perms, &p, false, |x, y| {
            Ok((x.to_path_buf(), y.to_path_buf()))
        })
        .unwrap();
        assert_eq!(a, p);
        assert_eq!(b, p, "object() must hand the op the same path twice");
        let _ = tmp;
    }
}
