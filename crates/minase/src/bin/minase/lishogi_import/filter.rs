//! 対局の除外判定と理由別の集計。

use super::input::{InputGame, time_control};
use minase_core::Color;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

/// 対局の除外理由を、serdeの`snake_case`によるJSON文字列で表す。
/// [`Report::excluded`]では、この文字列をキーとして最初に該当した理由だけに集計する。
/// 両サブコマンドとも全27種類のキーを出力し、適用しない理由も件数0と空のID配列を持つ。
/// 理由名は分類を表し、単位はない。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Exclusion {
    /// `variant`は入力の`variant`が`"chushogi"`でない場合で、両サブコマンドで数える。
    Variant,
    /// `initial_sfen`は入力に`initialSfen`欄がある場合で、nullでも両サブコマンドで数える。
    InitialSfen,
    /// `bot`は先後の対局者か`user`が欠ける、`aiLevel`が非null、または称号が`"BOT"`の場合である。
    /// 両サブコマンドで数える。
    Bot,
    /// `unrated`は入力の`rated`がtrueでない場合で、`games`でだけ数える。
    Unrated,
    /// `missing_speed`は`speed`、`clock`、`daysPerTurn`から持ち時間の区分を得られない場合である。
    /// `games`でだけ数える。
    MissingSpeed,
    /// `rating`は一方でもレーティングが欠けるか`provisional`がtrueの場合で、`games`でだけ数える。
    Rating,
    /// `status_outoftime`は入力の`status`が`"outoftime"`の場合で、`games`でだけ数える。
    StatusOutoftime,
    /// `status_timeout`は入力の`status`が`"timeout"`の場合で、`games`でだけ数える。
    StatusTimeout,
    /// `status_aborted`は入力の`status`が`"aborted"`の場合で、`games`でだけ数える。
    StatusAborted,
    /// `status_draw`は入力の`status`が`"draw"`の場合で、`games`でだけ数える。
    StatusDraw,
    /// `status_repetition`は入力の`status`が`"repetition"`の場合で、`games`でだけ数える。
    StatusRepetition,
    /// `status_perpetual_check`は入力の`status`が`"perpetualCheck"`の場合で、`games`でだけ数える。
    StatusPerpetualCheck,
    /// `status_bare_king`は入力の`status`が`"bareKing"`の場合で、`games`でだけ数える。
    StatusBareKing,
    /// `status_created`は入力の`status`が`"created"`の場合で、`games`でだけ数える。
    StatusCreated,
    /// `status_started`は入力の`status`が`"started"`の場合で、`games`でだけ数える。
    StatusStarted,
    /// `status_mate`は入力の`status`が`"mate"`の場合で、`games`でだけ数える。
    StatusMate,
    /// `status_stalemate`は入力の`status`が`"stalemate"`の場合で、`games`でだけ数える。
    StatusStalemate,
    /// `status_cheat`は入力の`status`が`"cheat"`の場合で、`games`でだけ数える。
    StatusCheat,
    /// `status_no_start`は入力の`status`が`"noStart"`の場合で、`games`でだけ数える。
    StatusNoStart,
    /// `status_unknown_finish`は入力の`status`が`"unknownFinish"`の場合で、`games`でだけ数える。
    StatusUnknownFinish,
    /// `unknown_status`は`status`が欠けるか、`resign`、`royalsLost`および上記の状態以外の場合である。
    /// `games`でだけ数える。
    UnknownStatus,
    /// `winner`は入力の`winner`が`"sente"`でも`"gote"`でもない場合で、`games`でだけ数える。
    Winner,
    /// `duplicate`は先行するメタデータ検査を通過した棋譜IDの重複で、両サブコマンドで数える。
    /// 先行する同一IDの棋譜が再生時に除外された場合も、後続の棋譜を重複として扱う。
    Duplicate,
    /// `illegal_default_rules`は`games`で着手の解析または`engine-default`規則での適用に失敗した場合である。
    /// 両サブコマンドで、候補局面の経過手数がu16の上限65,535手を超えた場合も数える。
    /// `openings`の不合法手による再生の打ち切りは、[`Report::truncated_illegal`]に記録する。
    IllegalDefaultRules,
    /// `winner_mismatch`は再生結果が引き分けか、勝者が入力と異なるか、`royalsLost`なのに終局しない場合である。
    /// `games`でだけ数える。
    WinnerMismatch,
    /// `resigner_mismatch`は未終局の再生末尾で手番側が入力の勝者と一致する場合で、`games`でだけ数える。
    ResignerMismatch,
    /// `deferred_promotion`は再生中に成りを保留した駒がある場合で、`games`でだけ数える。
    DeferredPromotion,
}

