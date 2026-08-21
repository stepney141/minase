// 領域D7（探索と評価）のspec-firstテスト。
//
// 期待値の根拠は挙動マトリクスd7-search-eval.md、docs/plans/search.mdの
// 規範節、およびRULES.md第20〜23条に限る。実装は到達手段（API形状）の
// 把握にだけ使い、期待定数を実装から写していない。
// 座標はマトリクスの筋段表記（筋1=先手から見て右端、段1=後手側最奥）を
// `fs`ヘルパで内部座標へ写して使う。

use super::*;
use crate::Square;
use crate::core::piece::{Color, PieceKind};
use crate::test_util::{position, sq};

use super::tt::{
    ADVISORY_GENERATION_MASK, ADVISORY_GENERATION_SHIFT, ADVISORY_MOVE_MASK, ADVISORY_MOVE_SHIFT,
    ADVISORY_RESERVED_MASK, Bound, CRITICAL_BOUND_MASK, CRITICAL_BOUND_SHIFT, CRITICAL_DEPTH_MASK,
    CRITICAL_DEPTH_SHIFT, CRITICAL_KEY_MASK, CRITICAL_KEY_SHIFT, CRITICAL_RESERVED_MASK,
    CRITICAL_SCORE_MASK, CRITICAL_SCORE_SHIFT, entry_size, pack_move, unpack_move,
};

// ---------------------------------------------------------------------------
// ヘルパ
// ---------------------------------------------------------------------------

