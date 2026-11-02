#![forbid(unsafe_code)]
#![cfg(all(target_os = "windows", feature = "gui"))]

#[test]
#[ignore = "requires an interactive Windows desktop"]
fn tray_starts_without_window_and_survives_settings_close() {
    let directory = tempfile::tempdir().unwrap();
    stile_platform::test_support::tray_starts_without_window_and_survives_settings_close(
        env!("CARGO_BIN_EXE_stile"),
        &directory.path().join("config.toml"),
    );
}

#[test]
#[ignore = "requires an interactive Windows desktop"]
fn terminal_ctrl_c_terminates_tray_app() {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    const CHILD: &str = "STILE_ISOLATED_CONSOLE_TEST";
    if std::env::var_os(CHILD).is_some() {
        stile_platform::test_support::terminal_ctrl_c_terminates_tray_app(env!(
            "CARGO_BIN_EXE_stile"
        ));
        return;
    }
    // CREATE_NO_WINDOW gives this console-subsystem test helper its own hidden
    // console. No control event can reach the user's terminal or other apps.
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "terminal_ctrl_c_terminates_tray_app",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
