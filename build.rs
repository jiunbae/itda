// Windows: embed the manifest (visual-style controls + per-monitor-v2 DPI awareness), the
// app icon and the version resource. ScreenCaptureKit is optional on macOS 12.0–12.2; the
// .app bundle is made by scripts/macos-bundle.sh, and runtime capture entry points must gate
// its availability.
fn main() {
    println!("cargo:rerun-if-changed=itda.manifest");
    println!("cargo:rerun-if-changed=assets/itda.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // Override the binding crate's framework link: an unavailable optional capture
        // framework must not prevent the viewer and file browser from launching.
        println!("cargo:rustc-link-arg=-Wl,-weak_framework,ScreenCaptureKit");
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let m = dir.join("itda.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", m.display());
    // The version Windows shows (and remote setup reads to refuse a downgrade) is the
    // package's own, so the resource script is written for each build.
    let rc = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("itda.rc");
    std::fs::write(&rc, resource(&dir.join("assets").join("itda.ico"), &std::env::var("CARGO_PKG_VERSION").unwrap())).unwrap();
    embed_resource::compile(&rc, embed_resource::NONE).manifest_optional().unwrap();
}

/// The icon and a VERSIONINFO block for `version` ("1.2.3" or "1.2.3-beta.1").
fn resource(icon: &std::path::Path, version: &str) -> String {
    let number = |i: usize| -> u16 {
        version.split(['-', '+']).next().unwrap_or("").split('.').nth(i).and_then(|n| n.parse().ok()).unwrap_or(0)
    };
    let numeric = format!("{},{},{},0", number(0), number(1), number(2));
    let text = |s: &str| s.replace('\\', "\\\\").replace('"', "\"\"");
    format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEFLAGSMASK 0x3F
FILEFLAGS 0x0
FILEOS 0x40004
FILETYPE 0x1
FILESUBTYPE 0x0
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "FileDescription", "Itda"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "itda"
            VALUE "OriginalFilename", "itda.exe"
            VALUE "ProductName", "Itda"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#,
        icon = text(&icon.display().to_string()),
        version = text(version),
    )
}