fn engine_rules() -> Rules {
    Rules::engine_default()
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

fn no_limits() -> SearchLimits {
    SearchLimits {
        depth: None,
        nodes: None,
        movetime_ms: None,
        clock: None,
        infinite: false,
    }
}

fn depth_limits(depth: u32) -> SearchLimits {
    SearchLimits {
        depth: Some(depth),
        ..no_limits()
    }
}

fn nodes_limits(nodes: u64) -> SearchLimits {
    SearchLimits {
        nodes: Some(nodes),
        ..no_limits()
    }
}

fn movetime_limits(milliseconds: u64) -> SearchLimits {
    SearchLimits {
        movetime_ms: Some(milliseconds),
        ..no_limits()
    }
}

fn infinite_limits() -> SearchLimits {
    SearchLimits {
        infinite: true,
        ..no_limits()
    }
}

fn clock(remaining_ms: u64, increment_ms: u64, byoyomi_ms: u64) -> ClockLimits {
    ClockLimits {
        remaining_ms,
        increment_ms,
        byoyomi_ms,
    }
}

fn small_tt() -> TranspositionTable {
    TranspositionTable::new(1)
}

fn worker_count(count: usize) -> NonZeroUsize {
    NonZeroUsize::new(count).expect("test worker count must be non-zero")
}

fn snapshot_for(position: &Position) -> SearchSnapshot {
    SearchSnapshot {
        root_moves: legal_moves(position),
        history_keys: vec![search_key(position)],
        position: position.clone(),
        rules: engine_rules(),
    }
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
fn drain_events(
    handle: &SearchHandle,
) -> (Vec<(u32, i32, u64, Duration, Vec<Move>)>, FinishedReport) {
    let mut progress = Vec::new();
    for event in drain_raw(handle) {
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

// ---------------------------------------------------------------------------
// D7-SRCH　探索本体
// ---------------------------------------------------------------------------

// D7-SRCH-01。search.md「探索内の終局と規則処理」: 相手の最後の王駒を取る
// 着手はMATE−ply。根ply=0のためMATE=30000。RULES.md第21条第1項。
#[test]
fn depth_one_capture_of_the_last_royal_scores_mate() {
    // 後手: 玉将6一（唯一の王駒）。先手: 飛車6十、王将6十二。先手番。
    let position = position(
        Color::Black,
        &[
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::Rook),
            (fs(6, 12), Color::Black, PieceKind::King),
        ],
    );
    let moves = legal_moves(&position);

    let result = search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    // 捕獲升は敵陣のため成・不成の2通りがあるが、いずれも同じ捕獲で
    // 同値のMATEになる。移動元と到達升だけを固定する（SPEC_UNCLEAR-07）。
    assert_eq!(result.best_move.from, fs(6, 10));
    assert_eq!(result.best_move.to, fs(6, 1));
    assert_eq!(result.best_move.mid, None);
    assert_eq!(result.score, MATE);
}

// D7-SRCH-02。search.md「探索内の終局と規則処理」: 王駒を2枚持つ側の
// 1枚目の王駒は駒割上の損得。RULES.md第20条第3〜5項。
#[test]
fn capture_of_the_first_of_two_royals_scores_as_material_gain() {
    // 後手: 玉将6一、太子8三（王駒2枚）。先手: 飛車8十、王将6十二。先手番。
    let position = position(
        Color::Black,
        &[
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(8, 3), Color::White, PieceKind::CrownPrince),
            (fs(8, 10), Color::Black, PieceKind::Rook),
            (fs(6, 12), Color::Black, PieceKind::King),
        ],
    );
    let moves = legal_moves(&position);

    let result = search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    assert_eq!(result.best_move.from, fs(8, 10));
    assert_eq!(result.best_move.to, fs(8, 3));
    // 太子1枚の喪失は終局ではなく通常評価値帯（絶対値29000未満）に
    // とどまり、詰み帯には入らない（INV-3）。
    assert!(result.score > 0);
    assert!(result.score < 29_000);
}

// D7-SRCH-03。search.md「探索内の終局と規則処理」: 反復は探索局面キーの
// スタックで検出し引き分けスコアで返す近似。引き分け0は詰み帯の負値より
// 選好される。
#[test]
fn repetition_with_history_is_preferred_as_a_draw_over_losing_lines() {
    let root = repetition_fixture();
    let moves = legal_moves(&root);
    let draw_move = repetition_move();
    assert!(moves.contains(&draw_move));

    // 履歴へ「反車が6七にあり後手番」の局面キーを1件入れる。
    let mut child = root.clone();
    child.make_move_unchecked(draw_move, engine_rules());
    let history = [search_key(&child)];

    let result = search(
        &root,
        engine_rules(),
        &moves,
        &history,
        &depth_limits(2),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    // 反復再現手だけが引き分け0を得る。他手はすべて次手で先手王将が
    // 取られ詰み帯の負値になるため、最善手は一意である。
    assert_eq!(result.best_move, draw_move);
    assert_eq!(result.score, 0);
}

// D7-SRCH-03境界。履歴が空なら反復は検出されず、評価は詰み帯の負値
// （全変化が次手の王駒捕獲でMATE−1、根から見て−(MATE−1)）へ落ちる。
// 履歴が探索入力であることの確認。
#[test]
fn without_history_the_repetition_fixture_scores_a_mate_band_loss() {
    let root = repetition_fixture();
    let moves = legal_moves(&root);

    let result = search(
        &root,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(2),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    assert_eq!(result.score, -(MATE - 1));
}

// D7-SRCH-04。search.md「評価関数v0」（獅子=2500）とMVV-LVA。RULES.md
// 第14条第5項により非獅子は足の有無にかかわらず獅子を取れる。
#[test]
fn free_lion_capture_is_preferred_without_entering_the_mate_band() {
    // 先手: 飛車3十、王将6十二。後手: 獅子3四（只取り）、玉将6一。先手番。
    let position = position(
        Color::Black,
        &[
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(3, 4), Color::White, PieceKind::Lion),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let moves = legal_moves(&position);

    let result = search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    assert_eq!(result.best_move.from, fs(3, 10));
    assert_eq!(result.best_move.to, fs(3, 4));
    // 獅子2500は高価だが王駒ではないため、詰み帯に入らない（INV-3）。
    assert!(result.score > 0);
    assert!(result.score < 29_000);
}

// D7-SRCH-05。search.md「探索内の終局と規則処理」: 獅子の2枚取りは両王駒が
// 消えた時点の判定として自然に扱える。RULES.md第21条第5項、第12条第4項。
#[test]
fn lion_double_capture_of_both_royals_scores_mate() {
    // 先手: 獅子6六、王将6十二。後手: 玉将6五、太子6四（隣接して直列）。
    let position = position(
        Color::Black,
        &[
            (fs(6, 6), Color::Black, PieceKind::Lion),
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 5), Color::White, PieceKind::King),
            (fs(6, 4), Color::White, PieceKind::CrownPrince),
        ],
    );
    let moves = legal_moves(&position);
    // 経由升で玉将、到達升で太子を取る2段階移動だけが両王駒を取る。
    // 6四への直接跳びは経由駒を取らない（第12条第7項）ため対象外。
    let expected = Move {
        from: fs(6, 6),
        mid: Some(fs(6, 5)),
        to: fs(6, 4),
        promote: false,
    };
    assert!(moves.contains(&expected));

    let result = search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    assert_eq!(result.best_move, expected);
    assert_eq!(result.score, MATE);
}

// D7-SRCH-06。search.md「探索内の終局と規則処理」: 合法手が1つもない場合は
// 第23条により−MATE+ply。先手のどの着手でも後手は合法手ゼロとなるため、
// 根の評価はMATE−1。RULES.md第23条第2〜3項（利きの除外ではなく物理的に
// 手が尽きる場合だけが対象）。
#[test]
fn opponent_with_no_legal_moves_scores_mate_minus_one_ply() {
    // 後手: 玉将1十二、歩兵1十一・2十一・2十二（いずれも不成）。
    // 2十二の歩兵は最奥段の移動不能駒（第19条第2項）、残り2枚は前方を
    // 自駒に塞がれ、玉将の3近接升はすべて自駒で、後手に合法手がない。
    // 先手: 王将8五、金将10七（後手駒と相互作用しない遠隔配置）。先手番。
    let position = position(
        Color::Black,
        &[
            (fs(1, 12), Color::White, PieceKind::King),
            (fs(1, 11), Color::White, PieceKind::Pawn),
            (fs(2, 11), Color::White, PieceKind::Pawn),
            (fs(2, 12), Color::White, PieceKind::Pawn),
            (fs(8, 5), Color::Black, PieceKind::King),
            (fs(10, 7), Color::Black, PieceKind::GoldGeneral),
        ],
    );
    let moves = legal_moves(&position);

    let result = search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(2),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    // 全候補が同値のため最善手の一意性は主張しない（SPEC_UNCLEAR-07）。
    assert_eq!(result.score, MATE - 1);
}

// D7-SRCH-07。search.md「bench」節・「検証」節、docs/sprt.mdの完全再現契約:
// depthまたはnodes制限だけの探索は同一入力に対し完全に決定的である
// （INV-2）。経過時間の値を除く全観測を比較する。
#[test]
fn search_with_node_limit_is_deterministic() {
    #[allow(clippy::type_complexity)]
    fn run(
        position: &Position,
        limits: SearchLimits,
        id: u64,
    ) -> (
        Vec<(u32, i32, u64, Vec<Move>)>,
        (Move, i32, u32, u64, Vec<Move>, StopReason),
    ) {
        let handle = start_search(
            snapshot_for(position),
            limits,
            id,
            DEFAULT_THREADS,
            small_tt(),
        );
        let (progress, finished) = drain_events(&handle);
        handle.join().expect("search thread must not panic");
        (
            progress
                .into_iter()
                .map(|(depth, score, nodes, _, pv)| (depth, score, nodes, pv))
                .collect(),
            (
                finished.best_move,
                finished.score,
                finished.depth,
                finished.nodes,
                finished.pv,
                finished.stop_reason,
            ),
        )
    }

    // 初期局面（RULES.md第5条）と中盤フィクスチャの双方で確認する。
    let initial = Position::initial();
    let first = run(&initial, nodes_limits(100_000), 71);
    let second = run(&initial, nodes_limits(100_000), 72);
    assert_eq!(first, second);

    let midgame = quiet_midgame();
    let first = run(&midgame, nodes_limits(30_000), 73);
    let second = run(&midgame, nodes_limits(30_000), 74);
    assert_eq!(first, second);
}

// D7-SRCH-08。search.md「静止探索」: 深さ0ではstand-patと捕獲手を探索する。
// 守られた歩兵を飛車で取る損な交換は静止探索で見抜かれ、最善手にならない。
#[test]
fn quiescence_avoids_a_losing_rook_for_pawn_capture() {
    // 先手: 飛車3十、王将6十二。後手: 歩兵3四、飛車3一、玉将6一。
    // 3筋の間は空で、後手飛車は歩兵越しに先手飛車から攻撃されない。
    let position = position(
        Color::Black,
        &[
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(3, 4), Color::White, PieceKind::Pawn),
            (fs(3, 1), Color::White, PieceKind::Rook),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    let moves = legal_moves(&position);

    let result = search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    assert!(
        result.best_move.from != fs(3, 10) || result.best_move.to != fs(3, 4),
        "守られた歩兵を飛車で取る手は最善手にならない"
    );
    assert!(result.score.abs() < 29_000);
}

// D7-SRCH-09。search.md「静止探索」: 捕獲手のない葉ではstand-patを返すため、
// depth=1の根評価は各合法手の子局面に対する静的評価の最大値と一致する。
#[test]
fn quiescence_without_captures_matches_static_evaluation() {
    let mut position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::GoldGeneral),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(6, 3), Color::White, PieceKind::GoldGeneral),
        ],
    );
    let moves = legal_moves(&position);
    let expected = moves
        .iter()
        .map(|&mv| {
            let undo = position.make_move_unchecked(mv, engine_rules());
            let score = -crate::eval::evaluate(&position);
            position.unmake_move(undo);
            score
        })
        .max()
        .expect("root must have a legal move");

    let result = search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    assert_eq!(result.score, expected);
}

// D7-SRCH-10。search.md「静止探索」: stand-patは損な捕獲より優先される
// fail-softの下限であり、捕獲手が存在しても選択を強制されない。
#[test]
fn quiescence_stand_pat_declines_a_losing_capture() {
    let mut position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(9, 9), Color::Black, PieceKind::Pawn),
            (fs(9, 12), Color::Black, PieceKind::Rook),
            (fs(6, 1), Color::White, PieceKind::King),
            (fs(9, 3), Color::White, PieceKind::Rook),
        ],
    );
    let moves = legal_moves(&position);
    let quiet_move = Move {
        from: fs(6, 12),
        mid: None,
        to: fs(6, 11),
        promote: false,
    };
    assert!(moves.contains(&quiet_move));

    let quiet_undo = position.make_move_unchecked(quiet_move, engine_rules());
    let losing_capture = Move {
        from: fs(9, 3),
        mid: None,
        to: fs(9, 9),
        promote: false,
    };
    assert!(legal_moves(&position).contains(&losing_capture));
    position.unmake_move(quiet_undo);

    let expected = moves
        .iter()
        .map(|&mv| {
            let undo = position.make_move_unchecked(mv, engine_rules());
            let score = -crate::eval::evaluate(&position);
            position.unmake_move(undo);
            score
        })
        .max()
        .expect("root must have a legal move");

    let result = search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    assert_eq!(result.score, expected);
}

