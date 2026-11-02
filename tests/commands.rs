#![forbid(unsafe_code)]

use std::process::Command;

#[test]
fn discovery_matches_plugin_manifest_without_loading_config() {
    let output = Command::new(env!("CARGO_BIN_EXE_stile"))
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
    let output = Command::new(env!("CARGO_BIN_EXE_stile"))
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
    let output = Command::new(env!("CARGO_BIN_EXE_stile"))
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

#[cfg(target_os = "windows")]
#[test]
fn binary_positions_disposable_window_on_every_monitor() {
    for growth in [
        stile::config::Growth::default(),
        stile::config::Growth {
            percent: 25,
            steps: 1,
        },
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = stile::config::Config {
            growth,
            width_resize: stile::config::WidthResize {
                percent: if growth.steps == 1 { 40 } else { 25 },
            },
            ..Default::default()
        };
        config.save(&path).unwrap();
        // Corrupt or obsolete cache content must reset instead of preventing placement.
        std::fs::write(dir.path().join("config.toml.cycle.json"), "invalid json").unwrap();
        stile_platform::test_support::binary_positions_disposable_window_on_every_monitor(
            env!("CARGO_BIN_EXE_stile"),
            path.to_str().unwrap(),
            growth.percent,
            growth.steps,
            config.width_resize.percent,
        );
    }
}
