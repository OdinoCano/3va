// Pruebas de integración para TestRunner: verifica el pipeline completo
// desde archivos .test.js/.test.ts hasta resultados Parseados.
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

#[tokio::test]
async fn runner_passes_simple_test() {
    let (_dir, path) = temp_test(
        r#"
        test('suma básica', () => {
            expect(1 + 1).toBe(2);
        });
        "#,
        "simple.test.js",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).await.unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, TestStatus::Passed, "test debe pasar");
    assert_eq!(results[0].name, "suma básica");
}

#[tokio::test]
async fn runner_reports_failing_test() {
    let (_dir, path) = temp_test(
        r#"
        test('falla intencional', () => {
            expect(1).toBe(2);
        });
        "#,
        "fail.test.js",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).await.unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, TestStatus::Failed);
    assert!(results[0].error.is_some(), "debe incluir mensaje de error");
}

#[tokio::test]
async fn runner_handles_multiple_tests_in_one_file() {
    let (_dir, path) = temp_test(
        r#"
        test('pasa', () => { expect(true).toBeTruthy(); });
        test('también pasa', () => { expect('hello').toContain('ell'); });
        test('falla', () => { expect(1).toBe(999); });
        "#,
        "multi.test.js",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).await.unwrap();

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

#[tokio::test]
async fn runner_supports_describe_blocks() {
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
    runner.run_file(&path).await.unwrap();

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

#[tokio::test]
async fn runner_runs_typescript_test_file() {
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
    runner.run_file(&path).await.unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, TestStatus::Passed);
}

// ── Matchers ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn runner_supports_all_core_matchers() {
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
    runner.run_file(&path).await.unwrap();

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

#[tokio::test]
async fn runner_discovers_test_files_in_directory() {
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
    runner.run_directory(dir.path()).await.unwrap();

    let results = runner.get_results();
    assert_eq!(
        results.len(),
        2,
        "debe encontrar exactamente 2 tests (a y b)"
    );
    assert!(results.iter().all(|r| r.status == TestStatus::Passed));
}

// ── Snapshots ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn runner_creates_snapshot_on_first_run() {
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
    runner.run_file(&path).await.unwrap();

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

#[tokio::test]
async fn runner_snapshot_fails_on_mismatch() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("mismatch.test.js");

    fs::write(
        &path,
        r#"test('snap', () => { expect('valor_a').toMatchSnapshot(); });"#,
    )
    .unwrap();

    // First run: creates snapshot with "valor_a"
    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).await.unwrap();
    assert_eq!(runner.get_results()[0].status, TestStatus::Passed);

    // Mutate test to produce different value
    fs::write(
        &path,
        r#"test('snap', () => { expect('valor_b').toMatchSnapshot(); });"#,
    )
    .unwrap();

    // Second run: should fail due to mismatch
    let mut runner2 = TestRunner::new(TestConfig::default());
    runner2.run_file(&path).await.unwrap();
    assert_eq!(
        runner2.get_results()[0].status,
        TestStatus::Failed,
        "snapshot mismatch debe fallar"
    );
}

#[tokio::test]
async fn runner_update_snapshots_flag_rewrites_stored_value() {
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
        .await
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
    runner.run_file(&path).await.unwrap();
    assert_eq!(
        runner.get_results()[0].status,
        TestStatus::Passed,
        "--update-snapshots debe actualizar y pasar"
    );

    // Next run without flag: should now pass with new value
    let mut runner2 = TestRunner::new(TestConfig::default());
    runner2.run_file(&path).await.unwrap();
    assert_eq!(runner2.get_results()[0].status, TestStatus::Passed);
}

// ── Syntax errors ────────────────────────────────────────────────────────────

#[tokio::test]
async fn runner_reports_syntax_error_as_failed_test() {
    let (_dir, path) = temp_test("const x = ;;;  // syntax error", "broken.test.js");

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).await.unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].status,
        TestStatus::Failed,
        "error de sintaxis debe reportarse como test fallido"
    );
}

// ── Watch mode (biblioteca) ───────────────────────────────────────────────────

#[tokio::test]
async fn watch_mode_reruns_produce_consistent_results() {
    // Simulates what --watch does: run tests, source changes, run again.
    // Validates that TestRunner is stateless enough to reuse across runs.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("watch.test.js");

    fs::write(&path, "test('v1', () => { expect(1 + 1).toBe(2); });").unwrap();

    // Primera pasada (archivo pasa)
    let mut runner1 = TestRunner::new(TestConfig::default());
    runner1.run_file(&path).await.unwrap();
    assert_eq!(runner1.get_results()[0].status, TestStatus::Passed);

    // Simula cambio de archivo: introduce un fallo
    fs::write(&path, "test('v2', () => { expect(1).toBe(99); });").unwrap();

    // Segunda pasada (archivo falla)
    let mut runner2 = TestRunner::new(TestConfig::default());
    runner2.run_file(&path).await.unwrap();
    assert_eq!(runner2.get_results()[0].status, TestStatus::Failed);

    // Simula corrección: vuelve a pasar
    fs::write(&path, "test('v3', () => { expect('ok').toBe('ok'); });").unwrap();

    let mut runner3 = TestRunner::new(TestConfig::default());
    runner3.run_file(&path).await.unwrap();
    assert_eq!(runner3.get_results()[0].status, TestStatus::Passed);
}

