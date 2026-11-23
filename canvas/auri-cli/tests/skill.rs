//! `auri skill` prints the path of the skill file bundled with the app.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn skill_prints_a_file_that_teaches_list_ask_notes_and_ask_back() {
    let skill = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../skills/auri/SKILL.md");
    assert!(skill.is_file(), "bundled skill file is missing: {}", skill.display());

    let output = Command::new(env!("CARGO_BIN_EXE_auri"))
        .arg("skill")
        .env("AURI_SKILL", &skill)
        .env_remove("AURI_SOCKET")
        .env_remove("AURI_TOKEN")
        .output()
        .expect("run auri skill");

    assert!(
        output.status.success(),
        "auri skill failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        skill.to_string_lossy().as_ref()
    );

    let body = std::fs::read_to_string(&skill).expect("read the printed skill file");
    for needle in ["auri list", "auri ask", "auri note read", "auri note write", "ask-back"] {
        assert!(body.contains(needle), "skill is missing {needle:?}");
    }
    assert!(
        body.contains("user") && body.contains("connect"),
        "skill must say that connections are user-controlled"
    );
}
