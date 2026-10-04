//! 学習局面の来歴を指定する引数と来歴JSONの書き出し。

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Write};
use std::path::PathBuf;

use crate::training::provenance::{
    GameOrigin, Provenance, ResultOrigin, SearchCondition, StartOrigin, Teacher,
};
use crate::training::records::Reader;
use crate::training::rescore;

use super::data_error;

/// 既存MNSDの来歴作成の引数。
#[derive(clap::Args)]
pub struct ProvenanceArguments {
    /// 元のMNSD。
    #[arg(long)]
    pub input: PathBuf,
    /// 新規作成する来歴JSON。
    #[arg(long)]
    pub output: PathBuf,
    /// 対局結果の由来。本操作ではselfplayだけを扱う。
    #[arg(long, value_enum)]
    pub result_origin: ResultOrigin,
    /// 開始局面の由来。本操作ではrandomだけを扱う。
    #[arg(long, value_enum)]
    pub start_origin: StartOrigin,
    /// 教師値に占める探索値の割合。
    #[arg(long)]
    pub lambda: f64,
    /// 教師の探索条件。
    #[arg(long, value_enum, default_value = "in-game")]
    pub search_condition: SearchCondition,
}

/// 対局番号と実戦棋譜の対応を含めて来歴を書く。
pub fn write_mapped_provenance(
    arguments: &ProvenanceArguments,
    games: Option<Vec<GameOrigin>>,
) -> io::Result<()> {
    let mut input = File::open(&arguments.input)?;
    let checksum = rescore::sha256(&mut input)?;
    let reader = Reader::new(BufReader::new(input)).map_err(data_error)?;
    let header = reader.header();
    let provenance = Provenance {
        format: "minase-provenance".to_owned(),
        version: 1,
        mnsd_sha256: hex(&checksum),
        teacher: Teacher {
            generation_commit: header.generation_commit().to_owned(),
            network_checksum: hex(header.network_checksum()),
            nodes: header.teacher_nodes(),
            rule_set: header.rule_set().to_owned(),
            search_condition: arguments.search_condition,
        },
        result_origin: arguments.result_origin,
        start_origin: arguments.start_origin,
        lambda: arguments.lambda,
        games,
    };
    provenance.validate().map_err(data_error)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&arguments.output)?;
    provenance.write(&mut output).map_err(data_error)?;
    output.flush()
}

/// バイト列を小文字16進文字列へ変換する。
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        text.push(DIGITS[usize::from(byte >> 4)] as char);
        text.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    text
}
