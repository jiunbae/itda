//! The optional Mac capture feature has a newer floor than the viewer and file browser.
#[cfg(target_os = "macos")]
pub fn macos_capture_requirement() -> &'static str {
    tr!("hp.captureRequirement")
}
#[cfg(target_os = "macos")]
pub fn capture_unavailable() -> &'static str {
    tr!("hp.captureUnavailable")
}

pub fn macos_supported(major: isize, minor: isize) -> bool {
    major >= 0 && minor >= 0 && (major, minor) >= (12, 3)
}

/// `sw_vers -productVersion` from the remote Mac, checked before any installation or write.
pub fn macos_version_supported(version: &str) -> bool {
    let mut parts = version.trim().split('.');
    match (parts.next().and_then(|p| p.parse().ok()), parts.next().and_then(|p| p.parse().ok())) {
        (Some(major), Some(minor)) => macos_supported(major, minor),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn capture_floor_is_separate_from_the_app_floor() {
        for version in ["11.7.10", "12.0", "12.2.1"] {
            assert!(!super::macos_version_supported(version), "{version}");
        }
        for version in ["12.3", "12.3.1", "12.7.6", "13.0", "26.0.1"] {
            assert!(super::macos_version_supported(version), "{version}");
        }
        for version in ["", "unknown", "12", "12.x", "-1.3", "13.-1"] {
            assert!(!super::macos_version_supported(version), "{version}");
        }
    }
}
