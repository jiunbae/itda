// Windows: embed the manifest (visual-style controls + per-monitor-v2 DPI awareness) and the
// app icon. ScreenCaptureKit is optional on macOS 12.0–12.2; the .app bundle is made by
// scripts/macos-bundle.sh, and runtime capture entry points must gate its availability.
fn main() {
    println!("cargo:rerun-if-changed=itda.manifest");
    println!("cargo:rerun-if-changed=assets/itda.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // Override the binding crate's framework link: an unavailable optional capture
        // framework must not prevent the viewer and file browser from launching.
        println!("cargo:rustc-link-arg=-Wl,-weak_framework,ScreenCaptureKit");
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let m = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("itda.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", m.display());
    embed_resource::compile("assets/itda.rc", embed_resource::NONE).manifest_optional().unwrap();
}
