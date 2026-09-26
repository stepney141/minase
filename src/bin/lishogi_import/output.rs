//! 変換結果に付随するファイルの書き出し。

use minase::datagen::data_error;
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub(super) fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut path = path.as_os_str().to_owned();
    path.push(suffix);
    path.into()
}

pub(super) fn create(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

pub(super) fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let mut file = create(path)?;
    let result = serde_json::to_writer_pretty(&mut file, value)
        .map_err(data_error)
        .and_then(|()| file.flush());
    if result.is_err() {
        fs::remove_file(path)?;
    }
    result
}
