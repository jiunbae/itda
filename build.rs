// Windows: embed the manifest (visual-style controls + per-monitor-v2 DPI awareness) and the
// app icon. macOS needs nothing here; the .app bundle is made by scripts/macos-bundle.sh.
fn main() {
    println!("cargo:rerun-if-changed=itda.manifest");
    println!("cargo:rerun-if-changed=assets/itda.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let m = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("itda.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", m.display());
    embed_resource::compile("assets/itda.rc", embed_resource::NONE).manifest_optional().unwrap();
}
