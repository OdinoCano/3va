// Tests de path traversal contra VirtualFs::resolve.
// VirtualFs normaliza internamente los segmentos .. y . antes de buscar
// el mount point, por lo que un path como /app/../etc/passwd se resuelve
// a /etc/passwd, que no está montado → error.
// Ver docs/06-permissions/03-sandboxing.md §3.4

use std::path::Path;
use tempfile::TempDir;
use vvva_permissions::sandbox::VirtualFs;

fn make_vfs_with_sandbox() -> (TempDir, VirtualFs) {
    let temp = TempDir::new().unwrap();
    let sandbox_dir = temp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox_dir).unwrap();
    let mut vfs = VirtualFs::new();
    vfs.mount("/app", &sandbox_dir, true);
    (temp, vfs)
}

// ── Ataques de traversal básicos ──────────────────────────────────────────────

#[test]
fn traversal_via_dotdot_rejected() {
    let (_temp, vfs) = make_vfs_with_sandbox();
    // /app/../etc/passwd → normaliza a /etc/passwd → no montado
    let result = vfs.resolve(Path::new("/app/../etc/passwd"));
    assert!(result.is_err(), "traversal con .. debe ser rechazado");
}

#[test]
fn absolute_path_outside_mount_rejected() {
    let (_temp, vfs) = make_vfs_with_sandbox();
    let result = vfs.resolve(Path::new("/etc/passwd"));
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "Path not mounted");
}

#[test]
fn deep_nested_traversal_rejected() {
    let (_temp, vfs) = make_vfs_with_sandbox();
    // Múltiples niveles de .. intentando salir
    let result = vfs.resolve(Path::new("/app/a/b/c/../../../../../../../etc/shadow"));
    assert!(result.is_err());
}

#[test]
fn traversal_to_sibling_directory_rejected() {
    let temp = TempDir::new().unwrap();
    let sandbox = temp.path().join("sandbox");
    let secrets = temp.path().join("secrets");
    std::fs::create_dir_all(&sandbox).unwrap();
    std::fs::create_dir_all(&secrets).unwrap();

    let mut vfs = VirtualFs::new();
    vfs.mount("/app", &sandbox, true);

    // /app/../secrets/key → normaliza a /<tempdir>/secrets/key → no montado
    let result = vfs.resolve(Path::new("/app/../secrets/key"));
    assert!(result.is_err());
}

// ── Normalización de segmentos especiales ─────────────────────────────────────

#[test]
fn dot_segment_normalized_stays_inside_mount() {
    let (_temp, vfs) = make_vfs_with_sandbox();
    // /app/./config.json → /app/config.json (válido)
    let result = vfs.resolve(Path::new("/app/./config.json"));
    assert!(result.is_ok(), "segmento . debe normalizarse sin error");
}

#[test]
fn multiple_slashes_within_mount_accepted() {
    let (_temp, vfs) = make_vfs_with_sandbox();
    // /app/subdir/file.json (path normal dentro del mount)
    let result = vfs.resolve(Path::new("/app/subdir/file.json"));
    assert!(result.is_ok());
}

// ── Resolución correcta dentro del mount ─────────────────────────────────────

#[test]
fn valid_nested_path_resolves_to_real_path() {
    let temp = TempDir::new().unwrap();
    let sandbox = temp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();

    let mut vfs = VirtualFs::new();
    vfs.mount("/app", &sandbox, true);

    let resolved = vfs.resolve(Path::new("/app/config.json")).unwrap();
    assert_eq!(resolved, sandbox.join("config.json"));
}

#[test]
fn deep_valid_path_resolves_correctly() {
    let temp = TempDir::new().unwrap();
    let sandbox = temp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();

    let mut vfs = VirtualFs::new();
    vfs.mount("/app", &sandbox, true);

    let resolved = vfs.resolve(Path::new("/app/subdir/nested/file.json")).unwrap();
    assert_eq!(resolved, sandbox.join("subdir/nested/file.json"));
}

// ── Múltiples mounts: solo el mount correspondiente resuelve ─────────────────

#[test]
fn path_resolves_via_correct_mount() {
    let temp = TempDir::new().unwrap();
    let dir1 = temp.path().join("dir1");
    let dir2 = temp.path().join("dir2");
    std::fs::create_dir_all(&dir1).unwrap();
    std::fs::create_dir_all(&dir2).unwrap();

    let mut vfs = VirtualFs::new();
    vfs.mount("/app", &dir1, false);
    vfs.mount("/data", &dir2, false);

    let r1 = vfs.resolve(Path::new("/app/file.js")).unwrap();
    let r2 = vfs.resolve(Path::new("/data/file.js")).unwrap();
    assert_eq!(r1, dir1.join("file.js"));
    assert_eq!(r2, dir2.join("file.js"));
}

#[test]
fn unmounted_path_always_rejected() {
    let (_temp, vfs) = make_vfs_with_sandbox();
    // Ningún mount cubre /tmp
    let result = vfs.resolve(Path::new("/tmp/evil"));
    assert!(result.is_err());
}