impl std::fmt::Display for Exclusion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Exclusion {}

impl Exclusion {
    const ALL: [Self; 27] = [
        Self::Variant,
        Self::InitialSfen,
        Self::Bot,
        Self::Unrated,
        Self::MissingSpeed,
        Self::Rating,
        Self::StatusOutoftime,
        Self::StatusTimeout,
        Self::StatusAborted,
        Self::StatusDraw,
        Self::StatusRepetition,
        Self::StatusPerpetualCheck,
        Self::StatusBareKing,
        Self::StatusCreated,
        Self::StatusStarted,
        Self::StatusMate,
        Self::StatusStalemate,
        Self::StatusCheat,
        Self::StatusNoStart,
        Self::StatusUnknownFinish,
        Self::UnknownStatus,
        Self::Winner,
        Self::Duplicate,
        Self::IllegalDefaultRules,
        Self::WinnerMismatch,
        Self::ResignerMismatch,
        Self::DeferredPromotion,
    ];
}

/// 1種類の対局除外理由について、件数と棋譜IDを持つJSONオブジェクトである。
/// 両サブコマンドの[`Report::excluded`]に出力し、該当がなくても両欄を省略しない。
#[derive(Default, Serialize)]
pub(super) struct Excluded {
    /// `count`は非負整数のJSON数値で、この理由で除外した入力対局の件数を表す。
    /// 両サブコマンドで出力し、`ids`の要素数に等しい。
    pub(super) count: usize,
    /// `ids`は除外した棋譜IDのJSON文字列配列で、両サブコマンドで入力順に格納する。
    /// IDに単位はなく、同じIDが複数回除外された場合は重複して格納する。
    ids: Vec<String>,
}

/// `minase data lishogi games`と`minase data lishogi openings`の`--report`へ書くJSONオブジェクトである。
/// 対局の受理件数とIDを`accepted`と`accepted_ids`に、理由別の除外を`excluded`に格納する。
/// `openings`の局面選別は`candidates`、`excluded_score`、`excluded_no_legal_moves`、`retained`に集計する。
/// 採用局面の経過手数は`retained_plies`、不合法手による打ち切り位置は`truncated_illegal`に格納する。
/// `nodes`には局面ごとの探索上限を格納し、`games`には実戦対局ごとの[`super::label::GameDetails`]を格納する。
/// `openings`では`games`欄だけを省略し、他の欄は0、空配列、空オブジェクトでも出力する。
/// `games`では全欄を出力するが、`openings`専用の集計値は0、手数の対応は空オブジェクトである。
/// 棋譜IDは入力の`id`に由来する空白を含まない空でない文字列で、単位はない。
/// ファイル全体をUTF-8の整形済みJSONとして書き、版欄は設けない。
#[derive(Serialize)]
pub(super) struct Report {
    /// `accepted`は非負整数のJSON数値で、メタデータ検査と再生を通過した対局の件数を表す。
    /// 両サブコマンドで集計し、出力局面が0件の対局も含める。
    pub(super) accepted: usize,
    /// `accepted_ids`は受理した棋譜IDのJSON文字列配列で、両サブコマンドで入力順に格納する。
    /// 要素数は`accepted`に等しい。
    pub(super) accepted_ids: Vec<String>,
    /// `excluded`は[`Exclusion`]のJSON名をキー、[`Excluded`]を値とするJSONオブジェクトである。
    /// 両サブコマンドで全27種類を出力し、各入力対局を最初に該当した除外理由だけに数える。
    pub(super) excluded: BTreeMap<Exclusion, Excluded>,
    /// `candidates`は非負整数のJSON数値で、`openings`の選別前の候補局面数を表す。
    /// 候補は未終局で、経過40手以上の20手ごと、盤上47枚以上、成りを保留した駒がない局面とする。
    /// `excluded_score + excluded_no_legal_moves + retained`に等しく、`games`では0を出力する。
    pub(super) candidates: usize,
    /// `excluded_score`は非負整数のJSON数値で、`openings`で探索値の絶対値が500センチポーンを超えた局面数を表す。
    /// 合法手のない候補は含めず、`games`では0を出力する。
    pub(super) excluded_score: usize,
    /// `excluded_no_legal_moves`は非負整数のJSON数値で、`openings`で探索前に合法手がなく除外した候補局面数を表す。
    /// `games`では0を出力する。
    pub(super) excluded_no_legal_moves: usize,
    /// `retained`は非負整数のJSON数値で、`openings`の局面一覧へ書いた局面数を表す。
    /// `games`では0を出力する。
    pub(super) retained: usize,
    /// `retained_plies`は棋譜IDをキー、採用局面の経過手数の配列を値とするJSONオブジェクトである。
    /// 手数は初期局面を0手とする非負整数のJSON数値で、片側の1着手を1手と数える。
    /// `openings`では受理した全対局を含み、配列は昇順で、採用局面がなければ空になる。
    /// `games`では空オブジェクトを出力する。
    pub(super) retained_plies: BTreeMap<String, Vec<u32>>,
    /// `truncated_illegal`は棋譜IDをキー、再生を打ち切った最初の不合法手の番号を値とするJSONオブジェクトである。
    /// 番号は初手を1とする正整数のJSON数値で、片側の1着手を1手と数える。
    /// `openings`で着手の解析または適用に失敗した受理対局だけを格納し、`games`では空オブジェクトを出力する。
    pub(super) truncated_illegal: BTreeMap<String, u32>,
    /// `nodes`は1〜4,294,967,295の整数のJSON数値で、1局面の探索に設定するノード上限を表す。
    /// 単位はノードで、両サブコマンドとも`--nodes`の値を出力する。
    nodes: u32,
    /// `games`は棋譜IDをキー、[`super::label::GameDetails`]を値とするJSONオブジェクトである。
    /// `games`サブコマンドでは受理した全対局を含み、受理が0件でも空オブジェクトを出力する。
    /// `openings`では欄自体を省略する。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) games: Option<BTreeMap<String, super::label::GameDetails>>,
}

