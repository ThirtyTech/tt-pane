#![forbid(unsafe_code)]

fn main() {
    println!("cargo:rerun-if-changed=assets/tt-pane.ico");
    println!("cargo:rerun-if-changed=assets/tt-pane.rc");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/tt-pane.rc", embed_resource::NONE)
            .manifest_required()
            .expect("Compile tt-pane icon resource");
    }
}
