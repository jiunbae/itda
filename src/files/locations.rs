//! Finder connection shortcuts. These are links to saved devices, not a mounted filesystem.
use super::{error, name};
use crate::store::Host;
use std::{collections::HashSet, fs, io, path::Path};

pub const NETWORK: &str = "__network";

pub fn folder() -> std::path::PathBuf {
    crate::store::dir().join("Locations").join("Itda")
}

pub fn device_url(id: &str) -> String {
    format!(
        "itda://files/{}",
        id.bytes().map(|b| format!("%{b:02X}")).collect::<String>()
    )
}

#[cfg(any(target_os = "macos", test))]
pub fn device_from_url(url: &str) -> Option<String> {
    let mut encoded = url.strip_prefix("itda://files/")?.as_bytes();
    let mut id = Vec::new();
    if encoded.is_empty() || encoded.len() > 3072 {
        return None;
    }
    while let Some((&b, rest)) = encoded.split_first() {
        if b == b'%' {
            let hex = std::str::from_utf8(encoded.get(1..3)?).ok()?;
            id.push(u8::from_str_radix(hex, 16).ok()?);
            encoded = &encoded[3..];
        } else {
            if !b.is_ascii_alphanumeric() && !matches!(b, b'-' | b'_') {
                return None;
            }
            id.push(b);
            encoded = rest;
        }
    }
    let id = String::from_utf8(id).ok()?;
    if id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control) {
        return None;
    }
    Some(id)
}

fn regular(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() && !m.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(error("연결 바로가기 경로가 일반 파일이 아닙니다")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let temp = path.with_file_name(format!(".itda-{}.tmp", crate::store::new_id()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        super::rename_local(&temp.to_string_lossy(), &path.to_string_lossy(), true)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

/// Only generated links named in our manifest are replaced or removed. Other files stay intact.
pub fn write(hosts: &[Host], root: &Path) -> io::Result<()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap();
    match fs::symlink_metadata(root) {
        Ok(m) if !m.is_dir() || m.file_type().is_symlink() => {
            return Err(error("연결 폴더 경로를 확인하세요"))
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir_all(root)?,
        Err(e) => return Err(e),
    }
    let manifest = root.join(".itda-links.json");
    let previous: HashSet<String> = if regular(&manifest)? {
        if fs::metadata(&manifest)?.len() > 1 << 20 {
            return Err(error("연결 바로가기 목록이 너무 큽니다"));
        }
        serde_json::from_slice(&fs::read(&manifest)?)
            .map_err(|_| error("연결 바로가기 목록을 읽을 수 없습니다"))?
    } else {
        HashSet::new()
    };
    for old in &previous {
        name(old)?;
        if !old.ends_with(".inetloc") {
            return Err(error("연결 바로가기 목록을 확인하세요"));
        }
    }
    let mut next = HashSet::new();
    let mut planned = Vec::new();
    for (index, h) in hosts.iter().enumerate() {
        let readable: String = h
            .label()
            .chars()
            .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':'))
            .take(70)
            .collect();
        let leaf = format!(
            "{:02} {}.inetloc",
            index + 1,
            readable.trim().trim_matches('.')
        );
        let path = root.join(&leaf);
        if regular(&path)? && !previous.contains(&leaf) {
            return Err(error(
                "같은 이름의 사용자 파일이 있어 연결 바로가기를 만들지 않았습니다",
            ));
        }
        let url = device_url(&h.id);
        let xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>URL</key><string>{url}</string></dict></plist>\n");
        planned.push((path, xml));
        next.insert(leaf);
    }
    // Keep stale links owned until cleanup succeeds, so interrupted refreshes can recover.
    let owned: HashSet<_> = previous.union(&next).cloned().collect();
    atomic_write(&manifest, &serde_json::to_vec(&owned)?)?;
    for (path, xml) in planned {
        atomic_write(&path, xml.as_bytes())?;
    }
    for old in previous.difference(&next) {
        let p = root.join(old);
        if regular(&p)? {
            fs::remove_file(p)?;
        }
    }
    atomic_write(&manifest, &serde_json::to_vec(&next)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn links_roundtrip_and_reject_external_commands() {
        for id in ["device-123", "한글 /?&<기기>", NETWORK] {
            assert_eq!(device_from_url(&device_url(id)).as_deref(), Some(id));
        }
        for url in [
            "itda://files/",
            "itda://files/id?host=evil",
            "itda://files/%ZZ",
            "itda://files/%00",
            "https://files/id",
            "itda://files/../id",
        ] {
            assert!(device_from_url(url).is_none());
        }
    }
    #[test]
    fn refresh_preserves_user_files_and_exports_no_credentials() {
        let root = std::env::temp_dir().join(format!("itda-links-{}", crate::store::new_id()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("my notes.txt"), "keep").unwrap();
        let mut h = Host {
            id: "device-1".into(),
            name: "Mac / Studio".into(),
            host: "secret-host".into(),
            account: "secret-account".into(),
            ..Default::default()
        };
        write(&[h.clone()], &root).unwrap();
        let entry = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .find(|e| e.path().extension().is_some_and(|e| e == "inetloc"))
            .unwrap();
        let text = fs::read_to_string(entry.path()).unwrap();
        assert!(text.contains(&device_url(&h.id)));
        assert!(!text.contains("secret-host") && !text.contains("secret-account"));
        h.name = "renamed".into();
        write(&[h], &root).unwrap();
        assert!(!entry.path().exists());
        write(&[], &root).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("my notes.txt")).unwrap(),
            "keep"
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                root.join("my notes.txt"),
                root.join(".itda-links.json.bad"),
            )
            .unwrap();
            fs::remove_file(root.join(".itda-links.json")).unwrap();
            fs::rename(
                root.join(".itda-links.json.bad"),
                root.join(".itda-links.json"),
            )
            .unwrap();
            assert!(write(&[], &root).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
