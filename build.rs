// Embed the manifest (visual-style controls + per-monitor-v2 DPI awareness) and the app icon.
fn main() {
    let m = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("itda.manifest");
    println!("cargo:rerun-if-changed=itda.manifest");
    println!("cargo:rerun-if-changed=assets/itda.ico");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", m.display());
    embed_resource::compile("assets/itda.rc", embed_resource::NONE).manifest_optional().unwrap();
}