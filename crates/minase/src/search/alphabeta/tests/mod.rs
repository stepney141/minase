//! 探索の共通試験補助。

// RULES.md第20条から第23条により、最後の王駒の捕獲と合法手なしを負けとし、王駒が残る間は探索を続ける。
// 探索は捕獲までを含む手数で詰み値を表し、深さ0では静止探索へ移る。
// 第31条R1の反復は1周で攻撃側の負けを近似し、双方またはどちらも攻撃側でなければ引き分けとする。
// 近似の理由は docs/adr/approximate-adjudication-inside-search-tree.md にある。
// 個々の期待値はこの契約と各テストの仕様から求め、期待定数を実装から写さない。
// 座標は筋段表記（筋1=先手から見て右端、段1=後手側最奥）を
// `fs`ヘルパで内部座標へ写して使う。

mod capture_history;
mod captures;
mod correction;
mod draw_value;
mod handle;
mod history;
mod improving;
mod late_move;
mod limits;
mod negamax;
mod ordering;
mod partial;
mod ponder;
mod pruning;
mod qsearch_move_limit;
mod quiesce;
mod repetition_band;
mod repetition_prohibition;
mod repetition_tree;
mod root;
mod root_repetition;
mod royal;
mod scoring;
#[cfg(feature = "search-stats")]
mod stats;
mod team;
mod time;
mod tt;
mod tuning;

use core::cmp::Reverse;
use core::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::eval::PIECE_STATE_COUNT;
use crate::eval::{Nnue, evaluate};
use crate::search::alphabeta::INFINITY;
use crate::search::alphabeta::correction::material_key;
use crate::search::alphabeta::deepening::{
    auxiliary_depths, run_auxiliary_worker, run_main_worker,
};
use crate::search::alphabeta::history::HistoryTable;
use crate::search::alphabeta::ordering::{
    MoveOrderKey, MovePicker, MovePickerStage, move_order_key, order_captures,
};
use crate::search::alphabeta::pruning::{
    capture_is_pruned_by_see, lmr_reduction, lmr_table, null_move_reduction,
};
use crate::search::alphabeta::quiesce::{CaptureRanks, QsearchBuffers};
use crate::search::alphabeta::root::{AspirationWindow, aspiration_delta};
use crate::search::alphabeta::royal::{captures_last_royal, royal_under_attack};
use crate::search::alphabeta::searcher::{
    KILLER_COUNT, PathMove, PonderIteration, RepetitionMatch, STOP_CHECK_INTERVAL, Searcher,
    new_searcher,
};
use crate::search::alphabeta::see::see_prunes;
use crate::search::alphabeta::team::{
    HardLimit, SharedSearch, WorkerOutcome, run_search_team, run_worker_team, select_worker_outcome,
};
use crate::search::alphabeta::time::{
    TimeBudget, clock_budget, iteration_prediction_fits, should_start_next_iteration,
    stable_signal, time_budget,
};
use crate::search::alphabeta::tt::{
    ADVISORY_GENERATION_MASK, ADVISORY_GENERATION_SHIFT, ADVISORY_MOVE_MASK, Bound,
    CRITICAL_BOUND_MASK, CRITICAL_RESERVED_MASK, pack_move, unpack_move,
};
use crate::search::error::SearchError;
use crate::search::events::{SearchEvent, SearchResult, StopReason};
use crate::search::handle::SearchHandle;
use crate::search::limits::{ClockLimits, SearchLimits};
use crate::search::snapshot::{SearchRepetition, SearchSnapshot, search_key};
use crate::search::{
    DEFAULT_THREADS, DRAW_SCORE, MATE, MATE_THRESHOLD, MAX_PLY, TranspositionTable,
    TranspositionTableError,
};
use minase_core::board::BOARD_SQUARE_COUNT;
use minase_core::mv::Move;
use minase_core::piece::{COLOR_COUNT, Color, PieceCode, PieceKind};
use minase_core::position::Position;
use minase_core::rules::MoveRules;
use minase_core::test_util::{position, position_from_codes, sq};
use minase_core::{MoveGenerator, Square, repetition_key};

// ---------------------------------------------------------------------------
// ヘルパ
// ---------------------------------------------------------------------------

/// 対角の隅に王将だけを置き、先手が指す静穏局面。
fn bare_kings() -> Position {
    position(
        Color::Black,
        &[
            (sq(0, 11), Color::White, PieceKind::King),
            (sq(11, 0), Color::Black, PieceKind::King),
        ],
    )
}

