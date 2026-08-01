//! ローカルルールの管理と、成り・獅子の捕獲制限の判定。

use core::fmt;

use crate::core::bitboard::Bitboard;
use crate::core::movegen::piece_control_with_occupancy;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceKind};
use crate::core::position::Position;
use crate::core::square::{BOARD_RANKS, Square};

/// 第10章のローカルルールコード。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RuleCode {
    /// 岡崎式の足条件付き先獅子(第29条)。標準規則と同内容。
    L0,
    /// 足条件なしの先獅子(第29条)。
    L1,
    /// 麒麟成獅子の同一升例外(第29条)。
    L2,
    /// 段階別の足判定(第29条)。
    L3,
    /// Hodges式の成り権回復(第30条)。
    P1,
    /// 旧英語版Wikipedia式の成り(第30条)。
    P2,
    /// 香車の最奥段救済(第30条)。
    P3,
    /// 仲人の最奥段救済(第30条)。
    P4,
    /// 日本中将棋連盟式千日手(第31条)。標準規則の反復規則。
    R0,
    /// Lishogi式の4回反復裁定(第31条)。
    R1,
    /// 既出局面の再現禁止(第31条)。
    R2,
    /// 王駒実捕獲による終局(第32条)。
    E1,
    /// 駒枯れ不採用(第32条)。
    E2,
}

impl RuleCode {
    /// 全ローカルルールコード。
    pub const ALL: [Self; 13] = [
        Self::L0,
        Self::L1,
        Self::L2,
        Self::L3,
        Self::P1,
        Self::P2,
        Self::P3,
        Self::P4,
        Self::R0,
        Self::R1,
        Self::R2,
        Self::E1,
        Self::E2,
    ];

    /// ビット集合表現でこのコードが占めるビットを返す。
    const fn bit(self) -> u16 {
        1 << self as u8
    }
}

/// 採用する反復規則(第25条)。R0・R1・R2は排他で、常にいずれか1つを採用する。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RepetitionRule {
    /// 日本中将棋連盟式千日手。
    R0,
    /// Lishogi式の4回反復裁定。
    R1,
    /// 既出局面の再現禁止。
    R2,
}

/// ルールコード集合の検証エラー。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RulesError {
    /// 併用できないコードの組合せ(第33条第9項)。
    Conflicting { first: RuleCode, second: RuleCode },
    /// 同じコードの重複指定。
    Duplicate(RuleCode),
    /// 未実装のコード。
    Unsupported(RuleCode),
}

impl fmt::Display for RulesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflicting { first, second } => {
                write!(
                    formatter,
                    "conflicting rule codes: {first:?} and {second:?}"
                )
            }
            Self::Duplicate(code) => write!(formatter, "duplicate rule code: {code:?}"),
            Self::Unsupported(code) => write!(formatter, "unsupported rule code: {code:?}"),
        }
    }
}

impl std::error::Error for RulesError {}

/// 対局で採用するローカルルールの集合。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rules {
    /// 採用コードのビット集合。R0は標準規則そのものなので保持せず、常に0ビットとする。
    codes: u16,
}

impl Rules {
    /// 標準規則(ローカルルール指定なし)を返す。
    pub const fn standard() -> Self {
        Self { codes: 0 }
    }

    /// エンジン既定の「標準規則−R0+R1」を返す。
    pub const fn engine_default() -> Self {
        Self {
            codes: RuleCode::R1.bit(),
        }
    }

    /// コード列から集合を作る。重複・矛盾・未実装のコードを検査する。
    pub fn from_codes(codes: &[RuleCode]) -> Result<Self, RulesError> {
        let mut adopted = 0_u16;
        for &code in codes {
            if adopted & code.bit() != 0 {
                return Err(RulesError::Duplicate(code));
            }
            adopted |= code.bit();
        }

        for (first, second) in [
            (RuleCode::L0, RuleCode::L1),
            (RuleCode::P1, RuleCode::P2),
            (RuleCode::R0, RuleCode::R1),
            (RuleCode::R0, RuleCode::R2),
            (RuleCode::R1, RuleCode::R2),
        ] {
            if adopted & first.bit() != 0 && adopted & second.bit() != 0 {
                return Err(RulesError::Conflicting { first, second });
            }
        }

        for &code in codes {
            match code {
                RuleCode::L0
                | RuleCode::L3
                | RuleCode::P3
                | RuleCode::P4
                | RuleCode::R0
                | RuleCode::R1
                | RuleCode::R2
                | RuleCode::E1
                | RuleCode::E2 => {}
                RuleCode::L1 | RuleCode::L2 | RuleCode::P1 | RuleCode::P2 => {
                    return Err(RulesError::Unsupported(code));
                }
            }
        }

        adopted &= !RuleCode::R0.bit();
        Ok(Self { codes: adopted })
    }

