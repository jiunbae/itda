//! Files handed to Finder are real file URLs or lazy native file promises, never text.
use super::*;

#[derive(Clone)]
pub struct File {
    pub id: String,
    pub remote: bool,
    pub source: String,
    pub name: String,
    pub dir: bool,
}
pub fn selection(v: &Value) -> io::Result<Vec<File>> {
    let side = get(v, "side");
    if side != "local" && side != "remote" {
        return Err(error("파일 위치를 확인하세요"));
    }
    let items = v["items"]
        .as_array()
        .ok_or_else(|| error("파일을 선택하세요"))?;
    if items.is_empty() || items.len() > 10_000 {
        return Err(error("복사할 파일 수를 확인하세요"));
    }
    let mut seen = std::collections::HashSet::new();
    items
        .iter()
        .map(|item| {
            let n = get(item, "name");
            name(&n)?;
            if !seen.insert(n.clone()) {
                return Err(error("중복된 파일 이름입니다"));
            }
            Ok(File {
                id: get(v, "id"),
                remote: side == "remote",
                source: get(v, "path"),
                name: n,
                dir: item["dir"] == true,
            })
        })
        .collect()
}
impl File {
    pub fn path(&self) -> io::Result<PathBuf> {
        name(&self.name)?;
        Ok(if self.remote {
            PathBuf::from(join(&self.source, &self.name)?)
        } else {
            Path::new(&canonical(&self.source)?).join(&self.name)
        })
    }
}
/// Copy requested remote files to a private, persistent staging directory for the
/// system clipboard. Retain it after quitting so Finder can paste after Itda closes.
pub fn prepare(files: &[File], emit: Emit) -> io::Result<Vec<PathBuf>> {
    if files.iter().all(|f| !f.remote) {
        return files
            .iter()
            .map(|f| {
                let path = f.path()?;
                std::fs::symlink_metadata(&path)?;
                Ok(path)
            })
            .collect();
    }
    let root = crate::store::dir()
        .join("file-exports")
        .join(crate::store::new_id());
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&root)?;
    let paths: Vec<_> = files.iter().map(|f| root.join(&f.name)).collect();
    let result = super::transfer::export_batch(files, &paths, emit).map(|()| paths);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&root);
    }
    result
}
pub fn write(file: &File, target: &Path, emit: Emit) -> io::Result<()> {
    super::transfer::export_batch(std::slice::from_ref(file), &[target.to_path_buf()], emit)
}
