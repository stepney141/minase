//! 対局の実体、各応答、および終局結果の保存形式。

use serde::{Deserialize, Deserializer, Serialize};
use std::path::PathBuf;

/// エンジンとの通信プロトコル。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredProtocol {
    /// USIによる通信。
    Usi,
    /// CECP（XBoard）による通信。
    Cecp,
}

/// エンジン実体の識別方法。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EngineIdentity {
    /// 同梱のランダムエンジン。
    Random {
        /// 実行ファイル全体のSHA-256。
        sha256: String,
    },
    /// Gitコミットからビルドしたエンジン。
    Commit {
        /// 完全コミットハッシュ。
        hash: String,
        /// 実際に起動するキャッシュ済みバイナリのSHA-256。
        sha256: String,
    },
    /// 任意の起動コマンド。
    Command {
        /// 実行ファイルの指定。
        program: PathBuf,
        /// 起動引数。
        args: Vec<String>,
        /// 通信プロトコル。
        protocol: StoredProtocol,
        /// 相対パスの解釈に使う作業ディレクトリ。
        working_directory: PathBuf,
    },
}

/// 保存用の手番。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredColor {
    /// 先手。
    Black,
    /// 後手。
    White,
}

/// エンジン異常の分類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// 不正着手。
    IllegalMove,
    /// プロセス終了またはパイプ切断。
    Crash,
    /// 応答タイムアウト。
    Timeout,
    /// 持ち時間の超過。
    TimeForfeit,
    /// 相手の合法手の拒否。
    RejectedMove,
}

/// エンジンが返した評価値。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScoreRecord {
    /// 通常評価値。
    Cp {
        /// エンジン固有尺度の評価値。
        value: i32,
    },
    /// 手番側が詰ませる評価値。
    MateIn {
        /// 詰みまでの手数。手数不明の`+`では`None`。
        moves: Option<u32>,
    },
    /// 手番側が詰む評価値。
    MatedIn {
        /// 詰みまでの手数。手数不明の`-`では`None`。
        moves: Option<u32>,
    },
}

/// 評価値が表す境界。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreBound {
    /// 上下界ではない評価値。
    Exact,
    /// 下界。
    Lower,
    /// 上界。
    Upper,
}

/// 1回の思考で最後に得た評価情報。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationRecord {
    /// 評価値の視点。
    pub perspective: StoredColor,
    /// 評価値と同じ`info`行の探索深さ。
    pub depth: Option<u32>,
    /// 評価値。
    pub score: ScoreRecord,
    /// 上下界の種別。
    pub bound: ScoreBound,
}

/// 探索を停止した条件。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReasonRecord {
    /// 指定深さを完了した。
    Depth,
    /// 指定ノード数へ達した。
    Nodes,
    /// 完了イテレーションの境界でsoft limitへ達した。
    Soft,
    /// 探索中にhard limitへ達した。
    Hard,
    /// 呼び出し側から停止を要求された。
    External,
}

/// `null`を認めつつ、JSON欄自体の省略は拒否する。
fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

/// 1回の思考に対するエンジン応答。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TurnResponse {
    /// エンジンが選んだ着手。
    Move {
        /// 審判層が正規化したUSI表記。
        usi: String,
    },
    /// 投了。
    Resigned,
    /// エンジン異常。
    Failure {
        /// 異常分類。
        reason: FailureKind,
    },
}

/// 1回の思考と応答の記録。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnRecord {
    /// 思考した側。
    pub side: StoredColor,
    /// 実測思考時間(ns)。
    pub think_time_ns: u64,
    /// 最後の評価情報。評価値がない場合は`None`。
    pub evaluation: Option<EvaluationRecord>,
    /// エンジンが報告した停止理由。報告がない場合は`None`。
    #[serde(deserialize_with = "deserialize_required_option")]
    pub stop_reason: Option<StopReasonRecord>,
    /// 最後に完了した反復の経過時間(ms)。報告がない場合は`None`。
    #[serde(deserialize_with = "deserialize_required_option")]
    pub completed_time_ms: Option<u64>,
    /// 応答に付いた予想手。欄は必須で、予想手がなければnull。
    #[serde(deserialize_with = "deserialize_required_option")]
    pub ponder: Option<String>,
    /// エンジンの応答。
    pub response: TurnResponse,
}

/// 1局の終局理由。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminationRecord {
    /// 審判層による一方の勝利。
    AdjudicatedWin {
        /// 勝者。
        winner: StoredColor,
        /// 審判層の終局理由名。
        reason: String,
    },
    /// 審判層による引き分け。
    AdjudicatedDraw {
        /// 審判層の終局理由名。
        reason: String,
    },
    /// エンジンの投了。
    Resigned {
        /// 投了した側。
        loser: StoredColor,
    },
    /// エンジン異常による反則負け。
    Forfeit {
        /// 異常を起こした側。
        loser: StoredColor,
        /// 異常分類。
        reason: FailureKind,
    },
    /// 手数上限による打ち切り。
    Cutoff,
}

