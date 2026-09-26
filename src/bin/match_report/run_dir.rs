//! 集計対象の実行ディレクトリの保存形式と読み出し。

use serde::Deserialize;
use std::{fs::File, io, path::Path};

/// 集計対象の実行記録形式。
pub(super) const FORMAT_VERSION: u32 = 4;

/// 実行条件から集計に必要な部分。
#[derive(Deserialize)]
pub(super) struct Manifest {
    pub(super) candidate: EngineRecord,
    pub(super) baseline: EngineRecord,
    pub(super) mode: Mode,
    pub(super) concurrency: usize,
    pub(super) engine_threads: ThreadCounts,
    pub(super) cpu: CpuRecord,
}

/// 集計対象エンジンの探索制限。
#[derive(Deserialize)]
pub(super) struct EngineRecord {
    pub(super) limit: SearchLimit,
}

/// 校正で許可する固定ペア数Eloモード。
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Mode {
    Elo,
    Gsprt,
}

/// 保存された探索制限。
#[derive(Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum SearchLimit {
    Fixed {
        depth: Option<u32>,
        nodes: Option<u64>,
    },
    Time {
        base_ms: u64,
        increment_ms: u64,
        byoyomi_ms: u64,
    },
}

/// 両エンジンの探索ワーカー数。
#[derive(Deserialize)]
pub(super) struct ThreadCounts {
    pub(super) candidate: Option<u32>,
    pub(super) baseline: Option<u32>,
}

/// 測定機の資源量。
#[derive(Deserialize)]
pub(super) struct CpuRecord {
    pub(super) physical_cores: Option<usize>,
    pub(super) physical_memory_bytes: Option<u64>,
}

/// 再開を含む有効実行時間。
#[derive(Deserialize)]
pub(super) struct RunSummary {
    pub(super) active_wall_time_ns: u64,
    pub(super) interrupted: bool,
    pub(super) invocation_active: bool,
}

/// JSONファイルを型付きで読む。
pub(super) fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<T> {
    serde_json::from_reader(File::open(path)?).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid JSON in {}: {error}", path.display()),
        )
    })
}
