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
