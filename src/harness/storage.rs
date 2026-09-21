//! runner間で共有する排他ロックと原子的な新規保存。

use fs2::FileExt;
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::Path,
};

/// 生存中だけ保持する実行ディレクトリの排他ロック。
#[derive(Debug)]
pub struct RunLock(File);

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

/// 実行ディレクトリをプロセス間で排他的にロックする。
pub fn lock_run_directory(path: &Path) -> io::Result<RunLock> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.join(".match_runner.lock"))?;
    file.try_lock_exclusive().map_err(|error| {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            format!("run directory is already in use: {error}"),
        )
    })?;
    Ok(RunLock(file))
}

/// 値を同一ディレクトリの一時ファイルへ同期後、renameで確定する。
pub fn atomic_write_json<T: Serialize>(
    directory: &Path,
    temporary_path: &Path,
    final_path: &Path,
    value: &T,
) -> io::Result<()> {
    let write_result = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary_path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, value)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        if final_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("record already exists: {}", final_path.display()),
            ));
        }
        fs::rename(temporary_path, final_path)?;
        File::open(directory)?.sync_all()
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(temporary_path);
    }
    write_result
}
