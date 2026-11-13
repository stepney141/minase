//! データ生成器の出力先で共有する排他と原子的な保存。
//!
//! [`DirectoryLock`]は、呼び出し側が指定したロックファイルを開き、処理中だけOSの
//! 排他ロックを保持する。
//! [`atomic_json`]はJSONを`.tmp`で終わる一時ファイルへ書き、内容を同期した後に
//! 最終名へ改名し、出力先ディレクトリを同期する。
//! [`commit_files`]は同じ確定手順を、呼び出し側が書き終えた任意の一時ファイルへ
//! 適用する。

use fs2::FileExt;
use serde::Serialize;
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

/// 排他の取得、JSONの符号化、またはファイル操作の失敗。
#[derive(Debug)]
pub enum Error {
    /// 出力先の排他ロックを取得できない。
    Locked {
        /// ロック対象の出力先。
        path: PathBuf,
        /// OSが返した失敗原因。
        source: io::Error,
    },
    /// JSONを符号化できない。
    Json(serde_json::Error),
    /// ファイル操作に失敗した。
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked { path, source } => {
                write!(
                    f,
                    "cannot lock output directory {}: {source}",
                    path.display()
                )
            }
            Self::Json(error) => write!(f, "cannot encode JSON: {error}"),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Locked { source, .. } | Self::Io(source) => Some(source),
            Self::Json(source) => Some(source),
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// ファイル記述子ごとのOS排他ロックを生存中保持する。
pub struct DirectoryLock(File);

impl DirectoryLock {
    /// `directory`内の`lock_name`を開き、排他ロックを取得する。
    pub fn acquire(directory: &Path, lock_name: &str) -> Result<Self, Error> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(lock_name))?;
        file.try_lock_exclusive().map_err(|source| Error::Locked {
            path: directory.to_owned(),
            source,
        })?;
        Ok(Self(file))
    }
}

impl Drop for DirectoryLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

/// 一時ファイルを同期して指定順に改名し、最後に出力先ディレクトリを同期する。
///
/// 呼び出し側は、渡す前に各一時ファイルへの書き込みを完了して閉じる。
pub fn commit_files(directory: &Path, files: &[(&Path, &Path)]) -> Result<(), Error> {
    for (temporary, _) in files {
        File::open(temporary)?.sync_all()?;
    }
    for (temporary, final_path) in files {
        fs::rename(temporary, final_path)?;
    }
    File::open(directory)?.sync_all()?;
    Ok(())
}

/// JSONを一時ファイルへ整形して書き、同期と改名によって最終名へ確定する。
pub fn atomic_json(directory: &Path, name: &str, value: &impl Serialize) -> Result<(), Error> {
    let temporary = directory.join(format!("{name}.tmp"));
    let final_path = directory.join(name);
    let mut writer = BufWriter::new(File::create(&temporary)?);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    drop(writer);
    commit_files(directory, &[(&temporary, &final_path)])
}
