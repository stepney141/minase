//! 詰みと反復の評価値を検査する。

use super::*;

// ---------------------------------------------------------------------------
// D7-SRCH　探索本体
// ---------------------------------------------------------------------------

// D7-SRCH-01。search.md「探索内の終局と規則処理」: 相手の最後の王駒を取る
// 着手はMATE−(ply+1)。根ply=0のためMATE−1=29999。RULES.md第21条第1項。
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

    let result = run_search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    // 捕獲升は敵陣のため成・不成の2通りがあるが、いずれも同じ捕獲で
    // 同値のMATE−1になる。移動元と到達升だけを固定する（SPEC_UNCLEAR-07）。
    assert_eq!(result.best_move.from, fs(6, 10));
    assert_eq!(result.best_move.to, fs(6, 1));
    assert_eq!(result.best_move.mid, None);
    assert_eq!(result.score, MATE - 1);
}

// D7-SRCH-02。search.md「探索内の終局と規則処理」: 王駒を2枚持つ側の
// 1枚目の王駒は駒割上の損得。RULES.md第20条第3〜5項。
#[test]
fn capture_of_the_first_of_two_royals_scores_as_material_gain() {
    // 後手: 玉将6一、太子8三（王駒2枚）。先手: 飛車8十、王将6十二。先手番。
    let position = position_with_promoted_pieces(
        Color::Black,
        &[
            (fs(6, 1), Color::White, PieceKind::King, false),
            (fs(8, 3), Color::White, PieceKind::CrownPrince, true),
            (fs(8, 10), Color::Black, PieceKind::Rook, false),
            (fs(6, 12), Color::Black, PieceKind::King, false),
        ],
    );
    let moves = legal_moves(&position);

    let result = run_search(
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

// D7-SRCH-03境界。履歴が空なら反復は検出されず、評価は詰み帯の負値
// （全変化が次の相手の着手で王駒捕獲となり、根から見て−(MATE−2)）へ落ちる。
// 履歴が探索入力であることの確認。
#[test]
fn without_history_the_repetition_fixture_scores_a_mate_band_loss() {
    let root = repetition_fixture();
    let moves = legal_moves(&root);

    let result = run_search(
        &root,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(2),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    assert_eq!(result.score, -(MATE - 2));
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

    let result = run_search(
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
    let position = position_with_promoted_pieces(
        Color::Black,
        &[
            (fs(6, 6), Color::Black, PieceKind::Lion, false),
            (fs(6, 12), Color::Black, PieceKind::King, false),
            (fs(6, 5), Color::White, PieceKind::King, false),
            (fs(6, 4), Color::White, PieceKind::CrownPrince, true),
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

    let result = run_search(
        &position,
        engine_rules(),
        &moves,
        &[],
        &depth_limits(1),
        DEFAULT_THREADS,
        &mut small_tt(),
    );

    assert_eq!(result.best_move, expected);
    assert_eq!(result.score, MATE - 1);
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

    let result = run_search(
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
