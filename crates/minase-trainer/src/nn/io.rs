use std::{fs::File, io::Write, path::Path};

/// 同じディレクトリの一時ファイルを同期し、renameで公開してディレクトリを同期する。
/// 同じ出力先への同時書き込みは呼び出し側が防ぐ。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(temporary, path)?;
    if let Some(parent) = path.parent() {
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}
