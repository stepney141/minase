//! 探索の共通試験補助。

// 領域D7（探索と評価）のspec-firstテスト。
//
// 期待値の根拠は挙動マトリクスd7-search-eval.md、docs/plans/search.mdの
// 規範節、およびRULES.md第20〜23条に限る。実装は到達手段（API形状）の
// 把握にだけ使い、期待定数を実装から写していない。
// 座標はマトリクスの筋段表記（筋1=先手から見て右端、段1=後手側最奥）を
// `fs`ヘルパで内部座標へ写して使う。

mod captures;
mod contracts;
mod correction;
mod handle;
mod limits;
mod negamax;
mod ordering;
mod ponder;
mod pruning;
mod quiesce;
mod root;
mod royal;
mod scoring;
mod team;
mod time;
mod tt;
mod tuning;

use core::cmp::Reverse;
use core::num::{NonZeroU64, NonZeroUsize};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::core::board::BOARD_SQUARE_COUNT;
use crate::core::mv::Move;
use crate::core::piece::{COLOR_COUNT, Color, PieceCode, PieceKind};
use crate::core::position::Position;
use crate::core::rules::MoveRules;
use crate::eval::pst::{PIECE_STATE_COUNT, piece_state_of};
use crate::eval::{Pst, evaluate, weights};
use crate::search::alphabeta::INFINITY;
use crate::search::alphabeta::correction::material_key;
use crate::search::alphabeta::deepening::{
    auxiliary_depths, run_auxiliary_worker, run_main_worker,
};
use crate::search::alphabeta::ordering::{
    MoveOrderKey, MovePicker, MovePickerStage, move_order_key, order_captures,
};
use crate::search::alphabeta::pruning::{
    capture_is_pruned_by_see, lmr_reduction, lmr_table, null_move_reduction,
};
use crate::search::alphabeta::quiesce::{CaptureRanks, QsearchBuffers};
use crate::search::alphabeta::root::{AspirationWindow, aspiration_delta, grow_aspiration_delta};
use crate::search::alphabeta::royal::{captures_last_royal, royal_under_attack};
use crate::search::alphabeta::searcher::{
    HistoryTable, KILLER_COUNT, PonderIteration, STOP_CHECK_INTERVAL, Searcher, new_searcher,
};
use crate::search::alphabeta::see::see_prunes;
use crate::search::alphabeta::team::{
    HardLimit, SharedSearch, WorkerOutcome, run_search_team, run_worker_team, select_worker_outcome,
};
use crate::search::alphabeta::time::{
    TimeBudget, clock_budget, iteration_prediction_fits, moves_to_go, should_start_next_iteration,
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
use crate::search::snapshot::{SearchSnapshot, search_key};
use crate::search::{
    DEFAULT_THREADS, DRAW_SCORE, MATE, MATE_THRESHOLD, MAX_PLY, TranspositionTable,
    TranspositionTableError,
};
use crate::test_util::{position, position_from_codes, sq};
use crate::{MoveGenerator, Square};

// ---------------------------------------------------------------------------
// ヘルパ
// ---------------------------------------------------------------------------

fn engine_rules() -> MoveRules {
    MoveRules::standard()
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

/// 時間制限のない単一ワーカーで根の窓と置換表を検査する。
fn with_root_searcher(position: &Position, history: &[u64], test: impl FnOnce(&mut Searcher<'_>)) {
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
    let pst = weights().unwrap();
    let table = small_tt();
    let mut searcher = new_searcher(&pst, position, engine_rules(), history, &shared, &table);
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
    let pst = crate::eval::weights().unwrap();
    let mut searcher = new_searcher(&pst, position, engine_rules(), &history, &shared, table);
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
    let pst = crate::eval::weights().unwrap();
    let mut searcher = new_searcher(&pst, position, engine_rules(), &history, &shared, table);
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
        crate::eval::weights().unwrap(),
        snapshot,
        limits,
        search_id,
        threads,
        tt,
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
        history_keys.to_vec(),
        root_moves.to_vec(),
    )
    .expect("test root moves must not be empty");
    let pst = crate::eval::weights().unwrap();
    crate::search::search(&pst, &snapshot, limits, threads, tt)
        .expect("valid test input must be searchable")
}

fn snapshot_for(position: &Position) -> SearchSnapshot {
    SearchSnapshot::from_parts(
        position.clone(),
        engine_rules(),
        vec![search_key(position)],
        legal_moves(position),
    )
    .expect("test position must have a legal move")
}

/// 静穏な中盤フィクスチャ。浅い深さで王駒捕獲や強制手順が現れないよう、
/// 双方の駒を離して置く（D7-API系の前提「静穏な中盤局面」）。
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

/// D7-SRCH-03／D7-TT-04共用のフィクスチャ。先手王将は飛車（十二段の横利き）
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
        weights().unwrap(),
        snapshot_for(&Position::initial()),
        limits,
        700,
        worker_count(threads),
        small_tt(),
        true,
    )
}

/// 指定した期間に到着する進捗を消費し、終了していないことを確かめる。
fn assert_ponder_running(handle: &SearchHandle, duration: Duration) {
    let until = Instant::now() + duration;
    while let Some(remaining) = until.checked_duration_since(Instant::now()) {
        match handle.events().recv_timeout(remaining) {
            Ok(SearchEvent::Progress { .. }) => {}
            Ok(event) => panic!("ponder ended before its hit: {event:?}"),
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(error) => panic!("ponder disconnected: {error}"),
        }
    }
}