    /// 指定コードを採用しているかどうかを返す。
    pub const fn contains(self, code: RuleCode) -> bool {
        self.codes & code.bit() != 0
    }

    /// 採用している反復規則を返す。指定がなければR0とする(第25条第3項)。
    pub const fn repetition_rule(self) -> RepetitionRule {
        if self.contains(RuleCode::R1) {
            RepetitionRule::R1
        } else if self.contains(RuleCode::R2) {
            RepetitionRule::R2
        } else {
            RepetitionRule::R0
        }
    }

    /// 詰みによる終局判定(第21条第2項)を行うかどうかを返す。E1採用時は行わない。
    pub const fn mate_adjudication_enabled(self) -> bool {
        !self.contains(RuleCode::E1)
    }

    /// 駒枯れ(第22条)を適用しないかどうかを返す。E2採用時に適用しない。
    pub const fn piece_exhaustion_disabled(self) -> bool {
        self.contains(RuleCode::E2)
    }

    /// 着手で成りを選択できるかどうかを判定して返す(第18条・第19条・第30条P3・P4)。
    pub(crate) fn promotion_choice(
        self,
        position: &Position,
        mv: &Move,
        moving_kind: PieceKind,
    ) -> PromotionChoice {
        let Some(piece) = position.piece_at(mv.origin()) else {
            return PromotionChoice::NoPromotion;
        };
        let Some(color) = piece.color() else {
            return PromotionChoice::NoPromotion;
        };
        if piece.is_promoted() || !moving_kind.can_promote() {
            return PromotionChoice::NoPromotion;
        }

        let from_in_zone = in_promotion_zone(color, mv.origin());
        let to_in_zone = in_promotion_zone(color, mv.destination());
        let has_capture = position
            .captured_squares(*mv)
            .into_iter()
            .any(|capture| capture.is_some());
        let enters_zone = !from_in_zone && to_in_zone;
        let capture_in_or_from_zone = has_capture && (from_in_zone || to_in_zone);
        let piece_reaches_last_rank_without_capture = !has_capture
            && match color {
                Color::Black => mv.destination().rank() == BOARD_RANKS - 1,
                Color::White => mv.destination().rank() == 0,
            }
            && match moving_kind {
                PieceKind::Pawn => true,
                PieceKind::Lance => self.contains(RuleCode::P3),
                PieceKind::GoBetween => self.contains(RuleCode::P4),
                _ => false,
            };

        if enters_zone || capture_in_or_from_zone || piece_reaches_last_rank_without_capture {
            PromotionChoice::PromotionOptional
        } else {
            PromotionChoice::NoPromotion
        }
    }

    /// 着手が獅子の捕獲制限(第13条〜第16条)をすべて満たすかどうかを返す。
    /// 獅子を取らない着手は常に満たす。
    pub(crate) fn special_move_is_legal(self, position: &Position, mv: Move) -> bool {
        let captured_lions = captured_lions(position, mv);
        if captured_lions.into_iter().all(|lion| lion.is_none()) {
            return true;
        }

        let moving_kind = position
            .piece_at(mv.origin())
            .and_then(|piece| piece.kind());
        if moving_kind == Some(PieceKind::Lion)
            && captured_lions
                .into_iter()
                .flatten()
                .any(|lion| !lion_capture_is_legal(self, position, mv, lion))
        {
            return false;
        }

        let move_is_tsukegui = captured_lions
            .into_iter()
            .flatten()
            .any(|lion| is_tsukegui(position, mv, lion));
        if position.lion_taken_by_non_lion().is_some()
            && !move_is_tsukegui
            && captured_lions
                .into_iter()
                .flatten()
                .any(|lion| lion_has_foot_after_capture(self, position, mv, lion))
        {
            return false;
        }

        true
    }
}

/// 着手に対する成りの選択肢。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PromotionChoice {
    /// この着手では成れない。
    NoPromotion,
    /// 成るか成らないかを選択できる(第18条第5項)。
    PromotionOptional,
}

/// 指定升が指定対局者の敵陣(相手側の最奥4段、第3条)にあるかどうかを返す。
#[inline]
pub const fn in_promotion_zone(color: Color, square: Square) -> bool {
    match color {
        Color::Black => square.rank() >= BOARD_RANKS - 4,
        Color::White => square.rank() < 4,
    }
}

/// 着手で取られる相手獅子の升を返す。
fn captured_lions(position: &Position, mv: Move) -> [Option<Square>; 2] {
    position.captured_squares(mv).map(|capture| {
        capture.filter(|&square| {
            position
                .piece_at(square)
                .is_some_and(|piece| piece.kind() == Some(PieceKind::Lion))
        })
    })
}

