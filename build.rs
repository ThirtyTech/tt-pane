#![forbid(unsafe_code)]

fn main() {
    println!("cargo:rerun-if-changed=assets/stile.ico");
    println!("cargo:rerun-if-changed=assets/stile.rc");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/stile.rc", embed_resource::NONE)
            .manifest_required()
            .expect("Compile sTile icon resource");
    }
}
