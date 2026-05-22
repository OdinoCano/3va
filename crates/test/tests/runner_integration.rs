// Pruebas de integración para TestRunner: verifica el pipeline completo
// desde archivos .test.js/.test.ts hasta resultados ParseadosD.
//
// Ejecutar: cargo test -p vvva_test --test runner_integration

use std::fs;
use tempfile::TempDir;
use vvva_test::{TestConfig, TestRunner, TestStatus};

fn temp_test(content: &str, filename: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(filename);
    fs::write(&path, content).unwrap();
    (dir, path)
}

// ── Casos básicos ─────────────────────────────────────────────────────────────

#[test]
fn runner_passes_simple_test() {
    let (_dir, path) = temp_test(
        r#"
        test('suma básica', () => {
            expect(1 + 1).toBe(2);
        });
        "#,
        "simple.test.js",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, TestStatus::Passed, "test debe pasar");
    assert_eq!(results[0].name, "suma básica");
}

#[test]
fn runner_reports_failing_test() {
    let (_dir, path) = temp_test(
        r#"
        test('falla intencional', () => {
            expect(1).toBe(2);
        });
        "#,
        "fail.test.js",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, TestStatus::Failed);
    assert!(results[0].error.is_some(), "debe incluir mensaje de error");
}

#[test]
fn runner_handles_multiple_tests_in_one_file() {
    let (_dir, path) = temp_test(
        r#"
        test('pasa', () => { expect(true).toBeTruthy(); });
        test('también pasa', () => { expect('hello').toContain('ell'); });
        test('falla', () => { expect(1).toBe(999); });
        "#,
        "multi.test.js",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 3);

    let passed = results
        .iter()
        .filter(|r| r.status == TestStatus::Passed)
        .count();
    let failed = results
        .iter()
        .filter(|r| r.status == TestStatus::Failed)
        .count();
    assert_eq!(passed, 2);
    assert_eq!(failed, 1);
}

// ── describe / it ─────────────────────────────────────────────────────────────

#[test]
fn runner_supports_describe_blocks() {
    let (_dir, path) = temp_test(
        r#"
        describe('Matemáticas', () => {
            it('suma', () => { expect(2 + 3).toBe(5); });
            it('resta', () => { expect(5 - 2).toBe(3); });
        });
        "#,
        "describe.test.js",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 2);
    assert!(
        results[0].name.contains("Matemáticas"),
        "nombre debe incluir suite"
    );
    assert!(results[0].name.contains("suma"));
    assert!(results.iter().all(|r| r.status == TestStatus::Passed));
}

// ── TypeScript ────────────────────────────────────────────────────────────────

#[test]
fn runner_runs_typescript_test_file() {
    let (_dir, path) = temp_test(
        r#"
        function add(a: number, b: number): number {
            return a + b;
        }

        test('TypeScript con tipos', () => {
            expect(add(10, 32)).toBe(42);
        });
        "#,
        "types.test.ts",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, TestStatus::Passed);
}

// ── Matchers ─────────────────────────────────────────────────────────────────

#[test]
fn runner_supports_all_core_matchers() {
    let (_dir, path) = temp_test(
        r#"
        test('toEqual con objeto', () => {
            expect({ a: 1, b: 2 }).toEqual({ a: 1, b: 2 });
        });
        test('toMatch con regex', () => {
            expect('hello world').toMatch(/world/);
        });
        test('toHaveLength', () => {
            expect([1, 2, 3]).toHaveLength(3);
        });
        test('toBeGreaterThan', () => {
            expect(10).toBeGreaterThan(5);
        });
        test('toBeLessThanOrEqual', () => {
            expect(5).toBeLessThanOrEqual(5);
        });
        test('toThrow', () => {
            expect(() => { throw new Error('boom'); }).toThrow('boom');
        });
        test('.not.toBe', () => {
            expect(1).not.toBe(2);
        });
        "#,
        "matchers.test.js",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 7);
    assert!(
        results.iter().all(|r| r.status == TestStatus::Passed),
        "todos los matchers deben pasar: {:?}",
        results
            .iter()
            .filter(|r| r.status != TestStatus::Passed)
            .collect::<Vec<_>>()
    );
}

