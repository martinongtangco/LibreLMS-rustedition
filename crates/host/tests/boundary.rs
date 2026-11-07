//! US2 — the boundary check must FAIL on an intentional module→module
//! dependency (SC-003: 1 of 1 detection) and pass on the clean tree.
//! The test injects a path dependency into crates/catalog/Cargo.toml, runs
//! the check, and restores the file (self-cleaning — Principle XVII).

use std::process::Command;

fn repo_root() -> std::path::PathBuf {
    // tests/ → crates/host/ → crates/ → repo root
    let here = std::env::current_dir().expect("cwd");
    // cargo sets CWD to the crate root for integration tests
    here.join("../..")
}

#[test]
fn boundary_check_detects_module_to_module_dependency() {
    let root = repo_root();
    let catalog_toml = root.join("crates/catalog/Cargo.toml");
    let original = std::fs::read_to_string(&catalog_toml).expect("read catalog toml");
    let check = root.join(".github/scripts/check-boundaries.sh");

    // Inject an intentional violation: catalog → enrollment.
    let violated = format!("{original}\nenrollment = {{ path = \"../enrollment\" }}\n");
    std::fs::write(&catalog_toml, &violated).expect("write violation");

    // Run the boundary check against the violating tree.
    let status = Command::new("bash")
        .arg(&check)
        .output()
        .expect("run check");
    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
    // ALWAYS restore, even if assertions below would panic.
    std::fs::write(&catalog_toml, &original).expect("restore catalog toml");

    assert!(
        !status.status.success(),
        "boundary check must FAIL on catalog→enrollment; output: {out}"
    );
    assert!(
        out.contains("catalog") && out.contains("enrollment"),
        "boundary check must name the offending pair; output: {out}"
    );

    // Clean tree passes.
    let clean = Command::new("bash")
        .arg(&check)
        .output()
        .expect("run clean check");
    let clean_out = format!(
        "{}{}",
        String::from_utf8_lossy(&clean.stdout),
        String::from_utf8_lossy(&clean.stderr)
    );
    assert!(
        clean.status.success(),
        "boundary check must pass on the clean tree; output: {clean_out}"
    );
}
