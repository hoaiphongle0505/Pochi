// Embeds the app icon, version info and DPI-aware manifest into pochi.exe.
fn main() {
    println!("cargo:rerun-if-changed=res/app.rc");
    println!("cargo:rerun-if-changed=res/icon.ico");
    println!("cargo:rerun-if-changed=res/app.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("res/app.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