/// 着手が付け喰い(第16条)にあたるかどうかを返す。付け喰いとは、獅子が第1段階で
/// 価値ある駒(歩兵・仲人以外)を取り、第2段階で隣接していない相手獅子を取る着手をいう。
fn is_tsukegui(position: &Position, mv: Move, lion_square: Square) -> bool {
    if position
        .piece_at(mv.origin())
        .and_then(|piece| piece.kind())
        != Some(PieceKind::Lion)
    {
        return false;
    }
    let Some(mid) = mv.mid else {
        return false;
    };
    let distance = mv
        .from
        .file()
        .abs_diff(lion_square.file())
        .max(mv.from.rank().abs_diff(lion_square.rank()));
    if mv.to != lion_square || distance != 2 {
        return false;
    }

    let [mid_capture, destination_capture] = position.captured_squares(mv);
    mid_capture == Some(mid)
        && destination_capture == Some(lion_square)
        && position.piece_at(mid).is_some_and(|piece| {
            !matches!(piece.kind(), Some(PieceKind::Pawn | PieceKind::GoBetween))
        })
}

/// 獅子による相手獅子の捕獲が第14条・第16条を満たすかどうかを返す。隣接していれば
/// 無条件に取れる。距離2では、付け喰いが成立するか、取られる獅子に足がない場合に限る。
fn lion_capture_is_legal(rules: Rules, position: &Position, mv: Move, lion_square: Square) -> bool {
    let distance = mv
        .origin()
        .file()
        .abs_diff(lion_square.file())
        .max(mv.origin().rank().abs_diff(lion_square.rank()));

    match distance {
        1 => true,
        2 if is_tsukegui(position, mv, lion_square) => true,
        2 => !lion_has_foot_after_capture(rules, position, mv, lion_square),
        _ => false,
    }
}

/// 着手適用後の占有状態だけを差分で表す仮想盤面。獅子が取られた直後の仮想的な盤面
/// (第13条第4項)での足の判定に使う。
#[derive(Clone, Copy)]
struct VirtualBoard {
    /// 駒がある升の集合。
    occupied: Bitboard,
    /// 着手側の駒の集合。
    own: Bitboard,
    /// 相手側の駒の集合。
    enemy: Bitboard,
    /// 動かしている駒の現在升。
    current: Square,
}

impl VirtualBoard {
    /// 着手を2段階とも適用した後の仮想盤面を作る。
    fn after_move(position: &Position, mv: Move) -> Self {
        let color = position
            .piece_at(mv.origin())
            .and_then(|piece| piece.color())
            .expect("move origin must contain a piece");
        let board = Self {
            occupied: position.occupied(),
            own: position.pieces_of(color),
            enemy: position.pieces_of(color.opposite()),
            current: mv.origin(),
        };

        if let Some(mid) = mv.mid {
            board.move_to(mid).move_to(mv.to)
        } else {
            board.move_to(mv.to)
        }
    }

    /// 駒を1段階動かした後の状態を返す。到達升にある相手駒は取り除く。
    fn move_to(mut self, to: Square) -> Self {
        self.occupied.clear(self.current);
        self.own.clear(self.current);
        if self.enemy.contains(to) {
            self.occupied.clear(to);
            self.enemy.clear(to);
        }
        self.occupied.set(to);
        self.own.set(to);
        self.current = to;
        self
    }
}

/// 相手獅子を取った直後に取り返される足(第13条)があるかどうかを返す。標準規則では、
/// 歩兵または仲人が唯一の足である場合、第1段階でその駒を取っても足が消滅したとは扱わない
/// (第16条第8項から第10項)。L3採用時は、これらの規定を適用せず、着手適用後の
/// 仮想盤面だけで足を判定する(第29条L3)。
fn lion_has_foot_after_capture(
    rules: Rules,
    position: &Position,
    mv: Move,
    lion_square: Square,
) -> bool {
    let defending_color = position
        .piece_at(lion_square)
        .and_then(|piece| piece.color())
        .expect("capture square must contain a lion");
    let board = VirtualBoard::after_move(position, mv);

    let captured_pawn_or_go_between_had_foot = if rules.contains(RuleCode::L3) {
        false
    } else {
        let [mid_capture, destination_capture] = position.captured_squares(mv);
        destination_capture == Some(lion_square)
            && mid_capture.is_some_and(|mid| {
                position.piece_at(mid).is_some_and(|piece| {
                    let Some(kind @ (PieceKind::Pawn | PieceKind::GoBetween)) = piece.kind() else {
                        return false;
                    };
                    piece_control_with_occupancy(board.occupied, defending_color, kind, mid)
                        .contains(mv.destination())
                })
            })
    };

    debug_assert!(board.own.contains(mv.destination()));
    debug_assert!(!board.enemy.contains(mv.destination()));
    captured_pawn_or_go_between_had_foot
        || square_is_controlled(position, board, defending_color, mv.destination())
}