impl Report {
    pub(super) fn new(nodes: u32) -> Self {
        Self {
            accepted: 0,
            accepted_ids: Vec::new(),
            excluded: Exclusion::ALL
                .into_iter()
                .map(|r| (r, Excluded::default()))
                .collect(),
            candidates: 0,
            excluded_score: 0,
            excluded_no_legal_moves: 0,
            retained: 0,
            retained_plies: BTreeMap::new(),
            truncated_illegal: BTreeMap::new(),
            nodes,
            games: None,
        }
    }
    pub(super) fn exclude(&mut self, reason: Exclusion, id: String) {
        let entry = self.excluded.entry(reason).or_default();
        entry.count += 1;
        entry.ids.push(id);
    }
}

pub(super) fn metadata_exclusion(
    game: &InputGame,
    openings: bool,
    seen: &mut HashSet<String>,
) -> Option<Exclusion> {
    use Exclusion::*;
    if game.variant.as_deref() != Some("chushogi") {
        return Some(Variant);
    }
    if game.initial_sfen.is_some() {
        return Some(InitialSfen);
    }
    let players = game
        .players
        .as_ref()
        .and_then(|p| Some([p.sente.as_ref()?, p.gote.as_ref()?]));
    let Some(players) = players else {
        return Some(Bot);
    };
    if players.iter().any(|p| {
        p.ai_level.is_some()
            || p.user
                .as_ref()
                .is_none_or(|u| u.title.as_deref() == Some("BOT"))
    }) {
        return Some(Bot);
    }
    if !openings {
        if game.rated != Some(true) {
            return Some(Unrated);
        }
        if time_control(game).is_none() {
            return Some(MissingSpeed);
        }
        if players
            .iter()
            .any(|p| p.rating.is_none() || p.provisional == Some(true))
        {
            return Some(Rating);
        }
        let status = match game.status.as_deref() {
            Some("resign" | "royalsLost") => None,
            Some("outoftime") => Some(StatusOutoftime),
            Some("timeout") => Some(StatusTimeout),
            Some("aborted") => Some(StatusAborted),
            Some("draw") => Some(StatusDraw),
            Some("repetition") => Some(StatusRepetition),
            Some("perpetualCheck") => Some(StatusPerpetualCheck),
            Some("bareKing") => Some(StatusBareKing),
            Some("created") => Some(StatusCreated),
            Some("started") => Some(StatusStarted),
            Some("mate") => Some(StatusMate),
            Some("stalemate") => Some(StatusStalemate),
            Some("cheat") => Some(StatusCheat),
            Some("noStart") => Some(StatusNoStart),
            Some("unknownFinish") => Some(StatusUnknownFinish),
            _ => Some(UnknownStatus),
        };
        if status.is_some() {
            return status;
        }
        if winner(game).is_none() {
            return Some(Winner);
        }
    }
    if !seen.insert(game.id.clone()) {
        return Some(Duplicate);
    }
    None
}

pub(super) fn winner(game: &InputGame) -> Option<Color> {
    match game.winner.as_deref() {
        Some("sente") => Some(Color::Black),
        Some("gote") => Some(Color::White),
        _ => None,
    }
}