fn engine_rules() -> MoveRules {
    MoveRules::standard()
}

fn step(from: Square, to: Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: false,
    }
}

/// 黒飛は3筋の白歩、白飛は8筋の黒歩を、それぞれ往復する2升から照準する。
fn rook_cycle(black_attacks: bool, white_attacks: bool) -> (Position, [Move; 4]) {
    let mut pieces = vec![
        (sq(0, 0), Color::Black, PieceKind::King),
        (sq(3, 3), Color::Black, PieceKind::Rook),
        (sq(8, 8), Color::White, PieceKind::Rook),
        (sq(11, 11), Color::White, PieceKind::King),
    ];
    if black_attacks {
        pieces.push((sq(3, 9), Color::White, PieceKind::Pawn));
    }
    if white_attacks {
        pieces.push((sq(8, 1), Color::Black, PieceKind::Pawn));
    }
    (
        position(Color::Black, &pieces),
        [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ],
    )
}

fn complete_cycle(searcher: &mut Searcher<'_>, current: &mut Position, mv: Move) -> i32 {
    let before = current.clone();
    let keys = searcher.path_keys.clone();
    let ply = searcher.path_moves.len() as u32;
    let score = searcher
        .search_move(current, mv, 1, -INFINITY, INFINITY, ply, true, 0)
        .unwrap();
    assert_eq!(*current, before);
    assert_eq!(searcher.path_keys, keys);
    assert_eq!(searcher.path_moves.len() + 1, searcher.path_keys.len());
    score
}

/// 実着手またはnull moveを適用し、対応する経路と各plyの局面状態を積む。
fn push_path_move(searcher: &mut Searcher<'_>, position: &mut Position, mv: Option<Move>) {
    if let Some(mv) = mv {
        let mut moves = Vec::new();
        searcher.generator.generate_moves(position, &mut moves);
        assert!(moves.contains(&mv));
        position.make_move_unchecked(mv, searcher.rules);
    } else {
        position.make_null_move();
        searcher.null_move_boundary = Some(searcher.path_keys.len());
        searcher.null_move_ply = Some(searcher.path_keys.len() as u32);
    }
    searcher
        .path_keys
        .push(repetition_key(searcher.repetition.rule(), position));
    searcher.path_moves.push(PathMove {
        mv,
        attacking: None,
    });
    let ply = searcher.path_moves.len();
    searcher.accumulators[ply] = searcher.nnue.refresh_accumulator(position);
    searcher.material_keys[ply] = material_key(position);
    assert_eq!(searcher.path_moves.len() + 1, searcher.path_keys.len());
}

/// 筋段表記（筋1〜12、段1〜12）を内部座標へ写す。
/// 段12が先手側最奥（内部rank 0）、筋1が内部file 11に対応する。
fn fs(file: u8, dan: u8) -> Square {
    sq(12 - file, 12 - dan)
}

fn legal_moves(position: &Position) -> Vec<Move> {
    let mut moves = Vec::new();
    MoveGenerator::new(engine_rules()).generate_moves(position, &mut moves);
    moves
}

/// 成駒を含むテスト局面を駒種と成否から構築する。
fn position_with_promoted_pieces(
    side_to_move: Color,
    pieces: &[(Square, Color, PieceKind, bool)],
) -> Position {
    let codes: Vec<_> = pieces
        .iter()
        .map(|&(square, color, kind, promoted)| {
            let piece = if promoted {
                PieceCode::new_promoted(color, kind).expect("test fixture promotion must be valid")
            } else {
                PieceCode::new(color, kind).expect("test fixture piece must allow unpromoted state")
            };
            (square, piece)
        })
        .collect();
    position_from_codes(side_to_move, &codes)
}

fn depth_limits(depth: u32) -> SearchLimits {
    SearchLimits::new(Some(depth), None, None, None).expect("test depth must be valid")
}

fn nodes_limits(nodes: u64) -> SearchLimits {
    SearchLimits::new(None, Some(nodes), None, None).expect("test node limit must be valid")
}

fn movetime_limits(milliseconds: u64) -> SearchLimits {
    SearchLimits::new(None, None, Some(milliseconds), None).expect("test movetime must be valid")
}

fn infinite_limits() -> SearchLimits {
    SearchLimits::infinite()
}

fn clock(remaining_ms: u64, increment_ms: u64, byoyomi_ms: u64) -> ClockLimits {
    clock_at_ply(remaining_ms, increment_ms, byoyomi_ms, 0)
}

