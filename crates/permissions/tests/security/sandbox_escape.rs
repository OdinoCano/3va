// Tests de aislamiento del sandbox contra intentos de escape.
// Cubre VirtualFs (aislamiento de archivos) y VirtualNetwork (aislamiento de red)
// documentados en docs/06-permissions/03-sandboxing.md §3.4 y §3.5.
//
// Nota sobre symlinks: VirtualFs::resolve trabaja a nivel de path lógico
// (normaliza .. y .) pero no canonicaliza con el FS real, por lo que el
// seguimiento de symlinks es responsabilidad de la capa OS / FsEnforcer.
// Los tests de symlinks documentan esta frontera de responsabilidad.

use std::path::Path;
use tempfile::TempDir;
use vvva_permissions::sandbox::{VirtualFs, VirtualNetwork};

// ── Aislamiento de mounts: un mount no da acceso a otro ──────────────────────

#[test]
fn mount_isolation_prevents_cross_mount_access() {
    let temp = TempDir::new().unwrap();
    let app_dir = temp.path().join("app");
    let secrets_dir = temp.path().join("secrets");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::create_dir_all(&secrets_dir).unwrap();

    let mut vfs = VirtualFs::new();
    vfs.mount("/app", &app_dir, false);
    // /secrets NO está montado intencionalmente

    let result = vfs.resolve(Path::new("/secrets/key.pem"));
    assert!(result.is_err(), "/secrets no montado debe ser rechazado");
}

#[test]
fn unmounted_root_is_inaccessible() {
    let temp = TempDir::new().unwrap();
    let app_dir = temp.path().join("app");
    std::fs::create_dir_all(&app_dir).unwrap();

    let mut vfs = VirtualFs::new();
    vfs.mount("/app", &app_dir, true);

    // La raíz del FS real no está montada
    let result = vfs.resolve(Path::new("/"));
    assert!(result.is_err());
}

#[test]
fn proc_fs_not_accessible_without_mount() {
    let temp = TempDir::new().unwrap();
    let app_dir = temp.path().join("app");
    std::fs::create_dir_all(&app_dir).unwrap();

    let mut vfs = VirtualFs::new();
    vfs.mount("/app", &app_dir, true);

    // /proc no debe resolverse a menos que esté montado explícitamente
    assert!(vfs.resolve(Path::new("/proc/self/environ")).is_err());
    assert!(vfs.resolve(Path::new("/proc/1/cmdline")).is_err());
    assert!(vfs.resolve(Path::new("/dev/shm/evil")).is_err());
}

// ── Symlinks: VirtualFs no sigue symlinks (frontera de responsabilidad) ───────

#[cfg(unix)]
#[test]
fn symlink_inside_mount_resolves_to_real_path_under_source() {
    let temp = TempDir::new().unwrap();
    let sandbox = temp.path().join("sandbox");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&sandbox).unwrap();
    std::fs::create_dir_all(&outside).unwrap();

    // Crear un symlink dentro del sandbox que apunta afuera
    let link = sandbox.join("escape");
    std::os::unix::fs::symlink(&outside, &link).unwrap();

    let mut vfs = VirtualFs::new();
    vfs.mount("/app", &sandbox, true);

    // VirtualFs resuelve /app/escape a sandbox/escape (path lógico dentro del mount).
    // No sigue el symlink al nivel de VirtualFs — eso es tarea del OS / FsEnforcer.
    let resolved = vfs.resolve(Path::new("/app/escape/secret.txt")).unwrap();
    assert!(
        resolved.starts_with(&sandbox),
        "VirtualFs debe anclar la resolución al source del mount"
    );
}

#[cfg(unix)]
#[test]
fn traversal_via_dotdot_rejected_even_with_symlink_present() {
    let temp = TempDir::new().unwrap();
    let sandbox = temp.path().join("sandbox");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&sandbox).unwrap();
    std::fs::create_dir_all(&outside).unwrap();

    let link = sandbox.join("link");
    std::os::unix::fs::symlink(&outside, &link).unwrap();

    let mut vfs = VirtualFs::new();
    vfs.mount("/app", &sandbox, true);

    // El ataque de traversal clásico: salir por .. aunque haya symlinks
    let result = vfs.resolve(Path::new("/app/../outside/secret"));
    assert!(result.is_err());
}

// ── VirtualNetwork: aislamiento de hosts ─────────────────────────────────────
// Espeja los registros usados en scripts/integration_tests.sh:
//   --allow-net=registry.npmjs.org
//   --allow-net=registry.yarnpkg.com
//   --allow-net=jsr.io

#[test]
fn virtual_network_empty_denies_all_hosts() {
    let vnet = VirtualNetwork::new();
    assert!(!vnet.is_allowed("registry.npmjs.org"));
    assert!(!vnet.is_allowed("registry.yarnpkg.com"));
    assert!(!vnet.is_allowed("jsr.io"));
}

#[test]
fn virtual_network_exact_host_allows_only_that_host() {
    let mut vnet = VirtualNetwork::new();
    vnet.allow_host("registry.npmjs.org");

    assert!(vnet.is_allowed("registry.npmjs.org"));
    // Subdomain distinto → denegado
    assert!(!vnet.is_allowed("api.registry.npmjs.org"));
    // Dominio diferente → denegado
    assert!(!vnet.is_allowed("registry.yarnpkg.com"));
}

#[test]
fn virtual_network_wildcard_subdomain_not_matches_parent() {
    // *.registry.npmjs.org NO cubre registry.npmjs.org (sin subdominio)
    // (clase de CVE: domain bypass a través del dominio padre)
    let mut vnet = VirtualNetwork::new();
    vnet.allow_host("*.registry.npmjs.org");

    assert!(vnet.is_allowed("api.registry.npmjs.org"));
    assert!(
        !vnet.is_allowed("registry.npmjs.org"),
        "el dominio padre no debe estar cubierto por wildcard"
    );
}

#[test]
fn virtual_network_wildcard_not_matches_evil_suffix() {
    // Protección contra ataques de sufijo: registry.npmjs.org.evil.com
    let mut vnet = VirtualNetwork::new();
    vnet.allow_host("*.npmjs.org");

    assert!(vnet.is_allowed("registry.npmjs.org"));
    assert!(!vnet.is_allowed("registry.npmjs.org.evil.com"));
    assert!(!vnet.is_allowed("evil.com"));
}

#[test]
fn virtual_network_multiple_registries_coexist() {
    // Refleja la fase 1-3 de scripts/integration_tests.sh:
    // npm + yarn + jsr coexisten sin otorgarse acceso mutuo
    let mut vnet = VirtualNetwork::new();
    vnet.allow_host("registry.npmjs.org");
    vnet.allow_host("registry.yarnpkg.com");
    vnet.allow_host("jsr.io");

    assert!(vnet.is_allowed("registry.npmjs.org"));
    assert!(vnet.is_allowed("registry.yarnpkg.com"));
    assert!(vnet.is_allowed("jsr.io"));
    // Hosts no listados siguen denegados
    assert!(!vnet.is_allowed("evil.com"));
    assert!(!vnet.is_allowed("npm.malicious.io"));
}

#[test]
fn virtual_network_star_wildcard_allows_all() {
    // Network("*") equivale a --allow-net sin argumentos
    let mut vnet = VirtualNetwork::new();
    vnet.allow_host("*");

    assert!(vnet.is_allowed("registry.npmjs.org"));
    assert!(vnet.is_allowed("evil.com"));
    assert!(vnet.is_allowed("127.0.0.1"));
}
