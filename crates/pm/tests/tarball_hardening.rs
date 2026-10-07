// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Tarballs maliciosos contra el extractor real (`ContentStore::store_tarball`).
//!
//! Confirma que un tarball servido por un registro comprometido no puede
//! escribir fuera del paquete: entradas `../`, rutas absolutas, symlinks y
//! hardlinks que apuntan fuera son rechazados o contenidos. Cada caso incluye
//! una entrada benigna hermana (`package/ok.txt`) para probar que el archivo se
//! leyó y que fue la guarda —no un fallo del lector— la que bloqueó la entrada.

use std::path::Path;
use vvva_pm::store::ContentStore;

fn raw_entry(
    name: &[u8],
    kind: tar::EntryType,
    linkname: Option<&[u8]>,
    data: &[u8],
) -> tar::Header {
    let mut h = tar::Header::new_gnu();
    h.set_size(data.len() as u64);
    h.set_mode(0o644);
    h.set_entry_type(kind);
    {
        let old = h.as_old_mut();
        old.name = [0u8; 100];
        old.name[..name.len()].copy_from_slice(name);
        old.linkname = [0u8; 100];
        if let Some(l) = linkname {
            old.linkname[..l.len()].copy_from_slice(l);
        }
    }
    h.set_cksum();
    h
}

fn tar_bytes(entries: &[(tar::Header, Vec<u8>)]) -> Vec<u8> {
    let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut b = tar::Builder::new(enc);
    for (h, d) in entries {
        b.append(h, &d[..]).unwrap();
    }
    b.into_inner().unwrap().finish().unwrap()
}

fn store_of(tmp: &tempfile::TempDir) -> (ContentStore, std::path::PathBuf) {
    let root = tmp.path().join("store");
    (ContentStore::with_root(root.clone()), root)
}

#[test]
fn parent_dir_entry_does_not_escape_the_store() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, root) = store_of(&tmp);
    let tgz = tar_bytes(&[
        (
            raw_entry(b"package/ok.txt", tar::EntryType::Regular, None, b"ok"),
            b"ok".to_vec(),
        ),
        (
            raw_entry(
                b"package/../../pwned.txt",
                tar::EntryType::Regular,
                None,
                b"pwned",
            ),
            b"pwned".to_vec(),
        ),
    ]);

    let dest = store.store_tarball(&tgz, "npm", "evil", "1.0.0").unwrap();
    assert!(dest.join("ok.txt").exists(), "la entrada benigna se extrae");
    // El escape esperado sería store_root/pwned.txt (dos `..` desde tmp).
    assert!(
        !root.join("pwned.txt").exists(),
        "una entrada ../../ escribió fuera del paquete"
    );
    assert!(!tmp.path().join("pwned.txt").exists());
}

#[test]
fn absolute_path_entry_is_contained_inside_the_package() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _root) = store_of(&tmp);
    let tgz = tar_bytes(&[
        (
            raw_entry(b"package/ok.txt", tar::EntryType::Regular, None, b"ok"),
            b"ok".to_vec(),
        ),
        (
            raw_entry(b"/pwned-abs.txt", tar::EntryType::Regular, None, b"pwned"),
            b"pwned".to_vec(),
        ),
    ]);

    let dest = store.store_tarball(&tgz, "npm", "evil", "1.0.0").unwrap();
    assert!(dest.join("ok.txt").exists());
    assert!(
        !Path::new("/pwned-abs.txt").exists(),
        "una ruta absoluta escribió en la raíz del filesystem"
    );
}

#[test]
fn symlink_entry_pointing_outside_is_not_materialized() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _root) = store_of(&tmp);
    let tgz = tar_bytes(&[
        (
            raw_entry(b"package/ok.txt", tar::EntryType::Regular, None, b"ok"),
            b"ok".to_vec(),
        ),
        (
            raw_entry(
                b"package/link",
                tar::EntryType::Symlink,
                Some(b"../../../../etc/passwd"),
                b"",
            ),
            Vec::new(),
        ),
    ]);

    let dest = store.store_tarball(&tgz, "npm", "evil", "1.0.0").unwrap();
    assert!(dest.join("ok.txt").exists());
    assert!(
        dest.join("link").symlink_metadata().is_err(),
        "se materializó un symlink que apunta fuera del paquete"
    );
}

#[test]
fn hardlink_entry_pointing_outside_is_not_materialized() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _root) = store_of(&tmp);
    let tgz = tar_bytes(&[
        (
            raw_entry(b"package/ok.txt", tar::EntryType::Regular, None, b"ok"),
            b"ok".to_vec(),
        ),
        (
            raw_entry(
                b"package/hard",
                tar::EntryType::Link,
                Some(b"../../../../etc/passwd"),
                b"",
            ),
            Vec::new(),
        ),
    ]);

    let dest = store.store_tarball(&tgz, "npm", "evil", "1.0.0").unwrap();
    assert!(dest.join("ok.txt").exists());
    assert!(
        dest.join("hard").symlink_metadata().is_err(),
        "se materializó un hardlink que apunta fuera del paquete"
    );
}