fn clock_at_ply(remaining_ms: u64, increment_ms: u64, byoyomi_ms: u64, ply: u32) -> ClockLimits {
    ClockLimits::new(remaining_ms, increment_ms, byoyomi_ms, ply)
        .expect("test clock must contain non-zero time")
}

fn small_tt() -> TranspositionTable {
    TranspositionTable::new(1).unwrap()
}

/// 学習済みネットに依存せず、全探索テストで同じ固定ネットを使う。
pub(super) fn test_nnue() -> &'static Nnue {
    shared_test_nnue()
}

/// 非同期探索にも同じネットを渡せるよう、所有権を共有する。
fn shared_test_nnue() -> &'static Arc<Nnue> {
    static NNUE: OnceLock<Arc<Nnue>> = OnceLock::new();
    NNUE.get_or_init(|| Arc::new(epoch_zero_nnue()))
}

fn epoch_zero_nnue() -> Nnue {
    Nnue::decode(&epoch_zero_nnue_bytes()).unwrap()
}

// NNUE形式のヘッダ長。eval::nnue::formatの定数は非公開なので、探索の試験で共有する。
pub(super) const NNUE_HEADER_LENGTH: usize = 192;

/// エポック0のP型の行を探索の特徴集合へ複写し、重みを変更するテストにも渡す。
pub(super) fn epoch_zero_nnue_bytes() -> Vec<u8> {
    use crate::eval::features::FEATURE_COUNT;
    use crate::eval::nnue::{ActiveFeatureSet, feature_set::FeatureSet};
    use sha2::{Digest, Sha256};

    let source = include_bytes!("../../../../nets/nnue-epoch0.bin");
    let width = u32::from_le_bytes(source[104..108].try_into().unwrap()) as usize;
    let hidden = NNUE_HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
    let end = hidden + FEATURE_COUNT * width * 2;
    let mut bytes = source[..hidden].to_vec();
    let architecture = format!(
        "features={};ft={width};linear=tapered2;head=direct-screlu",
        ActiveFeatureSet::NAME
    );
    bytes[8..104].fill(0);
    bytes[8..8 + architecture.len()].copy_from_slice(architecture.as_bytes());
    for _ in 0..ActiveFeatureSet::DIMENSIONS / FEATURE_COUNT {
        bytes.extend_from_slice(&source[hidden..end]);
    }
    bytes.extend_from_slice(&source[end..]);
    let checksum = Sha256::digest(&bytes[NNUE_HEADER_LENGTH..]);
    bytes[144..176].copy_from_slice(&checksum);
    bytes
}

