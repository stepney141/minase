//! 領域D2「獅子特殊規則とローカルルール」の挙動マトリクス
//! (scratchpad/matrices/d2-lion-local-rules.md、挙動ID D2-*)に基づくテスト。
//! 期待値の根拠はRULES.md第3条(9〜14号)・第13〜16条・第29条だけである。
//! 座標はマトリクスの表記(筋1〜12、段a〜l)をmsqで盤座標へ変換して用いる。

use crate::core::board::Square;
use crate::core::movegen::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::Position;
use crate::core::rules::{MoveRules, RuleCode, Rules};
use crate::test_util::{position_from_codes, sq};

// ---------- マトリクス共通の補助 ----------

/// マトリクスの座標(筋1〜12、段a〜l)を盤座標へ変換する。段aが後手側最奥。
fn msq(file: u8, rank: char) -> Square {
    sq(file - 1, 11 - (rank as u8 - b'a'))
}

fn piece(color: Color, kind: PieceKind) -> PieceCode {
    PieceCode::new(color, kind).expect("fixture uses an unpromoted-capable kind")
}

/// 麒麟由来の成獅子(第17条。同一側2枚目以降の獅子はこの形で置く)。
fn promoted_lion(color: Color) -> PieceCode {
    PieceCode::new_promoted(color, PieceKind::Lion).unwrap()
}

/// 先手王将12l・後手玉将1aを加えて局面を構築する(マトリクスの共通前提)。
fn fixture(side_to_move: Color, pieces: &[(Square, PieceCode)]) -> Position {
    let mut all = vec![
        (msq(12, 'l'), piece(Color::Black, PieceKind::King)),
        (msq(1, 'a'), piece(Color::White, PieceKind::King)),
    ];
    all.extend_from_slice(pieces);
    position_from_codes(side_to_move, &all)
}

/// マトリクス前提の基準規則(標準規則＋R1)。
fn base() -> MoveRules {
    Rules::ENGINE_DEFAULT.moves
}

fn rules_of(codes: &[RuleCode]) -> MoveRules {
    Rules::from_codes(codes).unwrap().moves
}

fn mv(from: Square, to: Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: false,
    }
}

fn mv2(from: Square, mid: Square, to: Square) -> Move {
    Move {
        from,
        mid: Some(mid),
        to,
        promote: false,
    }
}

fn mvp(from: Square, to: Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: true,
    }
}

/// 居喰い(第3条14号)の正準形。第1段階で隣接駒を取り元の升へ戻る。
fn igui(from: Square, victim: Square) -> Move {
    Move {
        from,
        mid: Some(victim),
        to: from,
        promote: false,
    }
}

/// じっと(第3条13号)の正準形。Minaseは経由升によらず単一のfrom==to形で表す。
fn jitto(from: Square) -> Move {
    Move {
        from,
        mid: None,
        to: from,
        promote: false,
    }
}

fn generated(rules: MoveRules, position: &Position) -> Vec<Move> {
    let mut moves = Vec::new();
    MoveGenerator::new(rules).generate_moves(position, &mut moves);
    moves
}

fn is_generated(rules: MoveRules, position: &Position, expected: Move) -> bool {
    generated(rules, position).contains(&expected)
}

/// 指定升の駒を取る着手だけを合法手集合から抽出する(符号化に依存しない観測)。
fn captures_of(rules: MoveRules, position: &Position, target: Square) -> Vec<Move> {
    generated(rules, position)
        .into_iter()
        .filter(|&candidate| {
            position
                .captured_squares(candidate)
                .into_iter()
                .flatten()
                .any(|square| square == target)
        })
        .collect()
}

/// 合法手集合に含まれることを確認したうえで着手を適用する。
fn play(rules: MoveRules, position: &mut Position, chosen: Move) {
    assert!(is_generated(rules, position, chosen), "{chosen:?}");
    position.make_move_unchecked(chosen, rules);
}

// ---------- 共有フィクスチャ ----------