// ---------------------------------------------------------------------------
// D7-LIM　SearchLimits
// ---------------------------------------------------------------------------

// D7-LIM-01。search.md「時間管理」節: 深さ制約は1以上、最大探索ply（256）
// 以下だけを受理し、範囲外は`SearchLimits`の検証で拒否する。
#[test]
fn depth_limit_accepts_the_1_to_256_range_only() {
    assert!(depth_limits(0).validate().is_err());
    assert!(depth_limits(1).validate().is_ok());
    assert!(depth_limits(256).validate().is_ok());
    assert!(depth_limits(257).validate().is_err());
}

// D7-LIM-02[実装契約]。search.md「時間管理」節の制約列挙からの導出:
// 無期限が明示の制約値として存在するため、全制約の欠落は指定漏れとして
// 拒否する。無期限の明示（D7-LIM-05）とは峻別される。
#[test]
fn limits_without_any_constraint_are_rejected_but_explicit_infinite_is_accepted() {
    assert!(no_limits().validate().is_err());
    assert!(infinite_limits().validate().is_ok());
}

// D7-LIM-03。search.md「時間管理」節（最も早く満たされた条件で停止）と
// 「スレッド構成」節（停止理由の語彙にノード上限）。超過幅の上限は規範に
// 明文がなく（SPEC_UNCLEAR-05）、ノード数は下限だけをassertする。
#[test]
fn node_limit_stops_the_search_with_a_legal_best_move() {
    let initial = Position::initial();
    let snapshot = snapshot_for(&initial);
    let root_moves = snapshot.root_moves.clone();

    let handle = start_search(
        snapshot,
        nodes_limits(5_000),
        31,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (_, finished) = drain_events(&handle);
    handle.join().expect("search thread must not panic");

    assert_eq!(finished.stop_reason, StopReason::NodeLimit);
    assert!(finished.nodes >= 5_000);
    assert!(root_moves.contains(&finished.best_move)); // INV-1

    // 境界: nodes=1の極小値でも合法な最善手が返る（D7-LIM-04と同根）。
    let moves = legal_moves(&initial);
    let result = search(
        &initial,
        engine_rules(),
        &moves,
        &[],
        &nodes_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );
    assert!(moves.contains(&result.best_move));
}

// D7-LIM-04。search.md「時間管理」節・「スレッド構成」節: 探索開始前に
// ルート合法手から1手を確保し、深さ1の完了前に停止しても常に合法な
// `bestmove`を返す。「確保した1手が先頭」は規範節にないため、集合帰属
// だけを検証する。
#[test]
fn stop_or_tiny_budget_before_depth_one_still_yields_a_legal_best_move() {
    let initial = Position::initial();

    // 最速ケース: 開始直後に停止フラグを立てる。無期限指定のため停止理由は
    // 外部停止要求しかあり得ない。
    let snapshot = snapshot_for(&initial);
    let root_moves = snapshot.root_moves.clone();
    let handle = start_search(snapshot, infinite_limits(), 41, DEFAULT_THREADS, small_tt());
    handle.request_stop();
    let (_, finished) = drain_events(&handle);
    handle.join().expect("search thread must not panic");
    assert_eq!(finished.stop_reason, StopReason::ExternalStop);
    assert!(root_moves.contains(&finished.best_move)); // INV-1

    // 境界: 極小のhard予算（movetime=1ms、D7-TIME-02境界の受理側）でも
    // 同じ保証が成り立つ。soft=hardのため停止理由は2値を許容する
    // （SPEC_UNCLEAR-04）。
    let handle = start_search(
        snapshot_for(&initial),
        movetime_limits(1),
        42,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (_, finished) = drain_events(&handle);
    handle.join().expect("search thread must not panic");
    assert!(matches!(
        finished.stop_reason,
        StopReason::SoftLimit | StopReason::HardLimit
    ));
    assert!(legal_moves(&initial).contains(&finished.best_move));
}

// D7-LIM-05。search.md「時間管理」節: 無期限ではsoft/hardの両リミットを
// 無効化し、外部停止要求だけで停止する。「届かないこと」の待ち時間は
// テスト定数であり規範値ではない。D7-API-03(5)の外部停止要求も兼ねる。
#[test]
fn infinite_limits_stop_only_on_external_request() {
    let initial = Position::initial();
    let snapshot = snapshot_for(&initial);
    let root_moves = snapshot.root_moves.clone();
    let handle = start_search(snapshot, infinite_limits(), 51, DEFAULT_THREADS, small_tt());

    // 数百ms待つ間、Finishedは届かずProgressが届き続ける。
    let started = Instant::now();
    let mut progress_seen = 0;
    while started.elapsed() < Duration::from_millis(300) {
        match handle.events().recv_timeout(Duration::from_millis(50)) {
            Ok(SearchEvent::Progress { search_id, .. }) => {
                assert_eq!(search_id, 51);
                progress_seen += 1;
            }
            Ok(SearchEvent::Finished { .. }) => {
                panic!("infinite search must not finish without a stop request");
            }
            Err(_) => {}
        }
    }
    assert!(progress_seen >= 1);

    handle.request_stop();
    let (_, finished) = drain_events(&handle);
    handle.join().expect("search thread must not panic");
    assert_eq!(finished.stop_reason, StopReason::ExternalStop);
    assert!(root_moves.contains(&finished.best_move)); // INV-1
}

// ---------------------------------------------------------------------------
// D7-TIME　時間予算
// ---------------------------------------------------------------------------

// D7-TIME-01。search.md「時間管理」節と「実施状況」2026年8月12日確定の
// 予算式: soft＝残り/50＋加算×0.7＋秒読み×0.8、hard＝min(soft×4,
// 残り/4＋秒読み×0.8)、hardは1ms以上かつ「残り＋秒読み−30ms」以下。
// 数値例はすべて端数の出ない入力から式で導出した固定値である
// （丸め規約はSPEC_UNCLEAR-02）。
#[test]
fn clock_budget_matches_the_normative_formula() {
    // (a) 残り60000・加算1000・秒読み0:
    //     soft = 60000/50 + 1000×0.7 = 1900、hard = min(7600, 15000) = 7600。
    let budget = clock_budget(clock(60_000, 1_000, 0));
    assert_eq!(budget.soft, Duration::from_millis(1_900));
    assert_eq!(budget.hard, Duration::from_millis(7_600));

    // (b) 残り10000・加算0・秒読み200:
    //     soft = 200 + 160 = 360、hard = min(1440, 2500+160) = 1440。
    let budget = clock_budget(clock(10_000, 0, 200));
    assert_eq!(budget.soft, Duration::from_millis(360));
    assert_eq!(budget.hard, Duration::from_millis(1_440));

    // (c) 残り0・加算0・秒読み100: soft = 80、式のhard = min(320, 80) = 80
    //     だが安全上限 0+100−30 = 70 が効いて hard = 70。softへの上限適用は
    //     規定されないため80のままassertする。
    let budget = clock_budget(clock(0, 0, 100));
    assert_eq!(budget.soft, Duration::from_millis(80));
    assert_eq!(budget.hard, Duration::from_millis(70));

    // 境界: 残り20msでは下限1msと安全上限が両立しない。優先順位は明文が
    // なく（SPEC_UNCLEAR-03）、hard ≥ 1msだけを実装契約としてassertする。
    let budget = clock_budget(clock(20, 0, 0));
    assert!(budget.hard >= Duration::from_millis(1));
}

// D7-TIME-02。search.md「時間管理」節: 時計なしの固定時間単独では、併用則
// （softとhardのそれぞれで小さい方）の帰結として両リミットとも指定値になる。
#[test]
fn movetime_alone_sets_both_soft_and_hard_to_the_given_value() {
    let budget = time_budget(&movetime_limits(500)).expect("movetime must produce a budget");
    assert_eq!(budget.soft, Duration::from_millis(500));
    assert_eq!(budget.hard, Duration::from_millis(500));
}

// D7-TIME-03。search.md「時間管理」節: 固定時間と時計の併用時の予算は、
// softとhardのそれぞれで両者の小さい方を採る（一括minではない独立比較）。
#[test]
fn movetime_and_clock_combine_per_limit_by_taking_the_smaller() {
    // 時計単独ならsoft=1900、hard=7600（D7-TIME-01(a)）。
    let base = clock(60_000, 1_000, 0);
    let with_movetime = |milliseconds: u64| SearchLimits {
        clock: Some(base),
        ..movetime_limits(milliseconds)
    };

    // (a) 交差例: softは時計側、hardはmovetime側が勝つ。独立比較の固定。
    let budget = time_budget(&with_movetime(5_000)).unwrap();
    assert_eq!(budget.soft, Duration::from_millis(1_900));
    assert_eq!(budget.hard, Duration::from_millis(5_000));

    // (b) movetimeが両方で勝つ。
    let budget = time_budget(&with_movetime(500)).unwrap();
    assert_eq!(budget.soft, Duration::from_millis(500));
    assert_eq!(budget.hard, Duration::from_millis(500));

    // (c) softは時計側、hardはmovetime側。
    let budget = time_budget(&with_movetime(2_000)).unwrap();
    assert_eq!(budget.soft, Duration::from_millis(1_900));
    assert_eq!(budget.hard, Duration::from_millis(2_000));
}

// D7-TIME-04。search.md「時間管理」節: softはイテレーション境界、hardは
// ノード周期の時計チェックで適用される。許容幅はテスト環境定数であり
// 規範値ではない。フィクスチャは詰みのない初期局面（SPEC_UNCLEAR-08）。
#[test]
fn movetime_search_stops_near_the_fixed_budget() {
    let initial = Position::initial();
    let handle = start_search(
        snapshot_for(&initial),
        movetime_limits(300),
        61,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (_, finished) = drain_events(&handle);
    handle.join().expect("search thread must not panic");

    assert!(finished.elapsed >= Duration::from_millis(300));
    // 上限ガード: ノード周期チェックの遅延を見込んだ十分な許容幅。
    assert!(finished.elapsed <= Duration::from_millis(1_000));
    // soft=hardのため停止理由は機構上の先着が不定（SPEC_UNCLEAR-04）。
    assert!(matches!(
        finished.stop_reason,
        StopReason::SoftLimit | StopReason::HardLimit
    ));
    assert!(legal_moves(&initial).contains(&finished.best_move)); // INV-1
}

// ---------------------------------------------------------------------------
// D7-API　探索呼び出し境界
// ---------------------------------------------------------------------------

// D7-API-01。search.md「探索骨格」節（深さ1から1ずつ深める）・「スレッド
// 構成」節・「検証」節（単調な深さ）。ノード数の単調非減少は累積からの
// 導出であり実装契約としてassertする。
#[test]
fn progress_depths_start_at_one_and_increase_by_one() {
    let midgame = quiet_midgame();
    let handle = start_search(
        snapshot_for(&midgame),
        depth_limits(4),
        81,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (progress, finished) = drain_events(&handle);
    handle.join().expect("search thread must not panic");

    let depths: Vec<u32> = progress.iter().map(|entry| entry.0).collect();
    assert_eq!(depths, vec![1, 2, 3, 4]);
    for (_, _, _, _, pv) in &progress {
        assert!(!pv.is_empty());
    }
    let nodes: Vec<u64> = progress.iter().map(|entry| entry.2).collect();
    assert!(nodes.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(finished.depth, 4);

    // 境界: depth=1では深さ列は[1]のみ。
    let handle = start_search(
        snapshot_for(&midgame),
        depth_limits(1),
        82,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (progress, _) = drain_events(&handle);
    handle.join().expect("search thread must not panic");
    let depths: Vec<u32> = progress.iter().map(|entry| entry.0).collect();
    assert_eq!(depths, vec![1]);
}

// D7-API-02。search.md「検証」節: `Progress`は反復深化の完了ごとに単調な
// 経過時間を返す。同一ミリ秒があり得るため厳密増加は要求しない。
#[test]
fn progress_and_finished_elapsed_times_are_monotonic() {
    let handle = start_search(
        snapshot_for(&quiet_midgame()),
        depth_limits(4),
        83,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (progress, finished) = drain_events(&handle);
    handle.join().expect("search thread must not panic");

    let mut elapsed: Vec<Duration> = progress.iter().map(|entry| entry.3).collect();
    elapsed.push(finished.elapsed);
    assert!(elapsed.windows(2).all(|pair| pair[0] <= pair[1]));
}

// D7-API-03(1)。search.md「スレッド構成」節: 停止理由「指定深さの完了」と
// 報告深さ。PVの先頭手が最善手と一致し、PVの各手が変化を順に進めた局面で
// 合法であることは「主変化」の意味からの導出（実装契約）。
#[test]
fn depth_limited_search_reports_depth_completed_with_a_consistent_pv() {
    let midgame = quiet_midgame();
    let handle = start_search(
        snapshot_for(&midgame),
        depth_limits(2),
        84,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (_, finished) = drain_events(&handle);
    handle.join().expect("search thread must not panic");

    assert_eq!(finished.stop_reason, StopReason::DepthCompleted);
    assert_eq!(finished.depth, 2);
    assert!(!finished.pv.is_empty());
    assert_eq!(finished.pv[0], finished.best_move);
    assert_pv_is_legal(&midgame, &finished.pv);
    assert!(legal_moves(&midgame).contains(&finished.best_move)); // INV-1
}

// D7-API-03(3)(4)。search.md「スレッド構成」節の停止理由のうちsoftリミットと
// hardリミット。イテレーション境界の先着に依存するため、いずれも2値で
// assertする（SPEC_UNCLEAR-04・09。複数制限の同着は扱わない）。
#[test]
fn clock_driven_searches_stop_with_a_time_limit_reason() {
    let initial = Position::initial();

    // (3) soft≪hardの時計設定（残り6000ms・加算100ms → soft=190ms、
    //     hard=760ms）。原則はsoftリミットで停止する。
    let limits = SearchLimits {
        clock: Some(clock(6_000, 100, 0)),
        ..no_limits()
    };
    let handle = start_search(
        snapshot_for(&initial),
        limits,
        85,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (_, finished) = drain_events(&handle);
    handle.join().expect("search thread must not panic");
    assert!(matches!(
        finished.stop_reason,
        StopReason::SoftLimit | StopReason::HardLimit
    ));
    assert!(legal_moves(&initial).contains(&finished.best_move)); // INV-1

    // (4) hard(70ms) < soft(80ms)の時計設定（D7-TIME-01(c)）。原則はhard
    //     リミットで停止する。
    let limits = SearchLimits {
        clock: Some(clock(0, 0, 100)),
        ..no_limits()
    };
    let handle = start_search(
        snapshot_for(&initial),
        limits,
        86,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (_, finished) = drain_events(&handle);
    handle.join().expect("search thread must not panic");
    assert!(matches!(
        finished.stop_reason,
        StopReason::SoftLimit | StopReason::HardLimit
    ));
    assert!(legal_moves(&initial).contains(&finished.best_move)); // INV-1
}

// D7-API-04。search.md「スレッド構成」節: 2種のイベントは両方が探索IDを
// 持ち、停止済み探索の遅延結果は呼び出し側がIDの不一致で破棄する。
// エンジン側の検証可能な契約は「全イベントが開始時に渡したIDを運ぶこと」。
#[test]
fn events_carry_the_search_id_given_at_start() {
    let initial = Position::initial();

    // ライフサイクル契約: 停止フラグ→join→新しい探索の開始。
    let stale_handle = start_search(
        snapshot_for(&initial),
        depth_limits(1),
        101,
        DEFAULT_THREADS,
        small_tt(),
    );
    stale_handle.request_stop();
    let stale_events = drain_raw(&stale_handle);
    stale_handle.join().expect("search thread must not panic");

    let current_handle = start_search(
        snapshot_for(&initial),
        depth_limits(1),
        202,
        DEFAULT_THREADS,
        small_tt(),
    );
    let current_events = drain_raw(&current_handle);
    current_handle.join().expect("search thread must not panic");

    for event in &stale_events {
        assert_eq!(event.search_id(), 101);
    }
    for event in &current_events {
        assert_eq!(event.search_id(), 202);
    }

    // 遅延して読まれた探索1のイベントは、IDの不一致で識別・破棄できる。
    let mixed: Vec<SearchEvent> = stale_events
        .into_iter()
        .chain(current_events.iter().cloned())
        .collect();
    let accepted: Vec<&SearchEvent> = mixed
        .iter()
        .filter(|event| event.search_id() == 202)
        .collect();
    assert_eq!(accepted.len(), current_events.len());
}

// D7-API-05。search.md「実施状況」2026年8月12日（`join()`による置換表の
// 返却）とライフサイクル契約。ノード数の非厳密な`≤`は実装契約。
#[test]
fn join_returns_the_transposition_table_for_reuse() {
    let midgame = quiet_midgame();
    let snapshot = snapshot_for(&midgame);

    let handle = start_search(
        snapshot.clone(),
        depth_limits(3),
        91,
        DEFAULT_THREADS,
        small_tt(),
    );
    let (_, first) = drain_events(&handle);
    let mut returned_tt = handle.join().expect("search thread must not panic");

    // ルート探索ごとに世代が進む(search.md「置換表」節)。世代が進まないと、
    // 前回探索の同深度エントリが異キーの新規格納を探索をまたいで阻止し続ける。
    // 変異検証(フェーズ4)で検出した配線無検証の補強。
    assert_eq!(returned_tt.generation(), 1);

    // 返却された置換表を渡した探索2は正常に完了し、最善手と最終評価が
    // 探索1と一致する。記録手とカットオフによりノード数は増えない。
    let second = search(
        &snapshot.position,
        snapshot.rules,
        &snapshot.root_moves,
        &snapshot.history_keys,
        &depth_limits(3),
        DEFAULT_THREADS,
        &mut returned_tt,
    );
    assert_eq!(second.best_move, first.best_move);
    assert_eq!(second.score, first.score);
    assert!(second.nodes <= first.nodes);
    assert_eq!(returned_tt.generation(), 2);
}

// ---------------------------------------------------------------------------
// D7-SMP　探索チーム
// ---------------------------------------------------------------------------

// D7-SMP-01・07。lazy-smp.md「探索チーム」「停止と探索予算」: 2・4
// ワーカーは深さ、ノード、時間、無期限＋外部停止の全経路で終了し、
// Finishedを1回だけ送り、全join後に世代が1回だけ進んだ置換表を返す。
#[test]
fn multi_worker_teams_finish_once_and_return_the_shared_table() {
    let initial = Position::initial();
    let root_moves = legal_moves(&initial);
    let cases = [
        (depth_limits(1), false),
        (nodes_limits(1_000), false),
        (movetime_limits(10), false),
        (infinite_limits(), true),
    ];
    let mut search_id = 300;

    for threads in [worker_count(2), worker_count(4)] {
        for (limits, request_stop) in cases {
            let handle = start_search(
                snapshot_for(&initial),
                limits,
                search_id,
                threads,
                small_tt(),
            );
            if request_stop {
                handle.request_stop();
            }
            let events = drain_raw(&handle);
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(event, SearchEvent::Finished { .. }))
                    .count(),
                1
            );
            let SearchEvent::Finished { best_move, .. } = events.last().unwrap() else {
                unreachable!("drain_raw ends with Finished");
            };
            assert!(root_moves.contains(best_move));
            assert!(matches!(
                handle.events().recv_timeout(Duration::from_secs(1)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ));
            let table = handle.join().expect("search team must not panic");
            assert_eq!(table.generation(), 1);
            search_id += 1;
        }
    }
}

// D7-SMP-02。lazy-smp.md「停止と探索予算」: ノード予約は探索チーム全体の
// AtomicU64に対して行い、Nを超える予約を拒否する。
#[test]
fn four_worker_node_limit_never_exceeds_the_team_budget() {
    let initial = Position::initial();
    let root_moves = legal_moves(&initial);
    let limit = 500;
    let handle = start_search(
        snapshot_for(&initial),
        nodes_limits(limit),
        320,
        worker_count(4),
        small_tt(),
    );
    let (_, finished) = drain_events(&handle);
    handle.join().expect("search team must not panic");

    assert_eq!(finished.stop_reason, StopReason::NodeLimit);
    assert!(finished.nodes <= limit);
    assert!(root_moves.contains(&finished.best_move));
}

// D7-SMP-03。lazy-smp.md「探索チーム」: Progressは主ワーカーだけが深さ1
// から1ずつ送り、チーム総ノード数と経過時間は単調非減少となる。
#[test]
fn four_worker_progress_is_main_worker_only_and_monotonic() {
    let handle = start_search(
        snapshot_for(&quiet_midgame()),
        depth_limits(4),
        321,
        worker_count(4),
        small_tt(),
    );
    let (progress, finished) = drain_events(&handle);
    handle.join().expect("search team must not panic");

    assert_eq!(
        progress.iter().map(|entry| entry.0).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert!(
        progress
            .windows(2)
            .all(|pair| pair[0].2 <= pair[1].2 && pair[0].3 <= pair[1].3)
    );
    assert!(progress.iter().all(|entry| !entry.4.is_empty()));
    assert!(progress.last().unwrap().2 <= finished.nodes);
}

// D7-SMP-04。lazy-smp.md「再現性」: Threads=1の固定ノード探索は、経過
// 時間を除く結果、PV、Progress列、ノード数、停止理由が完全に一致する。
// D7-SRCH-07のsearch_with_node_limit_is_deterministicが同じ観測を初期局面と
// 中盤局面の双方で固定する。

// D7-SMP-05。lazy-smp.md「停止と探索予算」: ExternalStopはNodeLimitより
// 優先する。開始時点で両方が成立し得る構成を直接チーム経路へ与える。
#[test]
fn external_stop_takes_priority_over_the_node_limit() {
    let initial = Position::initial();
    let snapshot = snapshot_for(&initial);
    let limits = nodes_limits(1);
    let external_stop = AtomicBool::new(true);
    let table = small_tt();
    let outcome = run_search_team(
        &snapshot.position,
        snapshot.rules,
        &snapshot.root_moves,
        &snapshot.history_keys,
        &limits,
        &external_stop,
        worker_count(4),
        &table,
        None,
    );

    assert_eq!(outcome.stop_reason, StopReason::ExternalStop);
    assert!(outcome.result.nodes <= 1);
}

// D7-SMP-06。lazy-smp.md「探索チーム」: 補助ワーカーはルート列を番号だけ
// 回転してから既存の安定ソートを適用する。TT手と捕獲価値の優先度を保ち、
// 同順位の静かな手だけが回転後の相対順になる。
#[test]
fn auxiliary_root_rotation_preserves_move_priorities_and_changes_ties() {
    let root = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 6), Color::Black, PieceKind::Rook),
            (fs(6, 2), Color::White, PieceKind::Lion),
            (fs(2, 6), Color::White, PieceKind::Pawn),
            (fs(12, 1), Color::White, PieceKind::King),
        ],
    );
    let legal = legal_moves(&root);
    let rook_from = fs(6, 6);
    let high_capture = Move {
        from: rook_from,
        mid: None,
        to: fs(6, 2),
        promote: false,
    };
    let tt_capture = Move {
        from: rook_from,
        mid: None,
        to: fs(2, 6),
        promote: false,
    };
    assert!(legal.contains(&high_capture));
    assert!(legal.contains(&tt_capture));
    let quiet: Vec<Move> = legal
        .iter()
        .copied()
        .filter(|&mv| mv.from == rook_from && move_order_key(&root, mv).is_none())
        .take(2)
        .collect();
    assert_eq!(quiet.len(), 2);

    let mut ordered = vec![quiet[0], tt_capture, quiet[1], high_capture];
    rotate_root_moves(&mut ordered, 1);
    assert_eq!(ordered, vec![tt_capture, quiet[1], high_capture, quiet[0]]);

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
    let history = [];
    let table = small_tt();
    let searcher = new_searcher(&root, engine_rules(), &history, &shared, &table);
    searcher.order_moves(&root, &mut ordered, Some(tt_capture), 0);

    assert_eq!(ordered, vec![tt_capture, high_capture, quiet[1], quiet[0]]);
}

// ---------------------------------------------------------------------------
// D7-TT　置換表
// ---------------------------------------------------------------------------

// D7-TT-01。search.md「置換表」節: bit 0–7がfrom、8–15がmid、16–23がto、
// 24がpromote。midなしは0xff、升の符号化は`Square::dense_index()`（0–143）。
// pack/unpackは全合法手の往復一致で検証する。
#[test]
fn packed_moves_round_trip_across_all_move_shapes() {
    // (1) 初期局面: midなしの通常手と跳び手（麒麟・鳳凰・獅子、RULES.md
    //     第9条）を含む。
    // (2) 2段階移動フィクスチャ: 経由升あり、居喰い（to=from・mid占有）、
    //     じっと（to=from。正準表記はmidなし）、2枚取り、dense_index 0と
    //     143の端升、成り選択（香車の敵陣入り）を含む。
    let two_stage = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King), // dense_index 0
            (sq(6, 6), Color::Black, PieceKind::Lion),
            (sq(11, 6), Color::Black, PieceKind::Lance), // (11,11)=dense 143へ到達
            (sq(2, 4), Color::Black, PieceKind::HornedFalcon),
            (sq(9, 2), Color::Black, PieceKind::SoaringEagle),
            (sq(6, 7), Color::White, PieceKind::Pawn), // 獅子の居喰い・2枚取りの1枚目
            (sq(6, 8), Color::White, PieceKind::Rook), // 2枚取りの2枚目
            (sq(2, 5), Color::White, PieceKind::Pawn), // 角鷹の居喰い
            (sq(0, 11), Color::White, PieceKind::King),
        ],
    );

    let mut saw_mid_none = false;
    let mut saw_two_stage = false;
    let mut saw_igui = false;
    let mut saw_jitto = false;
    let mut saw_double_capture = false;
    let mut saw_promotion = false;
    let mut saw_from_dense_0 = false;
    let mut saw_to_dense_143 = false;

    for position in [Position::initial(), two_stage] {
        let moves = legal_moves(&position);
        assert!(!moves.is_empty());
        for mv in moves {
            let packed = pack_move(mv);
            // 往復一致（恒等写像）。
            assert_eq!(unpack_move(packed), Some(mv));

            // 各フィールドの帯域。midなしは厳密に0xffで、別の番兵で
            // 代替されないことをビット層で確認する。
            let from_bits = packed & 0xff;
            let mid_bits = (packed >> 8) & 0xff;
            let to_bits = (packed >> 16) & 0xff;
            assert!(from_bits <= 143);
            assert!(to_bits <= 143);
            match mv.mid {
                None => assert_eq!(mid_bits, 0xff),
                Some(mid) => {
                    assert!(mid_bits <= 143);
                    assert_eq!(mid_bits, mid.dense_index() as u32);
                }
            }
            // promoteはbit 24の1ビットに収まり、上位ビットは使われない。
            assert_eq!(packed >> 25, 0);

            saw_mid_none |= mv.mid.is_none();
            saw_two_stage |= mv.mid.is_some();
            saw_promotion |= mv.promote;
            saw_from_dense_0 |= mv.from.dense_index() == 0;
            saw_to_dense_143 |= mv.to.dense_index() == 143;
            // じっとの正準表記はmidなし・to=from、居喰いはmid（敵駒升）
            // あり・to=fromである（獅子指し手の正準化）。
            saw_jitto |= mv.mid.is_none() && mv.to == mv.from;
            if let Some(mid) = mv.mid {
                let mid_occupied = position.piece_at(mid).is_some();
                saw_igui |= mv.to == mv.from && mid_occupied;
                saw_double_capture |=
                    mv.to != mv.from && mid_occupied && position.piece_at(mv.to).is_some();
            }
        }
    }

    // 検証対象の着手種別が実際に含まれていたことの確認。
    assert!(saw_mid_none);
    assert!(saw_two_stage);
    assert!(saw_igui);
    assert!(saw_jitto);
    assert!(saw_double_capture);
    assert!(saw_promotion);
    assert!(saw_from_dense_0);
    assert!(saw_to_dense_143);
}

// D7-TT-02。search.md「置換表」節: 格納時は`score >= MATE−256`なら
// `score + ply`、`score <= −(MATE−256)`なら`score − ply`、取り出し時に
// 逆変換する。分岐境界はMATE−256=29744。
#[test]
fn tt_scores_round_trip_between_root_and_node_relative_forms() {
    let best_move = Move {
        from: sq(0, 0),
        mid: None,
        to: sq(0, 1),
        promote: false,
    };
    let key = 0x0123_4567_0000_0042;
    let table = small_tt();
    table.new_search();

    // 全組合せでstore変換→load逆変換が恒等。29001〜29743はINV-3により
    // 発生しない帯なので入力に含めない。最大格納値30000+256=30256は
    // i16に収まる（store成功と往復一致で確認される）。
    let scores = [
        30_000, 29_999, 29_744, -29_744, -29_800, -30_000, 0, 500, 28_999,
    ];
    let plies = [0, 1, 5, 255, 256];
    for score in scores {
        for ply in plies {
            table.store(key, 8, score, Bound::Exact, best_move, ply);
            assert_eq!(table.probe(key, ply).unwrap().score, score);
        }
    }

    // 境界: 29744（詰み帯下限）は変換され、別plyの取り出しで根相対値が
    // ずれて観測される。28999（通常帯上限直下）は変換されない。
    table.store(key, 8, 29_744, Bound::Exact, best_move, 5);
    assert_eq!(table.probe(key, 4).unwrap().score, 29_745);
    table.store(key, 8, -29_744, Bound::Exact, best_move, 5);
    assert_eq!(table.probe(key, 4).unwrap().score, -29_745);
    table.store(key, 8, 28_999, Bound::Exact, best_move, 5);
    assert_eq!(table.probe(key, 4).unwrap().score, 28_999);
}

// D7-TT-03。search.md「置換表」節: 空きスロットまたは同一キーへは常に
// 書き込む。異なるキーは年齢（wrapping_sub）がより古いエントリを置換し、
// 同年齢なら深さがより浅いエントリだけを置換する。同年齢で深さが等しいか
// 深い既存エントリは保持する。
#[test]
fn tt_replacement_follows_same_key_generation_then_depth() {
    let best_move = Move {
        from: sq(0, 0),
        mid: None,
        to: sq(0, 1),
        promote: false,
    };
    // 1MBの表は2の冪スロットで下位ビットが一致するキー対が同一スロットに
    // 落ちる。上位32bitの照合キーは異なる。
    let key_a = 0x1111_1111_0000_0001;
    let key_b = 0x2222_2222_0000_0001;

    // (1) 空きスロットへは書き込まれる。
    let mut table = small_tt();
    table.new_search();
    table.store(key_a, 8, 100, Bound::Exact, best_move, 0);
    let hit = table.probe(key_a, 0).unwrap();
    assert_eq!(hit.score, 100);
    assert_eq!(hit.depth, 8);

    // (2) 同一キーは、新エントリが浅くても常に上書きされる。深さ優先の
    //     誤実装（浅い結果の格納拒否）と峻別する最重要assert。
    table.store(key_a, 2, 222, Bound::Upper, best_move, 0);
    let hit = table.probe(key_a, 0).unwrap();
    assert_eq!(hit.score, 222);
    assert_eq!(hit.depth, 2);
    assert_eq!(hit.bound, Bound::Upper);

    // (3) 異キーでも既存世代が古ければ深さによらず置換する。
    table.clear();
    table.new_search();
    table.store(key_a, 8, 100, Bound::Exact, best_move, 0);
    table.new_search();
    table.store(key_b, 1, 300, Bound::Exact, best_move, 0);
    assert!(table.probe(key_a, 0).is_none());
    assert_eq!(table.probe(key_b, 0).unwrap().score, 300);

    // (4) 異キー・同世代: 既存depth=3へdepth=5は置換する。
    table.clear();
    table.new_search();
    table.store(key_a, 3, 100, Bound::Exact, best_move, 0);
    table.store(key_b, 5, 400, Bound::Exact, best_move, 0);
    assert!(table.probe(key_a, 0).is_none());
    assert_eq!(table.probe(key_b, 0).unwrap().score, 400);

    // (5) 異キー・同世代・同深さ: 既存を保持する（`<`と`<=`の変異検出）。
    table.clear();
    table.new_search();
    table.store(key_a, 5, 100, Bound::Exact, best_move, 0);
    table.store(key_b, 5, 500, Bound::Exact, best_move, 0);
    assert_eq!(table.probe(key_a, 0).unwrap().score, 100);
    assert!(table.probe(key_b, 0).is_none());

    // (6) 異キー・同世代: 既存depth=7へdepth=5は保持する。
    table.clear();
    table.new_search();
    table.store(key_a, 7, 100, Bound::Exact, best_move, 0);
    table.store(key_b, 5, 600, Bound::Exact, best_move, 0);
    assert_eq!(table.probe(key_a, 0).unwrap().score, 100);
    assert!(table.probe(key_b, 0).is_none());

    // 境界: 世代の周回。既存gen=255・現在gen=0のとき年齢は
    // 0 wrapping_sub 255 = 1で「古い」と判定され置換される。
    let table = small_tt();
    for _ in 0..255 {
        table.new_search();
    }
    table.store(key_a, 8, 100, Bound::Exact, best_move, 0);
    table.new_search();
    table.store(key_b, 1, 700, Bound::Exact, best_move, 0);
    assert!(table.probe(key_a, 0).is_none());
    assert_eq!(table.probe(key_b, 0).unwrap().score, 700);
}

// D7-TT-04。search.md「置換表」節: 反復で終端した評価値は探索経路と対局
// 履歴に依存するため置換表へ保存しない。別経路への引き分けスコアの伝播を
// 防ぐ。親ノードの格納可否は明文がなく、子の不保存だけをassertする。
#[test]
fn repetition_draw_values_are_not_stored_in_the_table() {
    let root = repetition_fixture();
    let moves = legal_moves(&root);
    let draw_move = repetition_move();
    let mut child = root.clone();
    child.make_move_unchecked(draw_move, engine_rules());
    let child_key = search_key(&child);
    let history = [child_key];

    let mut table = small_tt();
    let result = search(
        &root,
        engine_rules(),
        &moves,
        &history,
        &depth_limits(2),
        DEFAULT_THREADS,
        &mut table,
    );

    // 反復終端が実際に起きたことの確認（D7-SRCH-03と同じ裁定）。
    assert_eq!(result.score, 0);
    // 反復で終端した子局面のキーはprobeしてもエントリが存在しない。
    assert!(table.probe(child_key, 0).is_none());
    assert!(table.probe(child_key, 1).is_none());
}

// D7-TT-05。search.md「置換表」節: 規則セットの変更時と新規対局の開始時には
// 置換表をクリアする。wire連動はD6の領域であり、ここではクリア契約だけを
// 検証する。クリア後の探索は空の置換表から正常に再構築される。
#[test]
fn tt_clear_empties_all_entries_and_search_restarts() {
    let best_move = Move {
        from: sq(0, 0),
        mid: None,
        to: sq(0, 1),
        promote: false,
    };
    let keys = [
        0x1111_1111_0000_0001_u64,
        0x2222_2222_0000_0002,
        0x3333_3333_0000_0003,
    ];
    let mut table = small_tt();
    table.new_search();
    for &key in &keys {
        table.store(key, 4, 100, Bound::Exact, best_move, 0);
        assert!(table.probe(key, 0).is_some());
    }

    table.clear();

    // 直前までヒットしていたキーのprobeがすべてミスになる。
    for &key in &keys {
        assert!(table.probe(key, 0).is_none());
    }

    // クリア後の探索が正常に完了する（D7-SRCH-07の空置換表前提と接続）。
    let midgame = quiet_midgame();
    let moves = legal_moves(&midgame);
    let result = search(
        &midgame,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(2),
        DEFAULT_THREADS,
        &mut table,
    );
    assert!(moves.contains(&result.best_move));
}

// D7-TT-06。search.md「置換表」節: サイズはMB単位で外部から設定でき、
// サイズ変更は探索中でないときに限って適用する。探索中の適用は
// `start_search`が置換表の所有権を奪う設計により静的に排除されるため、
// ここではアイドル時のリサイズと以後の探索の正常完了だけを検証する
// （SPEC_UNCLEAR-06。サイズの内部値はassertしない）。
#[test]
fn idle_resize_applies_and_later_searches_complete() {
    let midgame = quiet_midgame();
    let moves = legal_moves(&midgame);
    let mut table = TranspositionTable::new(4);

    let before = search(
        &midgame,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(2),
        DEFAULT_THREADS,
        &mut table,
    );
    assert!(moves.contains(&before.best_move));

    table.resize(1);

    let after = search(
        &midgame,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(2),
        DEFAULT_THREADS,
        &mut table,
    );
    assert!(moves.contains(&after.best_move));
}

// D7-TT-08。lazy-smp.md「共有置換表」節: 1エントリはcriticalとadvisoryの
// 2個のAtomicU64からなる厳密な16バイトで、全フィールドを所定の幅へ
// 詰め込む。予約ビットは常に0とする。
#[test]
fn atomic_tt_entry_is_16_bytes_and_all_fields_round_trip() {
    assert_eq!(entry_size(), 16);

    let best_move = Move {
        from: sq(3, 4),
        mid: Some(sq(4, 5)),
        to: sq(5, 6),
        promote: true,
    };
    let key = 0x89ab_cdef_0000_0042;
    let table = small_tt();
    for _ in 0..7 {
        table.new_search();
    }
    table.store(key, 23, -1_234, Bound::Upper, best_move, 0);

    let hit = table.probe(key, 0).unwrap();
    assert_eq!(hit.best_move, best_move);
    assert_eq!(hit.score, -1_234);
    assert_eq!(hit.depth, 23);
    assert_eq!(hit.bound, Bound::Upper);

    let (critical, advisory) = table.raw_entry(key);
    assert_eq!(
        ((critical & CRITICAL_KEY_MASK) >> CRITICAL_KEY_SHIFT) as u32,
        (key >> 32) as u32
    );
    assert_eq!(
        ((critical & CRITICAL_SCORE_MASK) >> CRITICAL_SCORE_SHIFT) as u16 as i16,
        -1_234
    );
    assert_eq!(
        ((critical & CRITICAL_DEPTH_MASK) >> CRITICAL_DEPTH_SHIFT) as u8,
        23
    );
    assert_eq!(
        ((critical & CRITICAL_BOUND_MASK) >> CRITICAL_BOUND_SHIFT) as u8,
        Bound::Upper as u8
    );
    assert_eq!(critical & CRITICAL_RESERVED_MASK, 0);
    assert_eq!(
        ((advisory & ADVISORY_MOVE_MASK) >> ADVISORY_MOVE_SHIFT) as u32,
        pack_move(best_move)
    );
    assert_eq!(
        ((advisory & ADVISORY_GENERATION_MASK) >> ADVISORY_GENERATION_SHIFT) as u8,
        7
    );
    assert_eq!(advisory & ADVISORY_RESERVED_MASK, 0);
}

// D7-TT-09。lazy-smp.md「共有置換表」節: criticalの予約ビット、バウンド、
// 検証キー、または指し手の復号が不正なら、probeはpanicせず未命中にする。
#[test]
fn malformed_atomic_tt_entries_are_probe_misses() {
    let best_move = Move {
        from: sq(0, 0),
        mid: None,
        to: sq(0, 1),
        promote: false,
    };
    let key = 0x1234_5678_0000_0042;
    let table = small_tt();
    table.new_search();
    table.store(key, 8, 100, Bound::Exact, best_move, 0);
    let (critical, advisory) = table.raw_entry(key);

    table.write_raw(key, critical | CRITICAL_RESERVED_MASK, advisory);
    assert!(table.probe(key, 0).is_none());

    table.write_raw(key, critical & !CRITICAL_BOUND_MASK, advisory);
    assert!(table.probe(key, 0).is_none());

    let mismatched_key = key ^ (1_u64 << 32);
    table.write_raw(key, critical, advisory);
    assert!(table.probe(mismatched_key, 0).is_none());

    let packed = pack_move(best_move);
    let invalid_moves = [
        (packed & !0xff) | 144,
        (packed & !(0xff << 8)) | (144 << 8),
        (packed & !(0xff << 16)) | (144 << 16),
    ];
    for invalid_move in invalid_moves {
        let invalid_advisory = (advisory & !ADVISORY_MOVE_MASK) | u64::from(invalid_move);
        table.write_raw(key, critical, invalid_advisory);
        assert!(table.probe(key, 0).is_none());
    }
}

// D7-TT-10。lazy-smp.md「共有置換表」節: 共有参照から同一スロットへの
// probe/storeを競合させてもpanicせず、取り違え得る指し手は書き込まれた
// 合法な候補のいずれかに限られる。
#[test]
fn concurrent_probe_and_store_only_return_written_moves() {
    let table = std::sync::Arc::new(small_tt());
    table.new_search();
    let keys = [
        0x1234_5678_0000_0042,
        0x1234_5678_0001_0042,
        0x1234_5678_0002_0042,
        0x1234_5678_0003_0042,
    ];
    let candidates = [
        Move {
            from: sq(0, 0),
            mid: None,
            to: sq(0, 1),
            promote: false,
        },
        Move {
            from: sq(1, 0),
            mid: Some(sq(1, 1)),
            to: sq(1, 2),
            promote: false,
        },
        Move {
            from: sq(2, 0),
            mid: None,
            to: sq(2, 1),
            promote: true,
        },
        Move {
            from: sq(3, 0),
            mid: Some(sq(3, 1)),
            to: sq(3, 0),
            promote: false,
        },
    ];

    let threads: Vec<_> = (0..4)
        .map(|thread_index| {
            let table = std::sync::Arc::clone(&table);
            std::thread::spawn(move || {
                for iteration in 0..20_000 {
                    let candidate_index = (thread_index + iteration) % candidates.len();
                    table.store(
                        keys[candidate_index],
                        8,
                        iteration as i32,
                        Bound::Exact,
                        candidates[candidate_index],
                        0,
                    );
                    if let Some(hit) = table.probe(keys[candidate_index], 0) {
                        assert!(candidates.contains(&hit.best_move));
                    }
                }
            })
        })
        .collect();

    for thread in threads {
        thread.join().unwrap();
    }
}
