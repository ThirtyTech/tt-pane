#![forbid(unsafe_code)]

fn main() {
    println!("cargo:rerun-if-changed=assets/tt-pane.ico");
    println!("cargo:rerun-if-changed=assets/tt-pane.rc");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version = std::env::var("CARGO_PKG_VERSION").expect("Cargo package version");
        let numeric = format!(
            "{},{},{},0",
            std::env::var("CARGO_PKG_VERSION_MAJOR").unwrap(),
            std::env::var("CARGO_PKG_VERSION_MINOR").unwrap(),
            std::env::var("CARGO_PKG_VERSION_PATCH").unwrap(),
        );
        embed_resource::compile(
            "assets/tt-pane.rc",
            [
                format!("APP_VERSION=\"{version}\""),
                format!("APP_VERSION_NUMERIC={numeric}"),
            ],
        )
        .manifest_required()
        .expect("Compile tt-pane icon and version resources");
    }
}