/// F1系: 後手獅子6d・先手獅子6f(距離2・非隣接)・先手金将7g(6fの足)。手番後手。
fn f1(extra: &[(Square, PieceCode)]) -> Position {
    let mut pieces = vec![
        (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
        (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
        (msq(7, 'g'), piece(Color::Black, PieceKind::GoldGeneral)),
    ];
    pieces.extend_from_slice(extra);
    fixture(Color::White, &pieces)
}

/// F9系: 後手獅子6c・飛車9a(・銅将5b)／先手獅子9f・飛車6i。手番後手。
fn f9(with_copper: bool) -> Position {
    let mut pieces = vec![
        (msq(6, 'c'), piece(Color::White, PieceKind::Lion)),
        (msq(9, 'a'), piece(Color::White, PieceKind::Rook)),
        (msq(9, 'f'), piece(Color::Black, PieceKind::Lion)),
        (msq(6, 'i'), piece(Color::Black, PieceKind::Rook)),
    ];
    if with_copper {
        pieces.push((msq(5, 'b'), piece(Color::White, PieceKind::CopperGeneral)));
    }
    fixture(Color::White, &pieces)
}

/// F10: 先手獅子6f／後手獅子6e(隣接)・金将5d(6eの足)。手番先手。
fn f10() -> Position {
    fixture(
        Color::Black,
        &[
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(6, 'e'), piece(Color::White, PieceKind::Lion)),
            (msq(5, 'd'), piece(Color::White, PieceKind::GoldGeneral)),
        ],
    )
}

/// F11系の駒組: 先手麒麟6e(・竪行6h)／後手獅子6c・金将5b。
fn f11_pieces(with_vertical_mover: bool) -> Vec<(Square, PieceCode)> {
    let mut pieces = vec![
        (msq(6, 'e'), piece(Color::Black, PieceKind::Kirin)),
        (msq(6, 'c'), piece(Color::White, PieceKind::Lion)),
        (msq(5, 'b'), piece(Color::White, PieceKind::GoldGeneral)),
    ];
    if with_vertical_mover {
        pieces.push((msq(6, 'h'), piece(Color::Black, PieceKind::VerticalMover)));
    }
    pieces
}

/// F11: 手番先手。麒麟は6cへ跳んで獅子を取り、敵陣(段a〜d)で成れる。
fn f11(with_vertical_mover: bool) -> Position {
    fixture(Color::Black, &f11_pieces(with_vertical_mover))
}

/// F11a: F11に先手獅子9f(・銀将9g=9fの足)と後手飛車9aを加える。手番先手。
fn f11a(with_silver: bool) -> Position {
    let mut pieces = f11_pieces(true);
    pieces.push((msq(9, 'f'), piece(Color::Black, PieceKind::Lion)));
    pieces.push((msq(9, 'a'), piece(Color::White, PieceKind::Rook)));
    if with_silver {
        pieces.push((msq(9, 'g'), piece(Color::Black, PieceKind::SilverGeneral)));
    }
    fixture(Color::Black, &pieces)
}

/// F12系: 先手獅子6h／後手獅子6f・経由駒6g(任意)・金将7e(任意=6fの足)。手番先手。
fn f12(mid_piece: Option<PieceKind>, with_gold: bool) -> Position {
    let mut pieces = vec![
        (msq(6, 'h'), piece(Color::Black, PieceKind::Lion)),
        (msq(6, 'f'), piece(Color::White, PieceKind::Lion)),
    ];
    if let Some(kind) = mid_piece {
        pieces.push((msq(6, 'g'), piece(Color::White, kind)));
    }
    if with_gold {
        pieces.push((msq(7, 'e'), piece(Color::White, PieceKind::GoldGeneral)));
    }
    fixture(Color::Black, &pieces)
}

/// F13: 先手獅子6d／後手獅子6f・歩兵6e(6fの唯一の足)。手番先手。
fn f13() -> Position {
    fixture(
        Color::Black,
        &[
            (msq(6, 'd'), piece(Color::Black, PieceKind::Lion)),
            (msq(6, 'f'), piece(Color::White, PieceKind::Lion)),
            (msq(6, 'e'), piece(Color::White, PieceKind::Pawn)),
        ],
    )
}

/// F15: 後手獅子6d・銀将6e・銅将5c(6dの足)・飛車9a／先手獅子6f・成獅子9f。手番後手。
fn f15() -> Position {
    fixture(
        Color::White,
        &[
            (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(6, 'e'), piece(Color::White, PieceKind::SilverGeneral)),
            (msq(5, 'c'), piece(Color::White, PieceKind::CopperGeneral)),
            (msq(9, 'a'), piece(Color::White, PieceKind::Rook)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(9, 'f'), promoted_lion(Color::Black)),
        ],
    )
}

/// F16: 後手獅子6c・銅将5b(6cの足)・飛車9a／先手獅子3f・横行2c・成獅子9f。手番後手。
fn f16() -> Position {
    fixture(
        Color::White,
        &[
            (msq(6, 'c'), piece(Color::White, PieceKind::Lion)),
            (msq(5, 'b'), piece(Color::White, PieceKind::CopperGeneral)),
            (msq(9, 'a'), piece(Color::White, PieceKind::Rook)),
            (msq(3, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(2, 'c'), piece(Color::Black, PieceKind::SideMover)),
            (msq(9, 'f'), promoted_lion(Color::Black)),
        ],
    )
}

/// F17: 後手獅子6c・銅将5b(6cの足)・飛車9a／先手獅子6d(6cに隣接)・成獅子9f。手番後手。
fn f17() -> Position {
    fixture(
        Color::White,
        &[
            (msq(6, 'c'), piece(Color::White, PieceKind::Lion)),
            (msq(5, 'b'), piece(Color::White, PieceKind::CopperGeneral)),
            (msq(9, 'a'), piece(Color::White, PieceKind::Rook)),
            (msq(6, 'd'), piece(Color::Black, PieceKind::Lion)),
            (msq(9, 'f'), promoted_lion(Color::Black)),
        ],
    )
}

/// F19: 後手獅子6c(足なし)・飛車9a／先手獅子6e・横行2c・成獅子9f。手番後手。
fn f19() -> Position {
    fixture(
        Color::White,
        &[
            (msq(6, 'c'), piece(Color::White, PieceKind::Lion)),
            (msq(9, 'a'), piece(Color::White, PieceKind::Rook)),
            (msq(6, 'e'), piece(Color::Black, PieceKind::Lion)),
            (msq(2, 'c'), piece(Color::Black, PieceKind::SideMover)),
            (msq(9, 'f'), promoted_lion(Color::Black)),
        ],
    )
}

// ---------- 第3条 用語のテスト上の観測 ----------

#[test]
fn article_3_11_lance_in_the_mid_square_is_a_valuable_piece() {
    // 第3条11号・第16条1・4項(D2-003-03): 香車は歩兵・仲人以外なので価値ある
    // 駒であり、香車を経由捕獲する付け喰いは足(金7e)があっても成立する。
    // 経由駒を歩兵に替えた不成立(D2-016-03)とのメタモルフィック対。
    let position = f12(Some(PieceKind::Lance), true);

    assert!(is_generated(
        base(),
        &position,
        mv2(msq(6, 'h'), msq(6, 'g'), msq(6, 'f'))
    ));
}

#[test]
fn article_3_14_igui_returns_the_lion_and_leaves_no_recapture_target() {
    // 第3条14号・第12条8項・第14条1項(D2-003-06): 居喰いは隣接獅子の捕獲と
    // して合法であり、着手後は獅子が6fへ戻るため、6eへ利く金5dの取り返しは
    // 対象を失う。停止形(D2-015-04)とは異なる着手として区別される。
    let mut position = f10();
    assert!(is_generated(
        base(),
        &position,
        mv(msq(6, 'f'), msq(6, 'e'))
    ));
    play(base(), &mut position, igui(msq(6, 'f'), msq(6, 'e')));

    assert_eq!(position.piece_at(msq(6, 'e')), None);
    assert_eq!(
        position.piece_at(msq(6, 'f')),
        Some(piece(Color::Black, PieceKind::Lion))
    );
}

// ---------- 第13条 足の判定 ----------

#[test]
fn article_13_1_a_footed_lion_cannot_be_captured_by_a_lion_at_distance_two() {
    // 第13条1項・第14条2項(D2-013-01): 金7gの利きが6fに届くため、後手獅子6d
    // が先手獅子6fを取る着手は経路のいかんによらず含まれない。足なし局面F1a
    // との反転対(D2-003-01)はarticle_14_3のテストが受け持つ。
    let position = f1(&[]);

    assert!(captures_of(base(), &position, msq(6, 'f')).is_empty());
    // 禁止は相手獅子を取る着手だけに掛かる(境界)。
    assert!(is_generated(
        base(),
        &position,
        mv(msq(6, 'd'), msq(6, 'e'))
    ));
}

#[test]
fn article_13_2_a_slider_blocked_by_the_lion_itself_is_a_hidden_foot() {
    // 第13条2項・第3条10号・第14条4項(D2-013-02): 飛車6jの利きは6fの自獅子
    // で遮られているが、獅子が盤上から除かれると通るため裏足となる。
    let hidden_foot = fixture(
        Color::White,
        &[
            (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(6, 'j'), piece(Color::Black, PieceKind::Rook)),
        ],
    );
    assert!(captures_of(base(), &hidden_foot, msq(6, 'f')).is_empty());

    // 飛車を6筋の線外(5j)へ移すと足がなくなり、同じ捕獲が含まれる(境界)。
    let off_line = fixture(
        Color::White,
        &[
            (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(5, 'j'), piece(Color::Black, PieceKind::Rook)),
        ],
    );
    assert!(is_generated(
        base(),
        &off_line,
        mv(msq(6, 'd'), msq(6, 'f'))
    ));
}

#[test]
fn article_13_3_a_piece_that_cannot_reach_the_capture_square_is_not_a_foot() {
    // 第13条3項(D2-013-03): 基準は隣接ではなく「駒本来の動きで捕獲後の升へ
    // 移動できるか」である。歩7fは6fの隣だが前(7e)へしか動けず足でない。
    let f3 = fixture(
        Color::White,
        &[
            (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(7, 'f'), piece(Color::Black, PieceKind::Pawn)),
        ],
    );
    assert!(is_generated(base(), &f3, mv(msq(6, 'd'), msq(6, 'f'))));

    // F3a: 歩6gの前は6fなので足となり、1升の置き換えで合法性が反転する。
    let f3a = fixture(
        Color::White,
        &[
            (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(6, 'g'), piece(Color::Black, PieceKind::Pawn)),
        ],
    );
    assert!(captures_of(base(), &f3a, msq(6, 'f')).is_empty());
}

#[test]
fn article_13_4_the_foot_is_judged_on_the_board_just_after_the_capture() {
    // 第13条4項(D2-013-04): 着手前は飛車6bの利きが捕獲側の起点6dで遮られる
    // が、捕獲直後の仮想盤面では6dが空いて6fへ通るため足がある。着手前の
    // 利きで判定する実装を検出する。
    let f4 = fixture(
        Color::White,
        &[
            (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(6, 'b'), piece(Color::Black, PieceKind::Rook)),
        ],
    );
    assert!(captures_of(base(), &f4, msq(6, 'f')).is_empty());

    // 飛車を線外(5b)へ移すと足がなくなる(境界)。
    let off_line = fixture(
        Color::White,
        &[
            (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(5, 'b'), piece(Color::Black, PieceKind::Rook)),
        ],
    );
    assert!(is_generated(
        base(),
        &off_line,
        mv(msq(6, 'd'), msq(6, 'f'))
    ));
}

#[test]
fn article_13_5_a_foot_that_can_itself_be_captured_still_counts() {
    // 第13条5項(D2-013-05): 足の金7gに後手飛車7aの当たりが掛かっていても
    // 足として扱う。足の駒の安全性は判定に関与しない(F1への単調性)。
    let position = f1(&[(msq(7, 'a'), piece(Color::White, PieceKind::Rook))]);

    assert!(captures_of(base(), &position, msq(6, 'f')).is_empty());
}

#[test]
fn article_13_6_a_king_as_the_only_foot_still_counts() {
    // 第13条6項・第8条3項(D2-013-06): 取り返せるのは王将6gだけで、取り返し
    // 升6fには後手角3cの利きが通っているが、王駒であることだけを理由に足から
    // 除外しない。この局面のみ先手王将は12lでなく6gに置く(F5)。
    let f5 = position_from_codes(
        Color::White,
        &[
            (msq(1, 'a'), piece(Color::White, PieceKind::King)),
            (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(3, 'c'), piece(Color::White, PieceKind::Bishop)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(6, 'g'), piece(Color::Black, PieceKind::King)),
        ],
    );
    assert!(!is_generated(base(), &f5, mv(msq(6, 'd'), msq(6, 'f'))));

    // 王将を6fへ届かない7hへ移すと足がなくなる(境界)。
    let unreachable_king = position_from_codes(
        Color::White,
        &[
            (msq(1, 'a'), piece(Color::White, PieceKind::King)),
            (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(3, 'c'), piece(Color::White, PieceKind::Bishop)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(7, 'h'), piece(Color::Black, PieceKind::King)),
        ],
    );
    assert!(is_generated(
        base(),
        &unreachable_king,
        mv(msq(6, 'd'), msq(6, 'f'))
    ));
}

#[test]
fn article_13_7_the_foot_judgement_is_not_recursive() {
    // 第13条7項・第14条2項(D2-013-07): 3枚獅子局面F6。成獅子6hは獅子の動き
    // で6fへ到達でき足である。金5eが6fへ利くため、取り返しへ第13〜16条を
    // 再帰適用すると足が否定されてしまうが、判定は仮想盤面での到達可能性
    // だけによる。金の有無で結果が変わらないこと(非再帰なら不変)も確認する。
    let with_gold = fixture(
        Color::White,
        &[
            (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(5, 'e'), piece(Color::White, PieceKind::GoldGeneral)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(6, 'h'), promoted_lion(Color::Black)),
        ],
    );
    // 金5e自身は非獅子として6fを取れる(第14条5項)ため、後手獅子の跳びだけを
    // 対象に観測する(空升経由の2段階は跳びへ正準化される)。
    assert!(!is_generated(
        base(),
        &with_gold,
        mv(msq(6, 'd'), msq(6, 'f'))
    ));

    let without_gold = fixture(
        Color::White,
        &[
            (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(6, 'h'), promoted_lion(Color::Black)),
        ],
    );
    assert!(captures_of(base(), &without_gold, msq(6, 'f')).is_empty());
}

// ---------- 第14条 獅子による獅子の捕獲 ----------

#[test]
fn article_14_1_an_adjacent_lion_can_be_captured_unconditionally() {
    // 第14条1項(D2-014-01)・第16条12項(D2-016-10): 隣接する相手獅子は足
    // (金7g)があっても取れ、付け喰いの成立を要しない。停止形と居喰い形の
    // 双方が含まれる。
    let f7 = fixture(
        Color::White,
        &[
            (msq(6, 'e'), piece(Color::White, PieceKind::Lion)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(7, 'g'), piece(Color::Black, PieceKind::GoldGeneral)),
        ],
    );
    let stop = mv(msq(6, 'e'), msq(6, 'f'));
    assert!(is_generated(base(), &f7, stop));
    assert!(is_generated(base(), &f7, igui(msq(6, 'e'), msq(6, 'f'))));

    // 獅子が獅子を取ったので先獅子は成立せず(第15条6項)、金7gで取り返せる。
    let mut position = f7;
    play(base(), &mut position, stop);
    assert!(is_generated(
        base(),
        &position,
        mv(msq(7, 'g'), msq(6, 'f'))
    ));
}

#[test]
fn article_14_3_an_unfooted_lion_at_distance_two_can_be_captured() {
    // 第14条3項(D2-014-03): 非隣接でも足がなければ獅子で取れる。F1(足あり、
    // D2-013-01)との反転対。Minaseの正準符号化では空升経由の2段階移動は
    // 跳び(midなし)へ正準化されるため、捕獲は跳び形で観測する。
    let f1a = fixture(
        Color::White,
        &[
            (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
        ],
    );

    assert!(is_generated(base(), &f1a, mv(msq(6, 'd'), msq(6, 'f'))));
}

#[test]
fn article_14_5_a_non_lion_captures_a_lion_regardless_of_feet() {
    // 第14条5項(D2-014-05): 後手飛車2fは足(金7g)のある先手獅子6fを取れる。
    let mut position = fixture(
        Color::White,
        &[
            (msq(2, 'f'), piece(Color::White, PieceKind::Rook)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(7, 'g'), piece(Color::Black, PieceKind::GoldGeneral)),
        ],
    );
    play(base(), &mut position, mv(msq(2, 'f'), msq(6, 'f')));

    // 後手に獅子は残らないため先獅子の保護対象はなく、直後の取り返しは
    // 通常どおり含まれる(第15条1項は「取った側に残る獅子」を前提とする)。
    assert!(is_generated(
        base(),
        &position,
        mv(msq(7, 'g'), msq(6, 'f'))
    ));
}

#[test]
fn article_14_6_promoted_lions_follow_the_same_capture_rules() {
    // 第14条6項・第17条(D2-014-06): 麒麟由来の成獅子でも第13〜16条の判定は
    // 変わらない。(i)取る側が成獅子、(ii)取られる側が成獅子のいずれもF1と
    // 同じく非隣接・足ありの捕獲は含まれない。
    let promoted_capturer = fixture(
        Color::White,
        &[
            (msq(6, 'd'), promoted_lion(Color::White)),
            (msq(6, 'f'), piece(Color::Black, PieceKind::Lion)),
            (msq(7, 'g'), piece(Color::Black, PieceKind::GoldGeneral)),
        ],
    );
    assert!(captures_of(base(), &promoted_capturer, msq(6, 'f')).is_empty());

    let promoted_target = fixture(
        Color::White,
        &[
            (msq(6, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(6, 'f'), promoted_lion(Color::Black)),
            (msq(7, 'g'), piece(Color::Black, PieceKind::GoldGeneral)),
        ],
    );
    assert!(captures_of(base(), &promoted_target, msq(6, 'f')).is_empty());
}

#[test]
fn articles_14_5_and_15_1_an_eagle_may_capture_two_lions_and_triggers_senjishi() {
    // 第14条5項・第11条2・3項・第7条9項・第15条1項(D2-014-07): 飛鷲は非獅子
    // なので2段階移動の各段階で足条件なく獅子を取れる。着手後は取った側に
    // 残る後手獅子9d(足=銅8c)へ先獅子が成立し、直後の「飛車9j×獅子9d」は
    // 含まれない。
    let mut position = fixture(
        Color::White,
        &[
            (
                msq(6, 'd'),
                PieceCode::new_promoted(Color::White, PieceKind::SoaringEagle).unwrap(),
            ),
            (msq(9, 'd'), piece(Color::White, PieceKind::Lion)),
            (msq(8, 'c'), piece(Color::White, PieceKind::CopperGeneral)),
            (msq(5, 'e'), piece(Color::Black, PieceKind::Lion)),
            (msq(4, 'f'), promoted_lion(Color::Black)),
            (msq(9, 'j'), piece(Color::Black, PieceKind::Rook)),
        ],
    );
    play(
        base(),
        &mut position,
        mv2(msq(6, 'd'), msq(5, 'e'), msq(4, 'f')),
    );

    assert!(!is_generated(
        base(),
        &position,
        mv(msq(9, 'j'), msq(9, 'd'))
    ));
}

// ---------- 第15条 先獅子 ----------

#[test]
fn article_15_2_without_a_foot_the_lion_can_be_recaptured_immediately() {
    // 第15条2項(D2-015-02、岡崎方式): 残る獅子6cに足がなければ先獅子は
    // 成立せず、直後の取り返しが含まれる(非獅子の捕獲は第14条5項で無条件)。
    let mut position = f9(false);
    play(base(), &mut position, mv(msq(9, 'a'), msq(9, 'f')));

    assert!(is_generated(
        base(),
        &position,
        mv(msq(6, 'i'), msq(6, 'c'))
    ));
}

#[test]
fn articles_15_4_and_15_5_senjishi_lasts_only_for_the_very_next_move() {
    // 第15条4・5項・第3条12号(D2-015-03・D2-003-04): 禁止は直後の1手だけに
    // 及び、相手が別の着手を行った時点で消滅する。足(銅5b)は維持されたまま
    // でも、非獅子による捕獲に足は関係ない(第14条5項)。
    let mut position = f9(true);
    play(base(), &mut position, mv(msq(9, 'a'), msq(9, 'f')));
    assert!(!is_generated(
        base(),
        &position,
        mv(msq(6, 'i'), msq(6, 'c'))
    ));

    // 先獅子は無関係な飛車の移動を妨げない。
    assert!(is_generated(
        base(),
        &position,
        mv(msq(6, 'i'), msq(6, 'd'))
    ));

    play(base(), &mut position, mv(msq(12, 'l'), msq(12, 'k')));
    play(base(), &mut position, mv(msq(1, 'a'), msq(1, 'b')));

    assert!(is_generated(
        base(),
        &position,
        mv(msq(6, 'i'), msq(6, 'c'))
    ));
}

#[test]
fn article_15_7_senjishi_can_protect_a_kirin_promoted_lion() {
    // 第15条7項・第15条1項・第18条1項(D2-015-05): 麒麟6e→6c捕獲・成りの
    // 終了時、新しい成獅子6c自身に足(麒麟の去った6筋で開通した竪行6h。
    // 第13条4項)があるため、標準規則では先獅子が成立する。
    let mut promoted_case = f11(true);
    play(base(), &mut promoted_case, mvp(msq(6, 'e'), msq(6, 'c')));
    assert!(!is_generated(
        base(),
        &promoted_case,
        mv(msq(5, 'b'), msq(6, 'c'))
    ));

    // 境界(i): 不成なら盤上の6cは麒麟であり獅子ではないため保護されない。
    let mut unpromoted_case = f11(true);
    play(base(), &mut unpromoted_case, mv(msq(6, 'e'), msq(6, 'c')));
    assert!(is_generated(
        base(),
        &unpromoted_case,
        mv(msq(5, 'b'), msq(6, 'c'))
    ));

    // 境界(ii): 竪行6hがなければ成獅子6cに足がなく先獅子は成立しない。
    let mut footless_case = f11(false);
    play(base(), &mut footless_case, mvp(msq(6, 'e'), msq(6, 'c')));
    assert!(is_generated(
        base(),
        &footless_case,
        mv(msq(5, 'b'), msq(6, 'c'))
    ));
}

#[test]
fn articles_15_5_and_3_13_jitto_is_a_move_that_expires_senjishi() {
    // 第15条5項・第3条13号・第6条4項(D2-015-06・D2-003-05): じっとは手番
    // 放棄ではなく合法な1手であり、「別の着手」として先獅子の禁止を消滅
    // させる。じっとを着手なしと扱い禁止を持続させる実装を検出する。
    let mut position = f16();
    play(base(), &mut position, mv(msq(9, 'a'), msq(9, 'f')));
    assert!(!is_generated(
        base(),
        &position,
        mv(msq(2, 'c'), msq(6, 'c'))
    ));

    play(base(), &mut position, jitto(msq(3, 'f')));
    play(base(), &mut position, mv(msq(1, 'a'), msq(1, 'b')));

    assert!(is_generated(
        base(),
        &position,
        mv(msq(2, 'c'), msq(6, 'c'))
    ));
}

#[test]
fn articles_15_1_and_14_1_senjishi_blocks_even_an_adjacent_lion_recapture() {
    // 第15条解説・第14条1項: 標準規則の先獅子は獅子による取り返しにも及び、
    // 隣接獅子を獅子で取る着手も直後の1手では認めない。
    let mut position = f17();
    play(base(), &mut position, mv(msq(9, 'a'), msq(9, 'f')));
    assert!(!is_generated(
        base(),
        &position,
        mv(msq(6, 'd'), msq(6, 'c'))
    ));
    // 境界として、居喰い形「6d→6c(捕獲)→6d」は合法となる。
    // 第13条3項は取り返す駒が「獅子捕獲後の升」へ移動できることを足の要件と
    // するため、着手ごとの判定では到達升6dに銅5bの利きが届かず足が成立しない。
    // 先獅子の禁止(第15条1項)は足条件付きであり、この着手には及ばない。
    assert!(is_generated(
        base(),
        &position,
        igui(msq(6, 'd'), msq(6, 'c'))
    ));

    // 第3手以降は失効し、隣接捕獲が通常どおり含まれる(境界)。
    play(base(), &mut position, mv(msq(12, 'l'), msq(12, 'k')));
    play(base(), &mut position, mv(msq(1, 'a'), msq(1, 'b')));
    assert!(is_generated(
        base(),
        &position,
        mv(msq(6, 'd'), msq(6, 'c'))
    ));
}

#[test]
fn article_15_1_each_remaining_footed_lion_is_protected_independently() {
    // D2-015-08(解釈固定): 第15条1項の「取った側に残る獅子」が複数ある場合
    // の扱いは明文がない(SPEC_UNCLEAR-2)。足のある残存獅子のそれぞれが独立
    // に保護される解釈を採り、新しい成獅子6cと既存の獅子9fの双方を保護する。
    let mut position = f11a(true);
    play(base(), &mut position, mvp(msq(6, 'e'), msq(6, 'c')));
    assert!(!is_generated(
        base(),
        &position,
        mv(msq(5, 'b'), msq(6, 'c'))
    ));
    assert!(!is_generated(
        base(),
        &position,
        mv(msq(9, 'a'), msq(9, 'f'))
    ));

    // 境界: 銀9gを除くと獅子9fに足がなく、保護は獅子ごとの足条件による。
    let mut without_silver = f11a(false);
    play(base(), &mut without_silver, mvp(msq(6, 'e'), msq(6, 'c')));
    assert!(is_generated(
        base(),
        &without_silver,
        mv(msq(9, 'a'), msq(9, 'f'))
    ));
    assert!(!is_generated(
        base(),
        &without_silver,
        mv(msq(5, 'b'), msq(6, 'c'))
    ));
}

// ---------- 第16条 付け喰い ----------

#[test]
fn articles_16_1_and_16_4_tsukegui_captures_a_footed_lion() {
    // 第16条1・2・4項・第14条2項(D2-016-01): 第1段階で銀6gを取り第2段階で
    // 獅子6fを取る付け喰いは、足(金7e)があっても含まれる。直接跳びは経由升
    // の駒を取らない(第12条7項)ため付け喰いにならず、含まれない。
    let position = f12(Some(PieceKind::SilverGeneral), true);
    let tsukegui = mv2(msq(6, 'h'), msq(6, 'g'), msq(6, 'f'));
    assert!(is_generated(base(), &position, tsukegui));
    assert!(!is_generated(
        base(),
        &position,
        mv(msq(6, 'h'), msq(6, 'f'))
    ));

    // 付け喰い後は足の金7eで取り返せる(第16条5項。獅子が取ったので先獅子は
    // 成立しない。第16条6項、D2-016-01境界)。
    let mut after = position;
    play(base(), &mut after, tsukegui);
    assert!(is_generated(base(), &after, mv(msq(7, 'e'), msq(6, 'f'))));
}

#[test]
fn article_16_3_capturing_a_pawn_in_the_mid_square_is_not_tsukegui() {
    // 第16条3項・第14条2・3項(D2-016-03): 歩兵経由では付け喰いが成立せず、
    // 足(金7e)があるため含まれない。第1段階終了時に両獅子が隣接する形に
    // なっても第14条1項の隣接例外は適用されない(隣接判定は着手開始時)。
    let f12a = f12(Some(PieceKind::Pawn), true);
    assert!(!is_generated(
        base(),
        &f12a,
        mv2(msq(6, 'h'), msq(6, 'g'), msq(6, 'f'))
    ));

    // F12a′: 足がなければ同じ2段階移動は通常の連続捕獲として含まれる
    // (第14条3項。付け喰いの成立は不要)。
    let footless = f12(Some(PieceKind::Pawn), false);
    assert!(is_generated(
        base(),
        &footless,
        mv2(msq(6, 'h'), msq(6, 'g'), msq(6, 'f'))
    ));
    // 跳びでは歩6gが盤上に残るが、歩は6fへ移動できず足ではない(境界)。
    assert!(is_generated(
        base(),
        &footless,
        mv(msq(6, 'h'), msq(6, 'f'))
    ));
}

#[test]
fn articles_16_4_and_16_5_tsukegui_stands_even_if_a_sliding_foot_opens() {
    // 第16条4・5・6項・第13条4項(D2-016-05): 経由捕獲で銀5gが消えると角3i
    // の斜線が6fへ開くが、付け喰いは足があっても成立し、直後に角で取り
    // 返せる(取り返されるリスクは合法性に影響しない)。
    let mut position = fixture(
        Color::Black,
        &[
            (msq(6, 'h'), piece(Color::Black, PieceKind::Lion)),
            (msq(6, 'f'), piece(Color::White, PieceKind::Lion)),
            (msq(5, 'g'), piece(Color::White, PieceKind::SilverGeneral)),
            (msq(3, 'i'), piece(Color::White, PieceKind::Bishop)),
        ],
    );
    play(
        base(),
        &mut position,
        mv2(msq(6, 'h'), msq(5, 'g'), msq(6, 'f')),
    );

    // 獅子が取ったので先獅子は不成立(第16条6項)、角の取り返しが含まれる。
    assert!(is_generated(
        base(),
        &position,
        mv(msq(3, 'i'), msq(6, 'f'))
    ));
}

#[test]
fn articles_16_7_and_15_8_tsukegui_overrides_an_established_senjishi() {
    // 第16条7項・第15条8項(D2-016-06・D2-015-07): 成立済みの先獅子による
    // 捕獲禁止と第14条2項の足あり禁止の双方に、付け喰いが優先する。
    let mut position = f15();
    play(base(), &mut position, mv(msq(9, 'a'), msq(9, 'f')));

    let tsukegui = mv2(msq(6, 'f'), msq(6, 'e'), msq(6, 'd'));
    // 保護された獅子6dを取れる手は付け喰いだけであり、直接跳びは両禁止に
    // 服して含まれない(境界)。
    assert_eq!(captures_of(base(), &position, msq(6, 'd')), vec![tsukegui]);

    play(base(), &mut position, tsukegui);
    // 付け喰いでは先獅子が成立しない(第16条6項)ため、足の銅5cによる
    // 取り返しが含まれる(第16条5項)。
    assert!(is_generated(
        base(),
        &position,
        mv(msq(5, 'c'), msq(6, 'd'))
    ));
}

#[test]
fn articles_16_8_to_16_10_a_go_between_as_the_only_foot_does_not_vanish_mid_move() {
    // 第16条8・9・10項・第12条13項(D2-016-07): 唯一の足である仲人6gを第1
    // 段階で取っても、着手の途中で足が消滅したとは扱わず、着手全体を1手と
    // して足ありと判定する。
    let position = f12(Some(PieceKind::GoBetween), false);
    assert!(!is_generated(
        base(),
        &position,
        mv2(msq(6, 'h'), msq(6, 'g'), msq(6, 'f'))
    ));
    // 跳びでは仲人が盤上に残り6fへ移動できる足であるため、こちらも含まれない。
    assert!(!is_generated(
        base(),
        &position,
        mv(msq(6, 'h'), msq(6, 'f'))
    ));
    // 仲人を取って停止する着手は通常の捕獲として含まれる。禁止は同じ着手の
    // 第2段階で獅子を取ることだけである(第16条9項の境界)。
    assert!(is_generated(
        base(),
        &position,
        mv(msq(6, 'h'), msq(6, 'g'))
    ));
}

#[test]
fn articles_16_8_to_16_10_a_pawn_as_the_only_foot_does_not_vanish_mid_move() {
    // 第16条8・9・10項(D2-016-08): 歩兵への拡張はRULES.mdが敷衍と明記する。
    // 仲人(D2-016-07)と同一の判定になる。
    let position = f13();

    assert!(!is_generated(
        base(),
        &position,
        mv2(msq(6, 'd'), msq(6, 'e'), msq(6, 'f'))
    ));
    assert!(!is_generated(
        base(),
        &position,
        mv(msq(6, 'd'), msq(6, 'f'))
    ));
}

#[test]
fn articles_13_4_and_16_3_a_captured_pawn_is_not_restored_to_block_slider_feet() {
    // 第13条4項・第16条3項・第16条8〜10項の適用限界(D2-016-09): 歩5gは足で
    // なく角3iの斜線を遮っているだけである。喰い進みは付け喰いにならず、
    // 着手完了後の仮想盤面(歩は除かれている)で開通した角が足になるため
    // 含まれない。第16条8項は歩・仲人が足である場合の規定であり、足でない
    // 歩を復元して走りの利きを遮る根拠にはならない。
    let position = fixture(
        Color::Black,
        &[
            (msq(6, 'h'), piece(Color::Black, PieceKind::Lion)),
            (msq(6, 'f'), piece(Color::White, PieceKind::Lion)),
            (msq(5, 'g'), piece(Color::White, PieceKind::Pawn)),
            (msq(3, 'i'), piece(Color::White, PieceKind::Bishop)),
        ],
    );
    assert!(!is_generated(
        base(),
        &position,
        mv2(msq(6, 'h'), msq(5, 'g'), msq(6, 'f'))
    ));
    // 跳びでは歩5gが盤上に残って角の斜線を実際に遮り、他に足がないため
    // 含まれる(第14条3項)。喰い進みと跳びで合法性が分かれる非自明な対。
    assert!(is_generated(
        base(),
        &position,
        mv(msq(6, 'h'), msq(6, 'f'))
    ));
}

#[test]
fn articles_16_1_and_16_11_a_lion_captured_in_the_mid_square_is_a_valuable_piece() {
    // 第16条1・11項・第3条11号・第14条1項(D2-016-11): 第1段階は隣接獅子の
    // 無条件捕獲、第2段階は経由升で価値ある駒(獅子は歩兵・仲人以外)を取った
    // 付け喰いとして、足(金7e)があっても成立する。1手で獅子2枚を取る。
    let mut position = fixture(
        Color::Black,
        &[
            (msq(6, 'h'), piece(Color::Black, PieceKind::Lion)),
            (msq(6, 'g'), piece(Color::White, PieceKind::Lion)),
            (msq(6, 'f'), promoted_lion(Color::White)),
            (msq(7, 'e'), piece(Color::White, PieceKind::GoldGeneral)),
        ],
    );
    play(
        base(),
        &mut position,
        mv2(msq(6, 'h'), msq(6, 'g'), msq(6, 'f')),
    );

    // 直後の「金7e×獅子6f」は含まれる(第16条5項。先獅子は不成立。第15条6項)。
    assert!(is_generated(
        base(),
        &position,
        mv(msq(7, 'e'), msq(6, 'f'))
    ));
}

// ---------- 第29条 獅子に関するローカルルール ----------

#[test]
fn articles_29_l0_and_33_1_explicit_l0_is_identical_to_the_standard_rules() {
    // 第29条L0・第33条1・2項(D2-029-01・D2-033-04): L0は標準規則と同内容の
    // 記録用コードであり、明示採用しても挙動を一切変えない。
    let l0 = rules_of(&[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E0]);
    assert_eq!(l0, base());
}

#[test]
fn article_29_l1_forbids_non_lion_recapture_regardless_of_foot() {
    // 第29条L1(D2-029-02): 足のないF9aでも、L1では非獅子による直後の
    // 取り返しが禁止される。標準規則(D2-015-02)とL1を弁別する最小局面。
    let l1 = rules_of(&[RuleCode::L1, RuleCode::P0, RuleCode::R1, RuleCode::E0]);
    let mut position = f9(false);
    play(l1, &mut position, mv(msq(9, 'a'), msq(9, 'f')));
    assert!(!is_generated(l1, &position, mv(msq(6, 'i'), msq(6, 'c'))));

    // 禁止は直後の1手だけであり、第15条4・5項と同じ時系列で失効する(境界)。
    play(l1, &mut position, mv(msq(12, 'l'), msq(12, 'k')));
    play(l1, &mut position, mv(msq(1, 'a'), msq(1, 'b')));
    assert!(is_generated(l1, &position, mv(msq(6, 'i'), msq(6, 'c'))));
}

#[test]
fn article_29_l1_restricts_only_non_lion_pieces() {
    // 第29条L1・第14条3項(D2-029-03): 同一局面・同一手番で、L1は非獅子
    // (横行)の取り返しだけを禁じ、獅子による捕獲は第14条だけに従う
    // (非隣接だが足なしなので合法)。
    let l1 = rules_of(&[RuleCode::L1, RuleCode::P0, RuleCode::R1, RuleCode::E0]);
    let mut restricted = f19();
    play(l1, &mut restricted, mv(msq(9, 'a'), msq(9, 'f')));
    assert!(!is_generated(l1, &restricted, mv(msq(2, 'c'), msq(6, 'c'))));
    // 空升経由の2段階は跳びへ正準化されるため、獅子の捕獲は跳び形で観測する。
    assert!(is_generated(l1, &restricted, mv(msq(6, 'e'), msq(6, 'c'))));

    // 標準規則の同一局面では、足なしのため先獅子が成立せず両方含まれる(境界)。
    let mut standard = f19();
    play(base(), &mut standard, mv(msq(9, 'a'), msq(9, 'f')));
    assert!(is_generated(
        base(),
        &standard,
        mv(msq(2, 'c'), msq(6, 'c'))
    ));
    assert!(is_generated(
        base(),
        &standard,
        mv(msq(6, 'e'), msq(6, 'c'))
    ));
}

#[test]
fn article_29_l1_plus_l2_reproduces_the_english_source_rule() {
    // 第29条L1・L2および同条注記(D2-029-05): 英語文献の原文規則はL1とL2の
    // 併用で再現される。L1単独では麒麟(捕獲時点で非獅子)による捕獲として
    // 足の有無によらず取り返しが禁止され、L1＋L2では例外が働く。
    let l1 = rules_of(&[RuleCode::L1, RuleCode::P0, RuleCode::R1, RuleCode::E0]);
    let mut l1_footed = f11(true);
    play(l1, &mut l1_footed, mvp(msq(6, 'e'), msq(6, 'c')));
    assert!(!is_generated(l1, &l1_footed, mv(msq(5, 'b'), msq(6, 'c'))));

    // 竪行6h(足)の有無にもよらない。
    let mut l1_footless = f11(false);
    play(l1, &mut l1_footless, mvp(msq(6, 'e'), msq(6, 'c')));
    assert!(!is_generated(
        l1,
        &l1_footless,
        mv(msq(5, 'b'), msq(6, 'c'))
    ));

    let l1_l2 = rules_of(&[
        RuleCode::L1,
        RuleCode::L2,
        RuleCode::P0,
        RuleCode::R1,
        RuleCode::E0,
    ]);
    let mut exempted = f11(true);
    play(l1_l2, &mut exempted, mvp(msq(6, 'e'), msq(6, 'c')));
    assert!(is_generated(l1_l2, &exempted, mv(msq(5, 'b'), msq(6, 'c'))));
}

#[test]
fn article_29_l2_exempts_only_the_new_promoted_lion() {
    // 第29条L2・第15条1項(D2-029-06): L2の例外は「その新しい獅子」だけに
    // 及び、既存の獅子9fへの先獅子の保護(足=銀9g)は残る。
    let l0_l2 = rules_of(&[
        RuleCode::L0,
        RuleCode::L2,
        RuleCode::P0,
        RuleCode::R1,
        RuleCode::E0,
    ]);
    let mut position = f11a(true);
    play(l0_l2, &mut position, mvp(msq(6, 'e'), msq(6, 'c')));

    assert!(is_generated(l0_l2, &position, mv(msq(5, 'b'), msq(6, 'c'))));
    assert!(!is_generated(
        l0_l2,
        &position,
        mv(msq(9, 'a'), msq(9, 'f'))
    ));
}

#[test]
fn article_29_l3_adopts_stage_wise_foot_judgement() {
    // 第29条L3(D2-029-07): 唯一の足である仲人・歩兵を第1段階で取った直後に
    // 足が消滅したと判定し、喰い進みを認める(第16条8〜10項の不適用)。
    // 跳びでは足の駒が盤上に残るため、L3でも含まれない。
    let l3 = rules_of(&[
        RuleCode::L0,
        RuleCode::L3,
        RuleCode::P0,
        RuleCode::R1,
        RuleCode::E0,
    ]);
    let go_between = f12(Some(PieceKind::GoBetween), false);
    assert!(is_generated(
        l3,
        &go_between,
        mv2(msq(6, 'h'), msq(6, 'g'), msq(6, 'f'))
    ));
    assert!(!is_generated(l3, &go_between, mv(msq(6, 'h'), msq(6, 'f'))));

    let pawn = f13();
    assert!(is_generated(
        l3,
        &pawn,
        mv2(msq(6, 'd'), msq(6, 'e'), msq(6, 'f'))
    ));
    assert!(!is_generated(l3, &pawn, mv(msq(6, 'd'), msq(6, 'f'))));

    // L3の効果は第16条8〜10項の無効化だけに限られる(性質)。F12(銀経由の
    // 付け喰い)とF14b(足でない歩の除去で開く走りの足)の判定は変わらない。
    let silver_mid = f12(Some(PieceKind::SilverGeneral), true);
    assert!(is_generated(
        l3,
        &silver_mid,
        mv2(msq(6, 'h'), msq(6, 'g'), msq(6, 'f'))
    ));
    assert!(!is_generated(l3, &silver_mid, mv(msq(6, 'h'), msq(6, 'f'))));

    let opened_slider = fixture(
        Color::Black,
        &[
            (msq(6, 'h'), piece(Color::Black, PieceKind::Lion)),
            (msq(6, 'f'), piece(Color::White, PieceKind::Lion)),
            (msq(5, 'g'), piece(Color::White, PieceKind::Pawn)),
            (msq(3, 'i'), piece(Color::White, PieceKind::Bishop)),
        ],
    );
    assert!(!is_generated(
        l3,
        &opened_slider,
        mv2(msq(6, 'h'), msq(5, 'g'), msq(6, 'f'))
    ));
    assert!(is_generated(
        l3,
        &opened_slider,
        mv(msq(6, 'h'), msq(6, 'f'))
    ));
}

#[test]
fn article_29_l4_limits_senjishi_to_non_lion_recaptures() {
    // 第29条L4: 非獅子が獅子を取った直後、足のある残存獅子を獅子で
    // 取り返す着手は認めるが、非獅子による取り返しは禁止したままとする。
    let l4 = rules_of(&[
        RuleCode::L0,
        RuleCode::L4,
        RuleCode::P0,
        RuleCode::R1,
        RuleCode::E0,
    ]);

    let mut adjacent_l4 = f17();
    play(l4, &mut adjacent_l4, mv(msq(9, 'a'), msq(9, 'f')));
    assert!(is_generated(l4, &adjacent_l4, mv(msq(6, 'd'), msq(6, 'c'))));

    let mut non_lion = f16();
    play(l4, &mut non_lion, mv(msq(9, 'a'), msq(9, 'f')));
    assert!(!is_generated(l4, &non_lion, mv(msq(2, 'c'), msq(6, 'c'))));
}