/// 開始手順の記録。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpeningRecord {
    /// 開始手順の生成に使ったシード。
    pub seed: u64,
    /// 初期局面からのUSI着手列。
    pub moves: Vec<String>,
}

/// 1局分の記録。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameRecord {
    /// この局で候補を割り当てた手番。
    pub candidate_color: StoredColor,
    /// 候補へ渡した乱数シード。
    pub candidate_seed: u64,
    /// 基準へ渡した乱数シード。
    pub baseline_seed: u64,
    /// エンジン起動から終局記録までの壁時計時間(ns)。
    pub wall_time_ns: u64,
    /// 候補エンジンのプロセスCPU時間(ns)。取得不能なら`None`。
    pub candidate_cpu_time_ns: Option<u64>,
    /// 基準エンジンのプロセスCPU時間(ns)。取得不能なら`None`。
    pub baseline_cpu_time_ns: Option<u64>,
    /// 候補エンジンが記録した最大常駐メモリ(byte)。取得不能なら`None`。
    pub candidate_peak_rss_bytes: Option<u64>,
    /// 基準エンジンが記録した最大常駐メモリ(byte)。取得不能なら`None`。
    pub baseline_peak_rss_bytes: Option<u64>,
    /// 開始手順後の全思考と応答。
    pub turns: Vec<TurnRecord>,
    /// 終局理由。
    pub termination: TerminationRecord,
}

/// 原子的に確定する1ペア分の記録。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairRecord {
    /// 1起算のペア番号。
    pub pair_number: u64,
    /// 基本シードとペア番号から派生したシード。
    pub pair_seed: u64,
    /// 両局で共有する開始手順。
    pub opening: OpeningRecord,
    /// 候補の先後を入れ替えた2局。
    pub games: [GameRecord; 2],
    /// 候補側ペア得点のペンタノミアル分類。打ち切りを含む場合は`None`。
    pub category: Option<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> GameRecord {
        GameRecord {
            candidate_color: StoredColor::Black,
            candidate_seed: 1,
            baseline_seed: 2,
            wall_time_ns: 84,
            candidate_cpu_time_ns: None,
            baseline_cpu_time_ns: None,
            candidate_peak_rss_bytes: None,
            baseline_peak_rss_bytes: None,
            turns: vec![turn()],
            termination: TerminationRecord::Resigned {
                loser: StoredColor::Black,
            },
        }
    }

    fn turn() -> TurnRecord {
        TurnRecord {
            side: StoredColor::Black,
            think_time_ns: 42,
            evaluation: None,
            stop_reason: Some(StopReasonRecord::Hard),
            completed_time_ms: Some(40),
            ponder: None,
            response: TurnResponse::Resigned,
        }
    }
    #[test]
    fn turn_record_round_trip_preserves_stop_data_and_nulls() {
        let with_values = turn();
        let value = serde_json::to_value(&with_values).unwrap();
        assert_eq!(value["stop_reason"], "hard");
        assert_eq!(value["completed_time_ms"], 40);

        let mut without_values = with_values.clone();
        without_values.stop_reason = None;
        without_values.completed_time_ms = None;
        let value = serde_json::to_value(&without_values).unwrap();
        assert!(value["stop_reason"].is_null());
        assert!(value["completed_time_ms"].is_null());
        assert_eq!(
            serde_json::from_value::<TurnRecord>(value).unwrap(),
            without_values
        );

        let mut missing = serde_json::to_value(&without_values).unwrap();
        missing.as_object_mut().unwrap().remove("stop_reason");
        assert!(serde_json::from_value::<TurnRecord>(missing).is_err());
        let mut missing = serde_json::to_value(&without_values).unwrap();
        missing.as_object_mut().unwrap().remove("completed_time_ms");
        assert!(serde_json::from_value::<TurnRecord>(missing).is_err());
    }

    #[test]
    fn stop_reason_record_uses_fixed_snake_case_words() {
        for (reason, word) in [
            (StopReasonRecord::Depth, "depth"),
            (StopReasonRecord::Nodes, "nodes"),
            (StopReasonRecord::Soft, "soft"),
            (StopReasonRecord::Hard, "hard"),
            (StopReasonRecord::External, "external"),
        ] {
            let value = serde_json::to_value(reason).unwrap();
            assert_eq!(value, word);
        }
    }

    #[test]
    fn unknown_json_field_is_rejected() {
        let mut value = serde_json::json!({
            "pair_number": 1,
            "pair_seed": 1,
            "opening": {"seed": 1, "moves": []},
            "games": [game(), game()],
            "category": null,
        });
        value
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_owned(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<PairRecord>(value).is_err());
    }
}