/// 時間制限のない単一ワーカーで根の窓と置換表を検査する。
fn with_root_searcher(position: &Position, history: &[u64], test: impl FnOnce(&mut Searcher<'_>)) {
    with_nnue_root_searcher(test_nnue(), position, history, test);
}

/// 明示した重みを使い、時間制限のない単一ワーカーで根を検査する。
fn with_nnue_root_searcher(
    nnue: &Nnue,
    position: &Position,
    history: &[u64],
    test: impl FnOnce(&mut Searcher<'_>),
) {
    with_repetition_root_searcher(
        nnue,
        position,
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        history,
        test,
    );
}

/// 着手規則と反復規則を指定し、時間制限のない単一ワーカーで検査する。
fn with_repetition_root_searcher(
    nnue: &Nnue,
    position: &Position,
    rules: MoveRules,
    repetition: SearchRepetition,
    history: &[u64],
    test: impl FnOnce(&mut Searcher<'_>),
) {
    with_draw_value_root_searcher(nnue, position, rules, repetition, DRAW_SCORE, history, test);
}

/// RULES.md第31条R1の引き分けの値も指定し、時間制限のない単一ワーカーで検査する。
fn with_draw_value_root_searcher(
    nnue: &Nnue,
    position: &Position,
    rules: MoveRules,
    repetition: SearchRepetition,
    draw_value: i32,
    history: &[u64],
    test: impl FnOnce(&mut Searcher<'_>),
) {
    let external_stop = AtomicBool::new(false);
    let shared = SharedSearch {
        external_stop: &external_stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        node_limit: None,
        started: Instant::now(),
        hard_limit: None,
    };
    let table = small_tt();
    let mut butterfly_history =
        Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    let mut capture_history = crate::search::alphabeta::capture_history::CaptureHistory::new();
    let mut searcher = new_searcher(
        nnue,
        position,
        rules,
        repetition,
        draw_value,
        history,
        &shared,
        &table,
        &mut butterfly_history,
        &mut capture_history,
    );
    test(&mut searcher);
}

/// 全ルート手が反復による引き分けとなる履歴を作る。
fn repeated_root_children(position: &Position, moves: &[Move]) -> Vec<u64> {
    moves
        .iter()
        .map(|&mv| {
            let mut child = position.clone();
            child.make_move_unchecked(mv, engine_rules());
            search_key(&child)
        })
        // 根を履歴の末尾に含め、どの子についても両対局者の非攻撃的な履歴を含む長さにする。
        .chain([search_key(position); 2])
        .collect()
}

/// 静止探索を直接実行し、評価値と当該Searcherの実着手の適用回数を返す。
fn run_quiesce(
    position: &Position,
    alpha: i32,
    beta: i32,
    ply: u32,
    table: &TranspositionTable,
) -> (i32, u64) {
    let external_stop = AtomicBool::new(false);
    let shared = SharedSearch {
        external_stop: &external_stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        node_limit: None,
        started: Instant::now(),
        hard_limit: None,
    };
    let history: Vec<u64> = Vec::new();
    let mut current = position.clone();
    let nnue = test_nnue();
    let mut butterfly_history =
        Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    let mut capture_history = crate::search::alphabeta::capture_history::CaptureHistory::new();
    let mut searcher = new_searcher(
        nnue,
        position,
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        DRAW_SCORE,
        &history,
        &shared,
        table,
        &mut butterfly_history,
        &mut capture_history,
    );
    let score = searcher
        .quiesce(&mut current, alpha, beta, ply)
        .expect("unlimited quiescence search must complete");
    (score, searcher.nodes)
}

/// 通常探索を直接実行し、評価値と当該Searcherの実着手の適用回数を返す。
fn run_negamax(
    position: &Position,
    depth: u32,
    alpha: i32,
    beta: i32,
    ply: u32,
    table: &TranspositionTable,
) -> (i32, u64) {
    let external_stop = AtomicBool::new(false);
    let shared = SharedSearch {
        external_stop: &external_stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        node_limit: None,
        started: Instant::now(),
        hard_limit: None,
    };
    let history: Vec<u64> = Vec::new();
    let mut current = position.clone();
    let nnue = test_nnue();
    let mut butterfly_history =
        Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    let mut capture_history = crate::search::alphabeta::capture_history::CaptureHistory::new();
    let mut searcher = new_searcher(
        nnue,
        position,
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        DRAW_SCORE,
        &history,
        &shared,
        table,
        &mut butterfly_history,
        &mut capture_history,
    );
    let score = searcher
        .negamax(&mut current, depth, alpha, beta, ply)
        .expect("unlimited negamax search must complete");
    (score, searcher.nodes)
}

fn worker_count(count: usize) -> NonZeroUsize {
    NonZeroUsize::new(count).expect("test worker count must be non-zero")
}

/// 検証済み入力で非同期探索を開始するテスト用ヘルパ。
fn start(
    snapshot: SearchSnapshot,
    limits: SearchLimits,
    search_id: u64,
    threads: NonZeroUsize,
    tt: TranspositionTable,
) -> SearchHandle {
    crate::search::start_search(
        Arc::clone(shared_test_nnue()),
        snapshot,
        limits,
        search_id,
        threads,
        tt,
        crate::search::HistoryTables::new(threads),
        false,
    )
}

/// 個別の探索入力をスナップショットへまとめて同期探索するテスト用ヘルパ。
#[allow(clippy::too_many_arguments)]
fn run_search(
    position: &Position,
    rules: MoveRules,
    root_moves: &[Move],
    history_keys: &[u64],
    limits: &SearchLimits,
    threads: NonZeroUsize,
    tt: &mut TranspositionTable,
) -> SearchResult {
    let snapshot = SearchSnapshot::from_parts(
        position.clone(),
        rules,
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        history_keys.to_vec(),
        root_moves.to_vec(),
    )
    .expect("test root moves must not be empty");
    let nnue = test_nnue();
    crate::search::search(
        nnue,
        &snapshot,
        limits,
        threads,
        tt,
        &mut crate::search::HistoryTables::new(threads),
    )
    .expect("valid test input must be searchable")
}

fn snapshot_for(position: &Position) -> SearchSnapshot {
    SearchSnapshot::from_parts(
        position.clone(),
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        vec![search_key(position)],
        legal_moves(position),
    )
    .expect("test position must have a legal move")
}

/// 静穏な中盤フィクスチャ。浅い深さで王駒捕獲や強制手順が現れないよう、
/// 双方の駒を離して置く。
fn quiet_midgame() -> Position {
    position(
        Color::Black,
        &[
            (fs(7, 12), Color::Black, PieceKind::King),
            (fs(6, 11), Color::Black, PieceKind::GoldGeneral),
            (fs(10, 9), Color::Black, PieceKind::Rook),
            (fs(5, 11), Color::Black, PieceKind::SilverGeneral),
            (fs(7, 9), Color::Black, PieceKind::Pawn),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(7, 2), Color::White, PieceKind::GoldGeneral),
            (fs(3, 3), Color::White, PieceKind::Bishop),
            (fs(8, 2), Color::White, PieceKind::CopperGeneral),
            (fs(6, 4), Color::White, PieceKind::Pawn),
        ],
    )
}

/// 捕獲手と静かな手を同時に持つ手選択フィクスチャ。
fn staged_picker_fixture() -> Position {
    position(
        Color::Black,
        &[
            (fs(7, 12), Color::Black, PieceKind::King),
            (fs(6, 8), Color::Black, PieceKind::Rook),
            (fs(6, 5), Color::White, PieceKind::Pawn),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    )
}

/// 反復判定と置換表の検証で共有するフィクスチャ。先手王将は飛車（十二段の横利き）
/// と奔王（12一〜1十二の斜線）の利きに入っており、反車6六→6七だけが
/// 履歴と同一キーの子局面を作る。
fn repetition_fixture() -> Position {
    position(
        Color::Black,
        &[
            (fs(1, 12), Color::Black, PieceKind::King),
            (fs(6, 6), Color::Black, PieceKind::ReverseChariot),
            (fs(12, 2), Color::White, PieceKind::King),
            (fs(12, 1), Color::White, PieceKind::FreeKing),
            (fs(1, 1), Color::White, PieceKind::VerticalMover),
            (fs(12, 12), Color::White, PieceKind::Rook),
        ],
    )
}

/// 反車6六→6七（非捕獲手）。repetition_fixtureの反復再現手。
fn repetition_move() -> Move {
    Move {
        from: fs(6, 6),
        mid: None,
        to: fs(6, 7),
        promote: false,
    }
}

/// `Finished`イベントの中身。
struct FinishedReport {
    best_move: Move,
    score: i32,
    depth: u32,
    nodes: u64,
    elapsed: Duration,
    pv: Vec<Move>,
    stop_reason: StopReason,
}

/// `Finished`が届くまで全イベントを受信して返す。
fn drain_raw(handle: &SearchHandle) -> Vec<SearchEvent> {
    let mut events = Vec::new();
    loop {
        let event = handle
            .events()
            .recv_timeout(Duration::from_secs(60))
            .expect("search must finish within the timeout");
        let finished = matches!(event, SearchEvent::Finished { .. });
        events.push(event);
        if finished {
            return events;
        }
    }
}

/// イベント列を`Progress`の内訳と`Finished`へ分解する。
#[allow(clippy::type_complexity)]
fn event_reports(
    events: Vec<SearchEvent>,
) -> (Vec<(u32, i32, u64, Duration, Vec<Move>)>, FinishedReport) {
    let mut progress = Vec::new();
    for event in events {
        match event {
            SearchEvent::Progress {
                depth,
                score,
                nodes,
                elapsed,
                pv,
                ..
            } => progress.push((depth, score, nodes, elapsed, pv)),
            SearchEvent::Finished {
                best_move,
                score,
                depth,
                nodes,
                elapsed,
                pv,
                stop_reason,
                ..
            } => {
                return (
                    progress,
                    FinishedReport {
                        best_move,
                        score,
                        depth,
                        nodes,
                        elapsed,
                        pv,
                        stop_reason,
                    },
                );
            }
        }
    }
    unreachable!("drain_raw always ends with a Finished event");
}

/// PVの各手を先頭から適用し、それぞれの局面で合法であることを確認する。
fn assert_pv_is_legal(root: &Position, pv: &[Move]) {
    let mut current = root.clone();
    for &mv in pv {
        assert!(
            legal_moves(&current).contains(&mv),
            "PVの各手はその変化を順に進めた局面で合法でなければならない"
        );
        current.make_move_unchecked(mv, engine_rules());
    }
}

fn start_ponder(limits: SearchLimits, threads: usize) -> SearchHandle {
    crate::search::start_search(
        Arc::clone(shared_test_nnue()),
        snapshot_for(&Position::initial()),
        limits,
        700,
        worker_count(threads),
        small_tt(),
        crate::search::HistoryTables::new(worker_count(threads)),
        true,
    )
}
