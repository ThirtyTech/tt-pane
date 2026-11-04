#![forbid(unsafe_code)]

use std::process::Command;

#[test]
fn binary_and_plugin_versions_match_package() {
    let output = Command::new(env!("CARGO_BIN_EXE_tt-pane"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        concat!("tt-pane ", env!("CARGO_PKG_VERSION"))
    );
    let manifest: toml::Value = toml::from_str(include_str!("../herdr-plugin.toml")).unwrap();
    assert_eq!(
        manifest["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn discovery_matches_plugin_manifest_without_loading_config() {
    let output = Command::new(env!("CARGO_BIN_EXE_tt-pane"))
        .args([
            "--config",
            "nonexistent-do-not-create/config.toml",
            "actions",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["schema_version"], 1);
    let manifest: toml::Value = toml::from_str(include_str!("../herdr-plugin.toml")).unwrap();
    let actions = manifest["actions"].as_array().unwrap();
    assert_eq!(response["actions"].as_array().unwrap().len(), actions.len());
    for action in actions {
        let id = action["id"].as_str().unwrap();
        assert!(
            response["actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["id"] == id)
        );
        assert_eq!(action["command"][2].as_str(), Some(id));
    }
}

#[test]
fn invalid_target_returns_json_and_failure_exit_status() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("config.toml.cycle.json");
    std::fs::write(&state, "existing cycle").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tt-pane"))
        .arg("--config")
        .arg(dir.path().join("config.toml"))
        .args(["apply", "left", "--window", "0", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["ok"], false);
    assert!(response["error"].is_string());
    assert_eq!(std::fs::read_to_string(state).unwrap(), "existing cycle");
}

#[test]
fn invalid_growth_config_returns_json_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "version = 1\nbindings = []\n[growth]\npercent = 0").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tt-pane"))
        .arg("--config")
        .arg(path)
        .args(["apply", "left", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["ok"], false);
    assert!(
        response["error"]
            .as_str()
            .unwrap()
            .contains("Growth percentage")
    );
}

#[test]
fn fill_gap_without_saved_state_reports_json_error() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tt-pane"))
        .arg("--config")
        .arg(dir.path().join("config.toml"))
        .args(["apply", "fill-gap", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], false);
    assert!(result["error"].as_str().unwrap().contains("No saved gap"));
}

#[cfg(target_os = "windows")]
#[test]
fn binary_positions_disposable_window_on_every_monitor() {
    for growth in [
        tt_pane::config::Growth::default(),
        tt_pane::config::Growth {
            percent: 25,
            steps: 1,
        },
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = tt_pane::config::Config {
            growth,
            gaps: if growth.steps == 1 {
                tt_pane::geometry::SnapGaps { inner: 0, outer: 0 }
            } else {
                Default::default()
            },
            width_resize: tt_pane::config::WidthResize {
                percent: if growth.steps == 1 { 50 } else { 25 },
            },
            ..Default::default()
        };
        config.save(&path).unwrap();
        // Corrupt or obsolete cache content must reset instead of preventing placement.
        std::fs::write(dir.path().join("config.toml.cycle.json"), "invalid json").unwrap();
        tt_pane_platform::test_support::binary_positions_disposable_window_on_every_monitor(
            env!("CARGO_BIN_EXE_tt-pane"),
            path.to_str().unwrap(),
            growth.percent,
            growth.steps,
            config.width_resize.percent,
            config.gaps,
        );
    }
}

#[cfg(target_os = "windows")]
#[test]
fn focus_errors_leave_saved_placement_state_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let state = dir.path().join("config.toml.cycle.json");
    std::fs::write(&state, "existing gap and growth state").unwrap();
    for direction in ["focus-left", "focus-down", "focus-up", "focus-right"] {
        let output = Command::new(env!("CARGO_BIN_EXE_tt-pane"))
            .arg("--config")
            .arg(&path)
            .args(["apply", direction, "--window", "0", "--json"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["ok"], false);
        assert!(
            response["error"]
                .as_str()
                .unwrap()
                .contains("Invalid or missing window")
        );
        assert_eq!(
            std::fs::read_to_string(&state).unwrap(),
            "existing gap and growth state"
        );
    }
    assert!(!dir.path().join("config.toml.cycle.lock").exists());
}
