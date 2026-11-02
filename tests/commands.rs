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
    let output = Command::new(env!("CARGO_BIN_EXE_stile"))
        .args(["apply", "left", "--window", "0", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["ok"], false);
    assert!(response["error"].is_string());
}

#[cfg(target_os = "windows")]
#[test]
fn binary_positions_disposable_window_on_every_monitor() {
    stile_platform::test_support::binary_positions_disposable_window_on_every_monitor(env!(
        "CARGO_BIN_EXE_stile"
    ));
}
