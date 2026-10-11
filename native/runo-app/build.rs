//! Embeds the app icon into the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/runo.ico");
    println!("cargo:rerun-if-changed=runo.rc");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("runo.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("could not embed the Windows icon");
    }
}