#[tokio::test]
async fn watch_mode_directory_reruns_pick_up_new_files() {
    // Verifica que run_directory puede ejecutarse varias veces sobre el mismo
    // directorio y detectar archivos añadidos entre pasadas.
    let dir = TempDir::new().unwrap();

    fs::write(
        dir.path().join("existing.test.js"),
        "test('existente', () => { expect(true).toBe(true); });",
    )
    .unwrap();

    let mut runner1 = TestRunner::new(TestConfig::default());
    runner1.run_directory(dir.path()).await.unwrap();
    assert_eq!(runner1.get_results().len(), 1);

    // Añade un segundo archivo (simula creación durante watch)
    fs::write(
        dir.path().join("nuevo.test.js"),
        "test('nuevo', () => { expect(42).toBeGreaterThan(0); });",
    )
    .unwrap();

    let mut runner2 = TestRunner::new(TestConfig::default());
    runner2.run_directory(dir.path()).await.unwrap();
    assert_eq!(runner2.get_results().len(), 2);
    assert!(runner2
        .get_results()
        .iter()
        .all(|r| r.status == TestStatus::Passed));
}

// ── Lifecycle hooks ───────────────────────────────────────────────────────────

#[tokio::test]
async fn hooks_beforeeach_and_aftereach_run_per_test() {
    let (_dir, path) = temp_test(
        r#"
        var log = [];
        beforeEach(() => { log.push('before'); });
        afterEach(()  => { log.push('after');  });

        test('primero', () => {
            expect(log).toEqual(['before']);
        });
        test('segundo', () => {
            // after del primero + before del segundo
            expect(log).toEqual(['before', 'after', 'before']);
        });
        "#,
        "lifecycle_each.test.js",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).await.unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 2);
    assert!(
        results.iter().all(|r| r.status == TestStatus::Passed),
        "ambos tests deben pasar: {:?}",
        results
            .iter()
            .filter(|r| r.status != TestStatus::Passed)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn hooks_beforeall_and_afterall_run_once() {
    let (_dir, path) = temp_test(
        r#"
        var calls = [];
        beforeAll(() => { calls.push('beforeAll'); });
        afterAll(()  => { calls.push('afterAll');  });

        test('a', () => {
            expect(calls).toEqual(['beforeAll']);
        });
        test('b', () => {
            expect(calls).toEqual(['beforeAll']);
        });
        "#,
        "lifecycle_all.test.js",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).await.unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 2);
    assert!(
        results.iter().all(|r| r.status == TestStatus::Passed),
        "beforeAll debe ejecutarse una sola vez antes de todos los tests: {:?}",
        results
            .iter()
            .filter(|r| r.status != TestStatus::Passed)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn hooks_scoped_to_describe_block() {
    let (_dir, path) = temp_test(
        r#"
        var outer = [];

        beforeEach(() => { outer.push('outer-before'); });

        describe('Grupo', () => {
            var inner = [];
            beforeEach(() => { inner.push('inner-before'); });
            afterEach(()  => { inner.push('inner-after');  });

            test('dentro del describe', () => {
                // outer beforeEach + inner beforeEach both ran
                expect(outer).toEqual(['outer-before']);
                expect(inner).toEqual(['inner-before']);
            });
        });

        test('fuera del describe', () => {
            // inner hooks must NOT run here
            expect(outer).toEqual(['outer-before', 'outer-before']);
        });
        "#,
        "lifecycle_scoped.test.js",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).await.unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 2);
    assert!(
        results.iter().all(|r| r.status == TestStatus::Passed),
        "hooks deben estar correctamente acotados al describe: {:?}",
        results
            .iter()
            .filter(|r| r.status != TestStatus::Passed)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn hooks_beforeall_failure_fails_all_tests_in_scope() {
    let (_dir, path) = temp_test(
        r#"
        beforeAll(() => { throw new Error('setup roto'); });

        test('test 1', () => { expect(1).toBe(1); });
        test('test 2', () => { expect(2).toBe(2); });
        "#,
        "lifecycle_beforeall_fail.test.js",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).await.unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 2);
    assert!(
        results.iter().all(|r| r.status == TestStatus::Failed),
        "si beforeAll falla, todos los tests del scope deben fallar"
    );
    assert!(
        results[0]
            .error
            .as_deref()
            .unwrap_or("")
            .contains("setup roto"),
        "el error debe mencionar la causa"
    );
}

#[tokio::test]
async fn hooks_beforeeach_failure_fails_only_that_test() {
    let (_dir, path) = temp_test(
        r#"
        var count = 0;
        beforeEach(() => {
            count++;
            if (count === 2) throw new Error('beforeEach en test 2');
        });

        test('test 1', () => { expect(1).toBe(1); });
        test('test 2 falla en setup', () => { expect(2).toBe(2); });
        test('test 3', () => { expect(3).toBe(3); });
        "#,
        "lifecycle_beforeeach_fail.test.js",
    );

    let mut runner = TestRunner::new(TestConfig::default());
    runner.run_file(&path).await.unwrap();

    let results = runner.get_results();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].status, TestStatus::Passed, "test 1 debe pasar");
    assert_eq!(
        results[1].status,
        TestStatus::Failed,
        "test 2 debe fallar por beforeEach"
    );
    assert_eq!(results[2].status, TestStatus::Passed, "test 3 debe pasar");
}
