use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn export_json_dot_returns_valid_json() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    fs::write(
        dir.path().join("main.py"),
        "from util import helper\n\ndef main():\n    return helper()\n",
    )?;
    fs::write(dir.path().join("util.py"), "def helper():\n    return 42\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_cm"))
        .current_dir(dir.path())
        .args(["export", "--json", ".", "--no-cache"])
        .output()?;

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["meta"]["root"], "");
    assert!(json["nodes"]["files"]
        .as_array()
        .is_some_and(|files| !files.is_empty()));
    assert!(json["edges"]["calls"].as_array().is_some());

    Ok(())
}
