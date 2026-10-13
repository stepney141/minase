//! 入力は正規化したパスと、形式に応じたヘッダまたはバイト長で識別する。
//! MNSDは`byte_length`と`header`（規則、生成コミット、重みの検査和、教師の種類と規模、シード、対局数、項目数）を持つ。
//! MNPTとMNUEはヘッダに記録済みの`checksum`、NPZ（学習器のチェックポイントを含む）は`byte_length`を持つ。

use crate::nn::hex;
use minase::training::records_v2::Reader;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{self, Read, Seek},
    path::{Path, PathBuf},
};

/// ファイル全体の検査和を計算せずに記録する入力の識別情報。
#[derive(Debug, Serialize)]
pub struct FileIdentity {
    path: PathBuf,
    #[serde(flatten)]
    metadata: Value,
}

/// ヘッダだけを読み、入力を識別する。
pub fn file_identity(path: &Path) -> io::Result<FileIdentity> {
    let path = path.canonicalize()?;
    let mut file = File::open(&path)?;
    let length = file.metadata()?.len();
    let mut magic = [0; 4];
    file.read_exact(&mut magic)?;
    let metadata = match &magic {
        b"MNSD" => {
            file.rewind()?;
            let reader = Reader::new(file)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let header = reader.header();
            json!({"byte_length": length, "header": {
                "rule_set": header.rule_set(), "generation_commit": header.generation_commit(),
                "network_checksum": hex(header.network_checksum()), "teacher_kind": header.teacher_kind() as u8, "teacher_scale": header.teacher_scale(),
                "seed": header.seed(), "game_count": header.game_count(), "entry_count": header.entry_count()
            }})
        }
        b"MNPT" | b"MNUE" => {
            let end = if &magic == b"MNPT" { 80 } else { 176 };
            let mut bytes = vec![0; end];
            file.read_exact(&mut bytes[4..])?;
            json!({"checksum": hex(&bytes[end-32..])})
        }
        b"PK\x03\x04" => json!({"byte_length": length}),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported input format for file identity",
            ));
        }
    };
    Ok(FileIdentity { path, metadata })
}

/// 新規作成するMNPTを、正規化した親ディレクトリとヘッダ内の検査和で識別する。
pub(crate) fn mnpt_output_identity(path: &Path, bytes: &[u8]) -> io::Result<FileIdentity> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        Some(_) => Path::new("."),
        None => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "output has no parent",
            ));
        }
    };
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "output has no file name"))?;
    let checksum = bytes
        .get(48..80)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "short MNPT header"))?;
    Ok(FileIdentity {
        path: parent.canonicalize()?.join(name),
        metadata: json!({"checksum": hex(checksum)}),
    })
}