/// 仮想盤面上で、指定した対局者のいずれかの駒が対象升に利きを持つかどうかを返す。
fn square_is_controlled(
    position: &Position,
    board: VirtualBoard,
    color: Color,
    target: Square,
) -> bool {
    for kind in PieceKind::ALL {
        let remaining = position.pieces_of_kind(color, kind) & board.enemy;
        for from in remaining {
            if piece_control_with_occupancy(board.occupied, color, kind, from).contains(target) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::movegen::MoveGenerator;
    use crate::core::piece::PieceCode;
    use crate::core::position::PositionBuilder;
    use crate::test_util::{position, sq};

    fn is_generated_with_rules(rules: Rules, position: &Position, expected: Move) -> bool {
        let mut moves = Vec::new();
        MoveGenerator::new(rules).generate_moves(position, &mut moves);
        moves.contains(&expected)
    }

    fn is_generated(position: &Position, expected: Move) -> bool {
        is_generated_with_rules(Rules::standard(), position, expected)
    }

    fn after_non_lion_capture(pieces: &[(Square, Color, PieceKind)], capture: Move) -> Position {
        let mut position = position(Color::Black, pieces);
        position.make_move_unchecked(capture);
        assert!(position.lion_taken_by_non_lion().is_some());
        position
    }

    #[test]
    fn article_33_9_conflicting_code_sets_are_invalid_and_other_inputs_are_validated() {
        assert_eq!(
            Rules::from_codes(&[RuleCode::L0, RuleCode::L1]),
            Err(RulesError::Conflicting {
                first: RuleCode::L0,
                second: RuleCode::L1,
            }),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::P1, RuleCode::P2]),
            Err(RulesError::Conflicting {
                first: RuleCode::P1,
                second: RuleCode::P2,
            }),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::R0, RuleCode::R1]),
            Err(RulesError::Conflicting {
                first: RuleCode::R0,
                second: RuleCode::R1,
            }),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::R0, RuleCode::R2]),
            Err(RulesError::Conflicting {
                first: RuleCode::R0,
                second: RuleCode::R2,
            }),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::R1, RuleCode::R2]),
            Err(RulesError::Conflicting {
                first: RuleCode::R1,
                second: RuleCode::R2,
            }),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::L0, RuleCode::L0]),
            Err(RulesError::Duplicate(RuleCode::L0)),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::R0, RuleCode::R0]),
            Err(RulesError::Duplicate(RuleCode::R0)),
        );
        for code in [RuleCode::L1, RuleCode::L2, RuleCode::P1, RuleCode::P2] {
            assert_eq!(
                Rules::from_codes(&[code]),
                Err(RulesError::Unsupported(code)),
            );
        }
        for code in [
            RuleCode::L3,
            RuleCode::P3,
            RuleCode::P4,
            RuleCode::R0,
            RuleCode::R1,
            RuleCode::R2,
            RuleCode::E1,
            RuleCode::E2,
        ] {
            assert!(Rules::from_codes(&[code]).is_ok());
        }
        assert!(
            Rules::from_codes(&[
                RuleCode::L3,
                RuleCode::P3,
                RuleCode::P4,
                RuleCode::R1,
                RuleCode::E1,
            ])
            .is_ok()
        );
    }

    #[test]
    fn articles_28_and_33_rules_retain_empty_and_explicit_l0_codes() {
        let standard = Rules::from_codes(&[]).unwrap();
        let explicit_l0 = Rules::from_codes(&[RuleCode::L0]).unwrap();

        assert!(!standard.contains(RuleCode::L0));
        assert!(explicit_l0.contains(RuleCode::L0));
    }

    #[test]
    fn article_31_r2_is_supported() {
        let rules = Rules::from_codes(&[RuleCode::R2]).unwrap();

        assert_eq!(rules.repetition_rule(), RepetitionRule::R2);
        assert!(rules.contains(RuleCode::R2));
    }

    #[test]
    fn article_33_5_engine_default_is_standard_minus_r0_plus_r1() {
        let rules = Rules::engine_default();

        assert_eq!(rules.repetition_rule(), RepetitionRule::R1);
        assert!(rules.mate_adjudication_enabled());
        assert!(!rules.piece_exhaustion_disabled());
        assert!(!rules.contains(RuleCode::L0));
        assert!(!rules.contains(RuleCode::E1));
        assert!(!rules.contains(RuleCode::E2));
    }

    #[test]
    fn article_9_all_eighteen_promotion_targets_are_explicit() {
        let promotion_pairs = [
            (PieceKind::GoldGeneral, PieceKind::Rook),
            (PieceKind::SilverGeneral, PieceKind::VerticalMover),
            (PieceKind::CopperGeneral, PieceKind::SideMover),
            (PieceKind::FerociousLeopard, PieceKind::Bishop),
            (PieceKind::BlindTiger, PieceKind::FlyingStag),
            (PieceKind::DrunkElephant, PieceKind::CrownPrince),
            (PieceKind::Pawn, PieceKind::GoldGeneral),
            (PieceKind::GoBetween, PieceKind::DrunkElephant),
            (PieceKind::Lance, PieceKind::WhiteHorse),
            (PieceKind::ReverseChariot, PieceKind::Whale),
            (PieceKind::SideMover, PieceKind::FreeBoar),
            (PieceKind::VerticalMover, PieceKind::FlyingOx),
            (PieceKind::Bishop, PieceKind::DragonHorse),
            (PieceKind::Rook, PieceKind::DragonKing),
            (PieceKind::DragonHorse, PieceKind::HornedFalcon),
            (PieceKind::DragonKing, PieceKind::SoaringEagle),
            (PieceKind::Kirin, PieceKind::Lion),
            (PieceKind::Phoenix, PieceKind::FreeKing),
        ];

        assert_eq!(promotion_pairs.len(), 18);
        for (unpromoted, promoted) in promotion_pairs {
            assert_eq!(unpromoted.promoted(), Some(promoted), "{unpromoted:?}");
        }
    }

    #[test]
    fn articles_15_8_and_16_1_tsukegui_exemption_requires_a_lion_mover() {
        let position = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(6, 6), Color::White, PieceKind::SoaringEagle),
                (sq(5, 5), Color::Black, PieceKind::SilverGeneral),
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(4, 1), Color::Black, PieceKind::Rook),
            ],
            Move {
                from: sq(0, 0),
                mid: None,
                to: sq(1, 1),
                promote: false,
            },
        );
        let double_capture = Move {
            from: sq(6, 6),
            mid: Some(sq(5, 5)),
            to: sq(4, 4),
            promote: false,
        };

        assert!(!Rules::standard().special_move_is_legal(&position, double_capture));
    }

    #[test]
    fn article_14_1_adjacent_lion_capture_is_unconditional() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(3, 2), Color::White, PieceKind::Lion),
                (sq(3, 5), Color::White, PieceKind::Rook),
            ],
        );
        let capture = Move {
            from: sq(2, 2),
            mid: None,
            to: sq(3, 2),
            promote: false,
        };

        assert!(is_generated(&position, capture));
    }

    #[test]
    fn article_14_2_lion_cannot_capture_a_defended_lion_at_distance_two() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Rook),
            ],
        );
        let capture = Move {
            from: sq(2, 2),
            mid: None,
            to: sq(4, 2),
            promote: false,
        };

        assert!(!is_generated(&position, capture));
    }

    #[test]
    fn articles_13_5_and_14_2_attackable_defender_still_blocks_distance_two_lion_capture() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(0, 5), Color::Black, PieceKind::Rook),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Rook),
            ],
        );
        let capture_defender = Move {
            from: sq(0, 5),
            mid: None,
            to: sq(4, 5),
            promote: false,
        };
        let capture_lion = Move {
            from: sq(2, 2),
            mid: None,
            to: sq(4, 2),
            promote: false,
        };

        assert!(is_generated(&position, capture_defender));
        assert!(!is_generated(&position, capture_lion));
    }

    #[test]
    fn articles_13_2_and_14_4_hidden_sliding_defender_counts_as_a_foot() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(6, 2), Color::White, PieceKind::Rook),
            ],
        );
        let capture = Move {
            from: sq(2, 2),
            mid: None,
            to: sq(4, 2),
            promote: false,
        };

        assert!(lion_has_foot_after_capture(
            Rules::standard(),
            &position,
            capture,
            sq(4, 2),
        ));
        assert!(!is_generated(&position, capture));
    }

    #[test]
    fn article_14_5_non_lion_can_capture_a_defended_lion() {
        let position = position(
            Color::Black,
            &[
                (sq(4, 0), Color::Black, PieceKind::Rook),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Rook),
            ],
        );
        let capture = Move {
            from: sq(4, 0),
            mid: None,
            to: sq(4, 2),
            promote: false,
        };

        assert!(is_generated(&position, capture));
    }

    #[test]
    fn article_14_3_undefended_lion_at_distance_two_can_be_captured() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(4, 2), Color::White, PieceKind::Lion),
            ],
        );
        let capture = Move {
            from: sq(2, 2),
            mid: None,
            to: sq(4, 2),
            promote: false,
        };

        assert!(is_generated(&position, capture));
    }

    #[test]
    fn articles_16_1_and_16_4_tsukegui_captures_a_defended_lion() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(3, 2), Color::White, PieceKind::SilverGeneral),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Rook),
            ],
        );
        let tsukegui = Move {
            from: sq(2, 2),
            mid: Some(sq(3, 2)),
            to: sq(4, 2),
            promote: false,
        };

        assert!(is_generated(&position, tsukegui));
    }

    #[test]
    fn articles_15_1_and_16_1_adjacent_double_capture_is_not_tsukegui() {
        let position = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(2, 2), Color::White, PieceKind::Lion),
                (sq(2, 3), Color::Black, PieceKind::SilverGeneral),
                (sq(3, 2), Color::Black, PieceKind::Lion),
                (sq(3, 5), Color::Black, PieceKind::Rook),
            ],
            Move {
                from: sq(0, 0),
                mid: None,
                to: sq(1, 1),
                promote: false,
            },
        );
        let adjacent_double_capture = Move {
            from: sq(2, 2),
            mid: Some(sq(2, 3)),
            to: sq(3, 2),
            promote: false,
        };

        assert!(!is_tsukegui(&position, adjacent_double_capture, sq(3, 2)));
        assert!(!is_generated(&position, adjacent_double_capture));
    }

    #[test]
    fn articles_12_13_and_16_8_9_10_restore_a_pawn_that_is_the_only_foot() {
        let position = position(
            Color::Black,
            &[
                (sq(4, 6), Color::Black, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Pawn),
                (sq(4, 4), Color::White, PieceKind::Lion),
            ],
        );
        let capture = Move {
            from: sq(4, 6),
            mid: Some(sq(4, 5)),
            to: sq(4, 4),
            promote: false,
        };

        assert!(lion_has_foot_after_capture(
            Rules::standard(),
            &position,
            capture,
            sq(4, 4),
        ));
        assert!(!is_generated(&position, capture));
    }

    #[test]
    fn article_29_l3_removes_a_captured_pawn_foot_between_lion_steps() {
        let position = position(
            Color::Black,
            &[
                (sq(5, 6), Color::Black, PieceKind::Lion),
                (sq(5, 5), Color::White, PieceKind::Pawn),
                (sq(5, 4), Color::White, PieceKind::Lion),
            ],
        );
        let capture = Move {
            from: sq(5, 6),
            mid: Some(sq(5, 5)),
            to: sq(5, 4),
            promote: false,
        };
        let l3 = Rules::from_codes(&[RuleCode::L3]).unwrap();

        assert!(lion_has_foot_after_capture(
            Rules::standard(),
            &position,
            capture,
            sq(5, 4),
        ));
        assert!(!lion_has_foot_after_capture(
            l3,
            &position,
            capture,
            sq(5, 4),
        ));
        assert!(!is_generated(&position, capture));
        assert!(is_generated_with_rules(l3, &position, capture));
    }

    #[test]
    fn articles_13_4_14_2_and_16_8_pawn_removal_opens_sliding_foot() {
        let position = position(
            Color::Black,
            &[
                (sq(3, 5), Color::Black, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Pawn),
                (sq(5, 5), Color::White, PieceKind::Lion),
                (sq(0, 5), Color::White, PieceKind::Rook),
            ],
        );
        let jump = Move {
            from: sq(3, 5),
            mid: None,
            to: sq(5, 5),
            promote: false,
        };
        let double = Move {
            from: sq(3, 5),
            mid: Some(sq(4, 5)),
            to: sq(5, 5),
            promote: false,
        };

        assert!(is_generated(&position, jump));
        assert!(!is_generated(&position, double));
    }

    #[test]
    fn article_13_4_vacating_origin_opens_sliding_foot() {
        let position = position(
            Color::Black,
            &[
                (sq(6, 2), Color::Black, PieceKind::Lion),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(9, 2), Color::White, PieceKind::Rook),
            ],
        );
        let before = position.clone();
        let capture = Move {
            from: sq(6, 2),
            mid: None,
            to: sq(4, 2),
            promote: false,
        };

        assert!(!is_generated(&position, capture));
        assert_eq!(position, before);
    }

    #[test]
    fn article_16_3_pawn_between_lions_is_not_tsukegui() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(3, 2), Color::White, PieceKind::Pawn),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::Rook),
            ],
        );
        let capture = Move {
            from: sq(2, 2),
            mid: Some(sq(3, 2)),
            to: sq(4, 2),
            promote: false,
        };

        assert!(!is_tsukegui(&position, capture, sq(4, 2)));
        assert!(!is_generated(&position, capture));
    }

    #[test]
    fn article_15_1_senjishi_prevents_immediate_capture_of_a_defended_lion() {
        let position = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(4, 3), Color::Black, PieceKind::GoldGeneral),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(4, 6), Color::White, PieceKind::Rook),
            ],
            Move {
                from: sq(0, 0),
                mid: None,
                to: sq(1, 1),
                promote: false,
            },
        );
        let recapture = Move {
            from: sq(4, 6),
            mid: None,
            to: sq(4, 4),
            promote: false,
        };

        assert!(!is_generated(&position, recapture));
    }

    #[test]
    fn article_15_2_senjishi_allows_immediate_capture_of_an_undefended_lion() {
        let position = after_non_lion_capture(
            &[
                (sq(1, 0), Color::Black, PieceKind::Pawn),
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(4, 6), Color::White, PieceKind::Rook),
            ],
            Move {
                from: sq(1, 0),
                mid: None,
                to: sq(1, 1),
                promote: false,
            },
        );
        let recapture = Move {
            from: sq(4, 6),
            mid: None,
            to: sq(4, 4),
            promote: false,
        };

        assert!(is_generated(&position, recapture));
    }

    #[test]
    fn articles_15_8_and_16_7_tsukegui_takes_priority_over_senjishi() {
        let position = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(4, 3), Color::Black, PieceKind::SilverGeneral),
                (sq(4, 7), Color::Black, PieceKind::Rook),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(4, 2), Color::White, PieceKind::Lion),
            ],
            Move {
                from: sq(0, 0),
                mid: None,
                to: sq(1, 1),
                promote: false,
            },
        );
        let tsukegui = Move {
            from: sq(4, 2),
            mid: Some(sq(4, 3)),
            to: sq(4, 4),
            promote: false,
        };

        assert!(is_generated(&position, tsukegui));
    }

    #[test]
    fn articles_15_8_and_16_7_tsukegui_exempts_the_whole_move_from_senjishi() {
        let mut builder = PositionBuilder::new(Color::Black);
        for (square, piece) in [
            (sq(0, 0), PieceCode::new(Color::Black, PieceKind::Bishop)),
            (sq(1, 1), PieceCode::new(Color::White, PieceKind::Lion)),
            (sq(2, 2), PieceCode::new(Color::White, PieceKind::Lion)),
            (sq(3, 2), PieceCode::new(Color::Black, PieceKind::Lion)),
            (
                sq(4, 2),
                PieceCode::new_promoted(Color::Black, PieceKind::Lion).unwrap(),
            ),
            (sq(4, 5), PieceCode::new(Color::Black, PieceKind::Rook)),
        ] {
            builder.put(square, piece).unwrap();
        }
        let mut position = builder.finish().unwrap();
        position.make_move_unchecked(Move {
            from: sq(0, 0),
            mid: None,
            to: sq(1, 1),
            promote: false,
        });
        assert!(position.lion_taken_by_non_lion().is_some());

        let tsukegui = Move {
            from: sq(2, 2),
            mid: Some(sq(3, 2)),
            to: sq(4, 2),
            promote: false,
        };
        assert!(is_tsukegui(&position, tsukegui, sq(4, 2)));
        assert!(is_generated(&position, tsukegui));
    }

    #[test]
    fn articles_15_6_and_16_6_lion_capture_does_not_trigger_senjishi() {
        let mut position = position(
            Color::Black,
            &[
                (sq(1, 1), Color::Black, PieceKind::Lion),
                (sq(2, 0), Color::Black, PieceKind::GoldGeneral),
                (sq(2, 1), Color::White, PieceKind::Lion),
                (sq(2, 4), Color::White, PieceKind::Rook),
            ],
        );
        position.make_move_unchecked(Move {
            from: sq(1, 1),
            mid: None,
            to: sq(2, 1),
            promote: false,
        });
        assert_eq!(position.lion_taken_by_non_lion(), None);

        let capture = Move {
            from: sq(2, 4),
            mid: None,
            to: sq(2, 1),
            promote: false,
        };
        assert!(is_generated(&position, capture));
    }

    #[test]
    fn articles_18_1_and_18_6_two_stage_move_with_only_mid_in_enemy_camp_cannot_promote() {
        let position = position(
            Color::Black,
            &[
                (sq(4, 7), Color::Black, PieceKind::DragonHorse),
                (sq(4, 8), Color::White, PieceKind::Pawn),
            ],
        );
        // This synthetic two-stage DragonHorse capture cannot be produced by the move generator;
        // it tests promotion_choice's isolated contract.
        let mv = Move {
            from: sq(4, 7),
            mid: Some(sq(4, 8)),
            to: sq(5, 7),
            promote: false,
        };

        assert_eq!(
            Rules::standard().promotion_choice(&position, &mv, PieceKind::DragonHorse),
            PromotionChoice::NoPromotion,
        );
    }

    #[test]
    fn article_18_3_non_capture_inside_enemy_camp_has_no_promotion_option() {
        let position = position(Color::Black, &[(sq(4, 8), Color::Black, PieceKind::Pawn)]);
        let non_promoting = Move {
            from: sq(4, 8),
            mid: None,
            to: sq(4, 9),
            promote: false,
        };
        let promoting = Move {
            from: sq(4, 8),
            mid: None,
            to: sq(4, 9),
            promote: true,
        };

        assert!(is_generated(&position, non_promoting));
        assert!(!is_generated(&position, promoting));
    }

    #[test]
    fn article_18_2_a_capture_inside_enemy_camp_can_promote() {
        let position = position(
            Color::Black,
            &[
                (sq(4, 8), Color::Black, PieceKind::Pawn),
                (sq(4, 9), Color::White, PieceKind::GoBetween),
            ],
        );
        let promotion = Move {
            from: sq(4, 8),
            mid: None,
            to: sq(4, 9),
            promote: true,
        };

        assert!(is_generated(&position, promotion));
    }

    #[test]
    fn article_18_2_b_capture_leaving_enemy_camp_can_promote() {
        let position = position(
            Color::Black,
            &[
                (sq(4, 8), Color::Black, PieceKind::Rook),
                (sq(4, 7), Color::White, PieceKind::Pawn),
            ],
        );
        let promotion = Move {
            from: sq(4, 8),
            mid: None,
            to: sq(4, 7),
            promote: true,
        };

        assert!(is_generated(&position, promotion));
    }

    #[test]
    fn article_19_1_pawn_can_promote_on_a_non_capture_to_the_last_rank() {
        let position = position(Color::Black, &[(sq(4, 10), Color::Black, PieceKind::Pawn)]);
        let promotion = Move {
            from: sq(4, 10),
            mid: None,
            to: sq(4, 11),
            promote: true,
        };

        assert!(is_generated(&position, promotion));
    }

    #[test]
    fn article_30_p3_lance_can_promote_on_a_non_capture_to_the_last_rank() {
        let position = position(Color::Black, &[(sq(4, 10), Color::Black, PieceKind::Lance)]);
        let non_promoting = Move {
            from: sq(4, 10),
            mid: None,
            to: sq(4, 11),
            promote: false,
        };
        let promoting = Move {
            promote: true,
            ..non_promoting
        };
        let p3 = Rules::from_codes(&[RuleCode::P3]).unwrap();

        assert!(is_generated(&position, non_promoting));
        assert!(!is_generated(&position, promoting));
        assert!(is_generated_with_rules(p3, &position, promoting));
    }

    #[test]
    fn article_30_p4_go_between_can_promote_on_a_non_capture_to_the_last_rank() {
        let position = position(
            Color::Black,
            &[(sq(4, 10), Color::Black, PieceKind::GoBetween)],
        );
        let non_promoting = Move {
            from: sq(4, 10),
            mid: None,
            to: sq(4, 11),
            promote: false,
        };
        let promoting = Move {
            promote: true,
            ..non_promoting
        };
        let p4 = Rules::from_codes(&[RuleCode::P4]).unwrap();

        assert!(is_generated(&position, non_promoting));
        assert!(!is_generated(&position, promoting));
        assert!(is_generated_with_rules(p4, &position, promoting));
    }

    #[test]
    fn article_26_8_try_make_move_rejects_unavailable_promotion() {
        let mut position = position(Color::Black, &[(sq(4, 4), Color::Black, PieceKind::Pawn)]);
        let illegal = Move {
            from: sq(4, 4),
            mid: None,
            to: sq(4, 5),
            promote: true,
        };

        assert_eq!(
            position.try_make_move(illegal, &MoveGenerator::standard()),
            Err(crate::IllegalMove(illegal)),
        );
    }

    #[test]
    fn article_27_1_try_make_move_error_preserves_entire_position() {
        let mut position = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(10, 10), Color::White, PieceKind::Pawn),
            ],
            Move {
                from: sq(0, 0),
                mid: None,
                to: sq(1, 1),
                promote: false,
            },
        );
        let before = position.clone();
        let side_to_move = position.side_to_move();
        let illegal = Move {
            from: sq(11, 11),
            mid: None,
            to: sq(11, 10),
            promote: false,
        };

        assert_eq!(
            position.try_make_move(illegal, &MoveGenerator::standard()),
            Err(crate::IllegalMove(illegal)),
        );
        assert_eq!(position, before);
        assert_eq!(position.side_to_move(), side_to_move);
    }
}