// ── run_directory ─────────────────────────────────────────────────────────────

#[test]
fn runner_discovers_test_files_in_directory() {
    let dir = TempDir::new().unwrap();

    fs::write(
        dir.path().join("a.test.js"),
        "test('a', () => { expect(1).toBe(1); });",
    )
    .unwrap();
    fs::write(
        dir.path().join("b.spec.ts"),
        "test('b', () => { expect(2).toBe(2); });",
    )
    .unwrap();
    // Non-test file — should be ignored
    fs::write(dir.path().join("helper.js"), "const x = 1;").unwrap();

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_directory(dir.path()).unwrap();

    let results = runner.get_results();
    assert_eq!(
        results.len(),
        2,
        "debe encontrar exactamente 2 tests (a y b)"
    );
    assert!(results.iter().all(|r| r.status == TestStatus::Passed));
}

// ── Snapshots ────────────────────────────────────────────────────────────────

#[test]
fn runner_creates_snapshot_on_first_run() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("snap.test.js");

    fs::write(
        &path,
        r#"
        test('snapshot', () => {
            expect({ name: 'edgar', role: 'admin' }).toMatchSnapshot();
        });
        "#,
    )
    .unwrap();

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).unwrap();

    // First run always creates snapshot → should pass
    let results = runner.get_results();
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].status,
        TestStatus::Passed,
        "primer snapshot siempre pasa (lo crea)"
    );

    // Snapshot file must exist
    let snap_dir = dir.path().join("__snapshots__").join("snap.test.js");
    let snap_file = format!("{}.snap.json", snap_dir.to_string_lossy());
    assert!(
        std::path::Path::new(&snap_file).exists(),
        "archivo de snapshot debe existir en __snapshots__/"
    );
}

#[test]
fn runner_snapshot_fails_on_mismatch() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("mismatch.test.js");

    fs::write(
        &path,
        r#"test('snap', () => { expect('valor_a').toMatchSnapshot(); });"#,
    )
    .unwrap();

    // First run: creates snapshot with "valor_a"
    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).unwrap();
    assert_eq!(runner.get_results()[0].status, TestStatus::Passed);

    // Mutate test to produce different value
    fs::write(
        &path,
        r#"test('snap', () => { expect('valor_b').toMatchSnapshot(); });"#,
    )
    .unwrap();

    // Second run: should fail due to mismatch
    let mut runner2 = TestRunner::new(TestConfig::default());
    runner2.run_file(&path).unwrap();
    assert_eq!(
        runner2.get_results()[0].status,
        TestStatus::Failed,
        "snapshot mismatch debe fallar"
    );
}

#[test]
fn runner_update_snapshots_flag_rewrites_stored_value() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("update.test.js");

    fs::write(
        &path,
        r#"test('snap', () => { expect('valor_original').toMatchSnapshot(); });"#,
    )
    .unwrap();

    // First run: create
    TestRunner::new(TestConfig::default())
        .run_file(&path)
        .unwrap();

    // Change value
    fs::write(
        &path,
        r#"test('snap', () => { expect('valor_nuevo').toMatchSnapshot(); });"#,
    )
    .unwrap();

    // Run with update flag: should pass and update
    let cfg = TestConfig {
        update_snapshots: true,
        ..Default::default()
    };
    let mut runner = TestRunner::new(cfg);
    runner.run_file(&path).unwrap();
    assert_eq!(
        runner.get_results()[0].status,
        TestStatus::Passed,
        "--update-snapshots debe actualizar y pasar"
    );

    // Next run without flag: should now pass with new value
    let mut runner2 = TestRunner::new(TestConfig::default());
    runner2.run_file(&path).unwrap();
    assert_eq!(runner2.get_results()[0].status, TestStatus::Passed);
}

// ── Syntax errors ────────────────────────────────────────────────────────────

#[test]
fn runner_reports_syntax_error_as_failed_test() {
    let (_dir, path) = temp_test("const x = ;;;  // syntax error", "broken.test.js");

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].status,
        TestStatus::Failed,
        "error de sintaxis debe reportarse como test fallido"
    );
}
