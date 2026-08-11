//! ローカルルールの管理と、成り・獅子の捕獲制限の判定。

use core::fmt;
use core::str::FromStr;

use crate::core::movegen::{VirtualBoard, piece_control_with_occupancy};
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::Position;
use crate::core::square::{BOARD_RANKS, Square};

/// 第10章のローカルルールコード。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RuleCode {
    /// 岡崎式の足条件付き先獅子(第29条)。標準規則と同内容。
    L0,
    /// 非獅子による取り返しを足条件なしで禁じる先獅子(第29条)。
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
    /// Lishogi式の4回反復裁定(第31条)。
    R1,
    /// 既出局面の再現禁止(第31条)。
    R2,
    /// 既出局面の4回目の出現を生じさせる着手の禁止(第31条)。
    R3,
    /// 王駒実捕獲による終局(第32条)。
    E1,
    /// 駒枯れ不採用(第32条)。
    E2,
    /// Lishogi式裸玉即時裁定(第32条)。
    E3,
}

impl RuleCode {
    /// 全ローカルルールコード。
    pub const ALL: [Self; 14] = [
        Self::L0,
        Self::L1,
        Self::L2,
        Self::L3,
        Self::P1,
        Self::P2,
        Self::P3,
        Self::P4,
        Self::R1,
        Self::R2,
        Self::R3,
        Self::E1,
        Self::E2,
        Self::E3,
    ];

    /// ビット集合表現でこのコードが占めるビットを返す。
    const fn bit(self) -> u16 {
        1 << self as u8
    }

    const fn text(self) -> &'static str {
        match self {
            Self::L0 => "L0",
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::L3 => "L3",
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::P3 => "P3",
            Self::P4 => "P4",
            Self::R1 => "R1",
            Self::R2 => "R2",
            Self::R3 => "R3",
            Self::E1 => "E1",
            Self::E2 => "E2",
            Self::E3 => "E3",
        }
    }
}

impl fmt::Display for RuleCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.text())
    }
}

impl FromStr for RuleCode {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|code| input.eq_ignore_ascii_case(code.text()))
            .ok_or_else(|| format!("unknown rule code '{input}'"))
    }
}

/// 規則セット名と、名前が表す規則コード列の対応表。
///
/// `engine-default`は[`Rules::engine_default`]と同じR1を表す。
/// `lishogi`の組合せはRULES.md第33条第6項に基づき、名前と組合せの一致は
/// `tests/lishogi_replay.rs`の棋譜リプレイ照合が検証する。
const RULE_SET_PRESETS: &[(&str, &[RuleCode])] = &[
    ("engine-default", &[RuleCode::R1]),
    (
        "lishogi",
        &[
            RuleCode::L1,
            RuleCode::L2,
            RuleCode::P3,
            RuleCode::R1,
            RuleCode::E1,
            RuleCode::E3,
        ],
    ),
];

/// 規則セット値を、プリセット名またはコンマ区切りの規則コード列として解析する。
///
/// プリセット名は単独で指定し、規則コードとの併記は認めない(第33条第5項・第6項)。
/// 重複および排他制約の検証は`Rules::from_codes`が担う。
pub fn parse_rule_set(input: &str) -> Result<Vec<RuleCode>, String> {
    if let Some((_, codes)) = RULE_SET_PRESETS
        .iter()
        .find(|(name, _)| input.eq_ignore_ascii_case(name))
    {
        return Ok(codes.to_vec());
    }

    input
        .split(',')
        .map(|element| {
            if let Some((name, _)) = RULE_SET_PRESETS
                .iter()
                .find(|(name, _)| element.eq_ignore_ascii_case(name))
            {
                Err(format!("rule set preset '{name}' must be specified alone"))
            } else {
                element.parse()
            }
        })
        .collect()
}

/// 採用する反復規則。R1、R2およびR3は相互に排他である(第31条)。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RepetitionRule {
    /// Lishogi式の4回反復裁定。
    R1,
    /// 既出局面の再現禁止。
    R2,
    /// 既出局面の4回目の出現を生じさせる着手の禁止。
    R3,
}

/// ルールコード集合の検証エラー。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RulesError {
    /// 併用できないコードの組合せ(第33条第9項)。
    Conflicting { first: RuleCode, second: RuleCode },
    /// 同じコードの重複指定。
    Duplicate(RuleCode),
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
        }
    }
}

impl std::error::Error for RulesError {}

/// 対局で採用するローカルルールの集合。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rules {
    /// 採用コードのビット集合。L0は標準規則そのものなので保持せず、常に0ビットとする。
    codes: u16,
}

impl Rules {
    /// ローカル差分のない規則集合を返す。
    ///
    /// 実行可能な反復規則は含まないため、この値だけでは対局を構築できない。
    pub const fn standard() -> Self {
        Self { codes: 0 }
    }

    /// 反復規則にR1を採用したエンジン既定規則を返す。
    pub const fn engine_default() -> Self {
        Self {
            codes: RuleCode::R1.bit(),
        }
    }

    /// コード列から集合を作る。重複・矛盾を検査する。
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
            (RuleCode::R1, RuleCode::R2),
            (RuleCode::R1, RuleCode::R3),
            (RuleCode::R2, RuleCode::R3),
            (RuleCode::E2, RuleCode::E3),
        ] {
            if adopted & first.bit() != 0 && adopted & second.bit() != 0 {
                return Err(RulesError::Conflicting { first, second });
            }
        }

        adopted &= !RuleCode::L0.bit();
        Ok(Self { codes: adopted })
    }

    /// 指定コードを採用しているかどうかを返す。
    pub const fn contains(self, code: RuleCode) -> bool {
        self.codes & code.bit() != 0
    }

    /// 採用している反復規則を返す。指定がなければ`None`を返す。
    pub const fn repetition_rule(self) -> Option<RepetitionRule> {
        if self.contains(RuleCode::R1) {
            Some(RepetitionRule::R1)
        } else if self.contains(RuleCode::R2) {
            Some(RepetitionRule::R2)
        } else if self.contains(RuleCode::R3) {
            Some(RepetitionRule::R3)
        } else {
            None
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

    /// 裸玉即時裁定(第32条E3)を適用するかどうかを返す。E3採用時に適用する。
    pub const fn bare_king_adjudication(self) -> bool {
        self.contains(RuleCode::E3)
    }

    /// 着手で成りを選択できるかどうかを判定して返す(第18条・第19条・第30条)。
    pub(crate) fn promotion_choice(
        self,
        position: &Position,
        mv: &Move,
        moving_kind: PieceKind,
    ) -> PromotionChoice {
        let Some(piece) = position.piece_at(mv.from) else {
            return PromotionChoice::NoPromotion;
        };
        let Some(color) = piece.color() else {
            return PromotionChoice::NoPromotion;
        };
        if piece.is_promoted() || !moving_kind.can_promote() {
            return PromotionChoice::NoPromotion;
        }

        let from_in_zone = in_promotion_zone(color, mv.from);
        let to_in_zone = in_promotion_zone(color, mv.to);
        let has_capture = position
            .captured_squares(*mv)
            .into_iter()
            .any(|capture| capture.is_some());
        let enters_zone = !from_in_zone && to_in_zone;
        let capture_in_or_from_zone = has_capture && (from_in_zone || to_in_zone);
        let piece_reaches_last_rank_without_capture = !has_capture
            && match color {
                Color::Black => mv.to.rank() == BOARD_RANKS - 1,
                Color::White => mv.to.rank() == 0,
            }
            && match moving_kind {
                PieceKind::Pawn => true,
                PieceKind::Lance => self.contains(RuleCode::P3),
                PieceKind::GoBetween => self.contains(RuleCode::P4),
                _ => false,
            };
        let p2_waiting_promotion =
            self.contains(RuleCode::P2) && from_in_zone && to_in_zone && !has_capture;
        let p1_recovered_promotion = self.contains(RuleCode::P1)
            && from_in_zone
            && to_in_zone
            && !has_capture
            && !position.promotion_deferred().contains(mv.from);

        if enters_zone
            || capture_in_or_from_zone
            || piece_reaches_last_rank_without_capture
            || p2_waiting_promotion
            || p1_recovered_promotion
        {
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

        let moving_kind = position.piece_at(mv.from).and_then(|piece| piece.kind());
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
        if !move_is_tsukegui
            && let Some(trigger) = position.lion_taken_by_non_lion()
            && captured_lions.into_iter().flatten().any(|lion| {
                let l2_exemption = self.contains(RuleCode::L2)
                    && trigger.by_kirin_promotion
                    && lion == trigger.square
                    && position.piece_at(lion).is_some_and(PieceCode::is_promoted);
                !l2_exemption
                    && if self.contains(RuleCode::L1) {
                        moving_kind != Some(PieceKind::Lion)
                    } else {
                        lion_has_foot_after_capture(self, position, mv, lion)
                    }
            })
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
    if position.piece_at(mv.from).and_then(|piece| piece.kind()) != Some(PieceKind::Lion) {
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
        .from
        .file()
        .abs_diff(lion_square.file())
        .max(mv.from.rank().abs_diff(lion_square.rank()));

    match distance {
        1 => true,
        2 if is_tsukegui(position, mv, lion_square) => true,
        2 => !lion_has_foot_after_capture(rules, position, mv, lion_square),
        _ => false,
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
                        .contains(mv.to)
                })
            })
    };

    debug_assert!(board.own.contains(mv.to));
    debug_assert!(!board.enemy.contains(mv.to));
    captured_pawn_or_go_between_had_foot
        || square_is_controlled(position, board, defending_color, mv.to)
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
    use crate::core::bitboard::Bitboard;
    use crate::core::movegen::MoveGenerator;
    use crate::core::position::PositionBuilder;
    use crate::test_util::{position, position_from_codes, sq};

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
        position.make_move_unchecked(capture, Rules::standard());
        assert!(position.lion_taken_by_non_lion().is_some());
        position
    }

    #[test]
    fn rule_codes_parse_case_insensitively_and_display_canonically() {
        for code in RuleCode::ALL {
            let canonical = code.to_string();
            assert_eq!(canonical.parse::<RuleCode>(), Ok(code));
            assert_eq!(canonical.to_ascii_lowercase().parse::<RuleCode>(), Ok(code));
        }
        assert_eq!(
            "R4".parse::<RuleCode>(),
            Err("unknown rule code 'R4'".to_owned())
        );
    }

    #[test]
    fn article_33_6_lishogi_preset_is_case_insensitive() {
        let expected = vec![
            RuleCode::L1,
            RuleCode::L2,
            RuleCode::P3,
            RuleCode::R1,
            RuleCode::E1,
            RuleCode::E3,
        ];

        assert_eq!(parse_rule_set("lishogi"), Ok(expected.clone()));
        assert_eq!(parse_rule_set("LISHOGI"), Ok(expected));
    }

    #[test]
    fn article_33_5_engine_default_preset_matches_engine_default() {
        let expected = vec![RuleCode::R1];

        assert_eq!(parse_rule_set("engine-default"), Ok(expected.clone()));
        assert_eq!(parse_rule_set("ENGINE-DEFAULT"), Ok(expected.clone()));
        assert_eq!(Rules::from_codes(&expected), Ok(Rules::engine_default()));
    }

    #[test]
    fn article_33_6_lishogi_preset_must_be_specified_alone() {
        let error = parse_rule_set("lishogi,P1").unwrap_err();

        assert!(error.contains("preset 'lishogi' must be specified alone"));
    }

    #[test]
    fn article_33_5_engine_default_preset_must_be_specified_alone() {
        let error = parse_rule_set("engine-default,R2").unwrap_err();

        assert!(error.contains("preset 'engine-default' must be specified alone"));
    }

    #[test]
    fn unknown_rule_set_names_and_codes_are_rejected() {
        assert_eq!(
            parse_rule_set("standard"),
            Err("unknown rule code 'standard'".to_owned())
        );
        assert_eq!(
            parse_rule_set("hachu"),
            Err("unknown rule code 'hachu'".to_owned())
        );
        assert_eq!(
            parse_rule_set("L1,ZZ"),
            Err("unknown rule code 'ZZ'".to_owned())
        );
    }

    #[test]
    fn explicit_lishogi_code_list_matches_the_preset() {
        assert_eq!(
            parse_rule_set("L1,L2,P3,R1,E1,E3"),
            parse_rule_set("lishogi")
        );
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
            Rules::from_codes(&[RuleCode::R1, RuleCode::R2]),
            Err(RulesError::Conflicting {
                first: RuleCode::R1,
                second: RuleCode::R2,
            }),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::R1, RuleCode::R3]),
            Err(RulesError::Conflicting {
                first: RuleCode::R1,
                second: RuleCode::R3,
            }),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::R2, RuleCode::R3]),
            Err(RulesError::Conflicting {
                first: RuleCode::R2,
                second: RuleCode::R3,
            }),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::E2, RuleCode::E3]),
            Err(RulesError::Conflicting {
                first: RuleCode::E2,
                second: RuleCode::E3,
            }),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::R1, RuleCode::R2, RuleCode::R3]),
            Err(RulesError::Conflicting {
                first: RuleCode::R1,
                second: RuleCode::R2,
            }),
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::L0, RuleCode::L0]),
            Err(RulesError::Duplicate(RuleCode::L0)),
        );
        for code in RuleCode::ALL {
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
    fn article_33_1_explicit_l0_normalizes_to_standard() {
        let explicit_l0 = Rules::from_codes(&[RuleCode::L0]).unwrap();

        assert_eq!(explicit_l0, Rules::standard());
        assert!(!explicit_l0.contains(RuleCode::L0));
    }

    #[test]
    fn article_31_r2_is_supported() {
        let rules = Rules::from_codes(&[RuleCode::R2]).unwrap();

        assert_eq!(rules.repetition_rule(), Some(RepetitionRule::R2));
        assert!(rules.contains(RuleCode::R2));
    }

    #[test]
    fn article_31_r3_is_supported() {
        let rules = Rules::from_codes(&[RuleCode::R3]).unwrap();

        assert_eq!(rules.repetition_rule(), Some(RepetitionRule::R3));
        assert!(rules.contains(RuleCode::R3));
    }

    #[test]
    fn standard_rules_do_not_select_a_repetition_rule() {
        assert_eq!(Rules::standard().repetition_rule(), None);
    }

    #[test]
    fn article_33_5_engine_default_selects_r1() {
        let rules = Rules::engine_default();

        assert_eq!(rules.repetition_rule(), Some(RepetitionRule::R1));
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
    fn article_13_7_lion_foot_is_judged_without_recursion() {
        fn captures_target_lion(position: &Position) -> Vec<Move> {
            let mut moves = Vec::new();
            MoveGenerator::standard().generate_moves(position, &mut moves);
            moves
                .into_iter()
                .filter(|&mv| {
                    mv.from == sq(2, 2)
                        && position
                            .captured_squares(mv)
                            .into_iter()
                            .flatten()
                            .any(|square| square == sq(4, 2))
                })
                .collect()
        }

        let with_lion_foot = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(4, 2), Color::White, PieceKind::Lion),
                (sq(6, 2), Color::White, PieceKind::Lion),
            ],
        );

        // 足となる白獅子の取り返しには第13〜16条を再帰適用せず、
        // 捕獲直後の盤面で(4,2)へ到達できることだけを判定する。
        assert!(captures_target_lion(&with_lion_foot).is_empty());

        let without_lion_foot = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(4, 2), Color::White, PieceKind::Lion),
            ],
        );
        assert!(captures_target_lion(&without_lion_foot).contains(&Move {
            from: sq(2, 2),
            mid: None,
            to: sq(4, 2),
            promote: false,
        }));
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
    fn article_29_l2_allows_immediate_capture_of_the_new_promoted_lion() {
        let position = after_non_lion_capture(
            &[
                (sq(5, 9), Color::Black, PieceKind::Kirin),
                (sq(5, 11), Color::White, PieceKind::Lion),
                (sq(4, 11), Color::Black, PieceKind::GoldGeneral),
                (sq(5, 4), Color::White, PieceKind::Rook),
            ],
            Move {
                from: sq(5, 9),
                mid: None,
                to: sq(5, 11),
                promote: true,
            },
        );
        let trigger = position.lion_taken_by_non_lion().unwrap();
        assert_eq!(trigger.square, sq(5, 11));
        assert!(trigger.by_kirin_promotion);

        let recapture = Move {
            from: sq(5, 4),
            mid: None,
            to: sq(5, 11),
            promote: false,
        };
        let l2 = Rules::from_codes(&[RuleCode::L2]).unwrap();

        assert!(!is_generated(&position, recapture));
        assert!(is_generated_with_rules(l2, &position, recapture));
    }

    #[test]
    fn article_29_l2_does_not_exempt_an_existing_unpromoted_lion() {
        let position = after_non_lion_capture(
            &[
                (sq(5, 9), Color::Black, PieceKind::Kirin),
                (sq(5, 11), Color::White, PieceKind::Lion),
                (sq(4, 11), Color::Black, PieceKind::GoldGeneral),
                (sq(5, 4), Color::White, PieceKind::Rook),
                (sq(8, 7), Color::Black, PieceKind::Lion),
                (sq(7, 7), Color::Black, PieceKind::GoldGeneral),
                (sq(8, 4), Color::White, PieceKind::Rook),
            ],
            Move {
                from: sq(5, 9),
                mid: None,
                to: sq(5, 11),
                promote: true,
            },
        );
        let l2 = Rules::from_codes(&[RuleCode::L2]).unwrap();
        let recapture_new_lion = Move {
            from: sq(5, 4),
            mid: None,
            to: sq(5, 11),
            promote: false,
        };
        let capture_existing_lion = Move {
            from: sq(8, 4),
            mid: None,
            to: sq(8, 7),
            promote: false,
        };

        assert!(is_generated_with_rules(l2, &position, recapture_new_lion));
        assert!(!is_generated_with_rules(
            l2,
            &position,
            capture_existing_lion
        ));
    }

    #[test]
    fn article_29_l2_exempts_only_the_new_promoted_lion() {
        let mut position = position_from_codes(
            Color::Black,
            &[
                (sq(5, 9), PieceCode::new(Color::Black, PieceKind::Kirin)),
                (sq(5, 11), PieceCode::new(Color::White, PieceKind::Lion)),
                (
                    sq(8, 7),
                    PieceCode::new_promoted(Color::Black, PieceKind::Lion).unwrap(),
                ),
                (sq(6, 7), PieceCode::new(Color::Black, PieceKind::Lion)),
                (
                    sq(4, 11),
                    PieceCode::new(Color::Black, PieceKind::GoldGeneral),
                ),
                (
                    sq(7, 7),
                    PieceCode::new(Color::Black, PieceKind::GoldGeneral),
                ),
                (sq(5, 4), PieceCode::new(Color::White, PieceKind::Rook)),
                (sq(8, 4), PieceCode::new(Color::White, PieceKind::Rook)),
                (sq(6, 4), PieceCode::new(Color::White, PieceKind::Rook)),
            ],
        );
        position.make_move_unchecked(
            Move {
                from: sq(5, 9),
                mid: None,
                to: sq(5, 11),
                promote: true,
            },
            Rules::standard(),
        );
        assert_eq!(
            position
                .lion_taken_by_non_lion()
                .map(|trigger| trigger.square),
            Some(sq(5, 11))
        );

        let l2 = Rules::from_codes(&[RuleCode::L2]).unwrap();
        let capture = |from, to| Move {
            from,
            mid: None,
            to,
            promote: false,
        };

        assert!(is_generated_with_rules(
            l2,
            &position,
            capture(sq(5, 4), sq(5, 11))
        ));
        assert!(!is_generated_with_rules(
            l2,
            &position,
            capture(sq(8, 4), sq(8, 7))
        ));
        assert!(!is_generated_with_rules(
            l2,
            &position,
            capture(sq(6, 4), sq(6, 7))
        ));
    }

    #[test]
    fn articles_14_6_15_7_and_29_l2_apply_per_lion_in_a_double_capture() {
        let after_kirin_promotion = |with_foot| {
            let mut pieces = vec![
                (sq(5, 9), Color::Black, PieceKind::Kirin),
                (sq(5, 10), Color::Black, PieceKind::Lion),
                (sq(5, 11), Color::White, PieceKind::Lion),
                (sq(4, 10), Color::White, PieceKind::Lion),
            ];
            if with_foot {
                pieces.push((sq(4, 11), Color::Black, PieceKind::GoldGeneral));
            }
            after_non_lion_capture(
                &pieces,
                Move {
                    from: sq(5, 9),
                    mid: None,
                    to: sq(5, 11),
                    promote: true,
                },
            )
        };
        let capture_both_lions = Move {
            from: sq(4, 10),
            mid: Some(sq(5, 10)),
            to: sq(5, 11),
            promote: false,
        };
        let l2 = Rules::from_codes(&[RuleCode::L2]).unwrap();
        let l1_l2 = Rules::from_codes(&[RuleCode::L1, RuleCode::L2]).unwrap();

        // 第29条L2は、麒麟が成った到達升の新しい成獅子だけを免除する。
        // 同じ手で取る既存の不成獅子に足があれば、第15条1項により手全体を禁止する。
        assert!(!is_generated_with_rules(
            l2,
            &after_kirin_promotion(true),
            capture_both_lions
        ));
        // 不成獅子に足がなければ、標準の第15条2項ではL2と併せて両方を取れる。
        assert!(is_generated_with_rules(
            l2,
            &after_kirin_promotion(false),
            capture_both_lions
        ));
        // L1併用時は捕獲主体が獅子なので、第29条L1ではなく第14条だけに従う。
        // 両方の相手獅子が隣接しているため、第14条1項により足の有無を問わず取れる。
        assert!(is_generated_with_rules(
            l1_l2,
            &after_kirin_promotion(true),
            capture_both_lions
        ));
    }

    #[test]
    fn article_29_l1_prevents_immediate_non_lion_capture_regardless_of_foot() {
        let after_capture = |with_foot| {
            let mut pieces = vec![
                (sq(1, 0), Color::Black, PieceKind::Pawn),
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(4, 6), Color::White, PieceKind::Rook),
            ];
            if with_foot {
                pieces.push((sq(4, 0), Color::Black, PieceKind::Rook));
            }
            after_non_lion_capture(
                &pieces,
                Move {
                    from: sq(1, 0),
                    mid: None,
                    to: sq(1, 1),
                    promote: false,
                },
            )
        };
        let recapture = Move {
            from: sq(4, 6),
            mid: None,
            to: sq(4, 4),
            promote: false,
        };
        let l1 = Rules::from_codes(&[RuleCode::L1]).unwrap();

        // 標準規則では第15条1・2項により足の有無で結果が分かれる。
        assert!(!is_generated(&after_capture(true), recapture));
        assert!(is_generated(&after_capture(false), recapture));
        // 第29条L1では、非獅子による直後の取り返しを足の有無にかかわらず禁じる。
        assert!(!is_generated_with_rules(
            l1,
            &after_capture(true),
            recapture
        ));
        assert!(!is_generated_with_rules(
            l1,
            &after_capture(false),
            recapture
        ));
    }

    #[test]
    fn articles_11_1_11_2_14_5_and_29_l1_block_non_lion_double_captures() {
        let l1 = Rules::from_codes(&[RuleCode::L1]).unwrap();
        let cases = [
            (PieceKind::HornedFalcon, sq(5, 5), sq(5, 4), sq(5, 3)),
            (PieceKind::SoaringEagle, sq(6, 6), sq(5, 5), sq(4, 4)),
        ];

        for (kind, from, mid, to) in cases {
            let position = after_non_lion_capture(
                &[
                    (sq(0, 0), Color::Black, PieceKind::Pawn),
                    (sq(0, 1), Color::White, PieceKind::Lion),
                    (from, Color::White, kind),
                    (mid, Color::Black, PieceKind::Lion),
                    (to, Color::Black, PieceKind::Lion),
                ],
                Move {
                    from: sq(0, 0),
                    mid: None,
                    to: sq(0, 1),
                    promote: false,
                },
            );
            let capture_both_lions = Move {
                from,
                mid: Some(mid),
                to,
                promote: false,
            };

            // 第14条5項により標準規則では非獅子の2枚取りを許す。一方、L1では
            // 角鷹・飛鷲による取り返しを非獅子の駒による捕獲として禁止する。
            assert!(is_generated(&position, capture_both_lions), "{kind:?}");
            assert!(
                !is_generated_with_rules(l1, &position, capture_both_lions),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn articles_14_2_14_3_and_29_l1_govern_distance_two_lion_recapture_with_a_trigger() {
        let l1 = Rules::from_codes(&[RuleCode::L1]).unwrap();
        let capture = Move {
            from: sq(2, 2),
            mid: None,
            to: sq(4, 2),
            promote: false,
        };
        let defended = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Pawn),
                (sq(0, 1), Color::White, PieceKind::Lion),
                (sq(2, 2), Color::White, PieceKind::Lion),
                (sq(4, 2), Color::Black, PieceKind::Lion),
                (sq(4, 5), Color::Black, PieceKind::Rook),
            ],
            Move {
                from: sq(0, 0),
                mid: None,
                to: sq(0, 1),
                promote: false,
            },
        );
        let undefended = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Pawn),
                (sq(0, 1), Color::White, PieceKind::Lion),
                (sq(2, 2), Color::White, PieceKind::Lion),
                (sq(4, 2), Color::Black, PieceKind::Lion),
            ],
            Move {
                from: sq(0, 0),
                mid: None,
                to: sq(0, 1),
                promote: false,
            },
        );

        // 第29条L1でも獅子による取り返しは第14条だけに従う。距離2では、
        // 第13条の足があれば第14条2項により禁止され、なければ第3項により許される。
        assert!(!is_generated_with_rules(l1, &defended, capture));
        assert!(is_generated_with_rules(l1, &undefended, capture));
    }

    #[test]
    fn articles_14_1_and_29_l1_allow_adjacent_lion_recapture_with_a_trigger() {
        let position = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (sq(1, 1), Color::White, PieceKind::Lion),
                (sq(4, 5), Color::Black, PieceKind::Lion),
                (sq(4, 4), Color::Black, PieceKind::GoldGeneral),
                (sq(5, 6), Color::White, PieceKind::Lion),
            ],
            Move {
                from: sq(0, 0),
                mid: None,
                to: sq(1, 1),
                promote: false,
            },
        );
        let adjacent_recapture = Move {
            from: sq(5, 6),
            mid: None,
            to: sq(4, 5),
            promote: false,
        };
        let l1 = Rules::from_codes(&[RuleCode::L1]).unwrap();

        // 標準規則では足のある獅子への取り返しを第15条1項が禁じる。
        assert!(!is_generated(&position, adjacent_recapture));
        // 第29条L1では獅子による捕獲を第15条の足判定から外し、隣接捕獲を
        // 足の有無にかかわらず許す第14条1項だけを適用する。
        assert!(is_generated_with_rules(l1, &position, adjacent_recapture));
    }

    #[test]
    fn articles_14_2_16_1_16_4_and_29_l1_allow_lion_tsukegui() {
        let position = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(4, 3), Color::Black, PieceKind::SilverGeneral),
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
        let l1 = Rules::from_codes(&[RuleCode::L1]).unwrap();

        // L1は獅子による捕獲を制限せず、付け喰いは第16条1・4項により
        // 距離2で足のある獅子を取れる第14条2項の例外となる。
        assert!(is_generated_with_rules(l1, &position, tsukegui));
    }

    #[test]
    fn article_29_l2_exempts_a_new_promoted_lion_under_l1() {
        let position = after_non_lion_capture(
            &[
                (sq(5, 9), Color::Black, PieceKind::Kirin),
                (sq(5, 11), Color::White, PieceKind::Lion),
                (sq(4, 11), Color::Black, PieceKind::GoldGeneral),
                (sq(5, 4), Color::White, PieceKind::Rook),
            ],
            Move {
                from: sq(5, 9),
                mid: None,
                to: sq(5, 11),
                promote: true,
            },
        );
        let recapture = Move {
            from: sq(5, 4),
            mid: None,
            to: sq(5, 11),
            promote: false,
        };
        let l1 = Rules::from_codes(&[RuleCode::L1]).unwrap();
        let l1_l2 = Rules::from_codes(&[RuleCode::L1, RuleCode::L2]).unwrap();

        // 麒麟は捕獲時点では非獅子なので、L1だけなら新しい成獅子への
        // 非獅子による直後の取り返しを禁じ、L2併用時だけ同一升例外を適用する。
        assert!(!is_generated_with_rules(l1, &position, recapture));
        assert!(is_generated_with_rules(l1_l2, &position, recapture));
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
    fn articles_14_2_16_3_and_29_l1_l3_allow_lion_capture_after_foot_disappears() {
        let position = after_non_lion_capture(
            &[
                (sq(0, 0), Color::Black, PieceKind::Pawn),
                (sq(0, 1), Color::White, PieceKind::Lion),
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(4, 3), Color::Black, PieceKind::Pawn),
                (sq(4, 2), Color::White, PieceKind::Lion),
            ],
            Move {
                from: sq(0, 0),
                mid: None,
                to: sq(0, 1),
                promote: false,
            },
        );
        let capture = Move {
            from: sq(4, 2),
            mid: Some(sq(4, 3)),
            to: sq(4, 4),
            promote: false,
        };
        let l3 = Rules::from_codes(&[RuleCode::L3]).unwrap();
        let l1_l3 = Rules::from_codes(&[RuleCode::L1, RuleCode::L3]).unwrap();

        // 第16条3項により付け喰いではないが、L3では歩兵の足が第1段階で消滅する。
        // そのため第14条2・3項により捕獲でき、L1併用時も獅子の捕獲なので結果は同じ。
        assert!(is_generated_with_rules(l3, &position, capture));
        assert!(is_generated_with_rules(l1_l3, &position, capture));
    }

    #[test]
    fn articles_13_2_14_4_and_29_l3_keep_a_sliding_foot_opened_by_pawn_removal() {
        let position = position(
            Color::Black,
            &[
                (sq(2, 2), Color::Black, PieceKind::Lion),
                (sq(3, 3), Color::White, PieceKind::Pawn),
                (sq(4, 4), Color::White, PieceKind::Lion),
                (sq(0, 0), Color::White, PieceKind::Bishop),
            ],
        );
        let jump = Move {
            from: sq(2, 2),
            mid: None,
            to: sq(4, 4),
            promote: false,
        };
        let double = Move {
            from: sq(2, 2),
            mid: Some(sq(3, 3)),
            to: sq(4, 4),
            promote: false,
        };
        let l3 = Rules::from_codes(&[RuleCode::L3]).unwrap();

        // 跳びでは歩兵が角道を遮るので足はない。一方、2段階手は歩兵除去によって
        // 第13条2・4項の裏足が開くため、L3でも第14条4項により禁止される。
        assert!(is_generated_with_rules(l3, &position, jump));
        assert!(!is_generated_with_rules(l3, &position, double));
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
    fn articles_13_4_16_3_and_16_4_tsukegui_with_hidden_foot_opened_by_mid_capture() {
        let position = position(
            Color::Black,
            &[
                (sq(3, 5), Color::Black, PieceKind::Lion),
                (sq(4, 5), Color::White, PieceKind::SilverGeneral),
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
        let tsukegui = Move {
            from: sq(3, 5),
            mid: Some(sq(4, 5)),
            to: sq(5, 5),
            promote: false,
        };

        // 歩兵なら第16条3項により付け喰いが成立せず、銀の除去で開く裏足により不合法となる。
        // 銀は価値ある駒なので、同じ裏足が開いても第16条4項により付け喰いが優先される。
        assert!(is_generated(&position, tsukegui));
        assert!(is_generated(&position, jump));
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
        position.make_move_unchecked(
            Move {
                from: sq(0, 0),
                mid: None,
                to: sq(1, 1),
                promote: false,
            },
            Rules::standard(),
        );
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
        position.make_move_unchecked(
            Move {
                from: sq(1, 1),
                mid: None,
                to: sq(2, 1),
                promote: false,
            },
            Rules::standard(),
        );
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
    fn articles_15_6_and_16_6_tsukegui_does_not_trigger_senjishi() {
        let mut position = position(
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

        position.make_move_unchecked(tsukegui, Rules::standard());

        // 第16条6項: 付け喰いは獅子による獅子捕獲なので、先獅子を新設しない。
        // したがって、第15条6項どおり直後の飛車による取り返しも禁止されない。
        assert_eq!(position.lion_taken_by_non_lion(), None);
        assert!(is_generated(
            &position,
            Move {
                from: sq(4, 5),
                mid: None,
                to: sq(4, 2),
                promote: false,
            }
        ));
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
    fn article_30_p2_quiet_move_inside_enemy_camp_can_promote() {
        let mut position = position(
            Color::Black,
            &[(sq(4, 9), Color::Black, PieceKind::SilverGeneral)],
        );
        let non_promoting = Move {
            from: sq(4, 9),
            mid: None,
            to: sq(4, 10),
            promote: false,
        };
        let promoting = Move {
            promote: true,
            ..non_promoting
        };
        let p2 = Rules::from_codes(&[RuleCode::P2]).unwrap();

        assert!(is_generated(&position, non_promoting));
        assert!(!is_generated(&position, promoting));
        assert!(is_generated_with_rules(p2, &position, non_promoting));
        assert!(is_generated_with_rules(p2, &position, promoting));

        position.make_move_unchecked(non_promoting, p2);
        assert!(position.promotion_deferred().is_empty());
        assert_eq!(position.rights_zobrist(), 0);
    }

    #[test]
    fn article_30_p2_does_not_add_promotion_to_captures_or_enemy_camp_exit() {
        let capture_position = position(
            Color::Black,
            &[
                (sq(4, 9), Color::Black, PieceKind::SilverGeneral),
                (sq(4, 10), Color::White, PieceKind::Pawn),
            ],
        );
        let capture = Move {
            from: sq(4, 9),
            mid: None,
            to: sq(4, 10),
            promote: false,
        };
        let capture_promotion = Move {
            promote: true,
            ..capture
        };
        let p2 = Rules::from_codes(&[RuleCode::P2]).unwrap();
        let mut standard_capture_variants = Vec::new();
        MoveGenerator::standard().generate_moves(&capture_position, &mut standard_capture_variants);
        standard_capture_variants.retain(|mv| mv.from == capture.from && mv.to == capture.to);
        let mut p2_capture_variants = Vec::new();
        MoveGenerator::new(p2).generate_moves(&capture_position, &mut p2_capture_variants);
        p2_capture_variants.retain(|mv| mv.from == capture.from && mv.to == capture.to);

        assert_eq!(standard_capture_variants.len(), 2);
        assert!(standard_capture_variants.contains(&capture));
        assert!(standard_capture_variants.contains(&capture_promotion));
        assert_eq!(p2_capture_variants, standard_capture_variants);

        let exit_position = position(
            Color::Black,
            &[(sq(4, 8), Color::Black, PieceKind::SilverGeneral)],
        );
        let exit = Move {
            from: sq(4, 8),
            mid: None,
            to: sq(3, 7),
            promote: false,
        };
        let exit_promotion = Move {
            promote: true,
            ..exit
        };
        assert!(is_generated_with_rules(p2, &exit_position, exit));
        assert!(!is_generated_with_rules(p2, &exit_position, exit_promotion));
    }

    #[test]
    fn articles_19_1_19_4_and_30_p2_p3_p4_merge_last_rank_choices_without_duplicates() {
        let p2 = Rules::from_codes(&[RuleCode::P2]).unwrap();
        let p2_p3 = Rules::from_codes(&[RuleCode::P2, RuleCode::P3]).unwrap();
        let p2_p4 = Rules::from_codes(&[RuleCode::P2, RuleCode::P4]).unwrap();

        for (kind, rules) in [
            (PieceKind::Pawn, p2),
            (PieceKind::Lance, p2_p3),
            (PieceKind::GoBetween, p2_p4),
        ] {
            let from = sq(4, 10);
            let to = sq(4, 11);
            let position = position(Color::Black, &[(from, Color::Black, kind)]);
            let non_promoting = Move {
                from,
                mid: None,
                to,
                promote: false,
            };
            let promoting = Move {
                promote: true,
                ..non_promoting
            };
            let mut variants = Vec::new();
            MoveGenerator::new(rules).generate_moves(&position, &mut variants);
            variants.retain(|mv| mv.from == from && mv.to == to);

            // 第19条1・4項と第30条P2/P3/P4の条件が重なっても、成り選択肢は
            // PromotionOptionalの1組だけであり、不成・成を重複生成しない。
            assert_eq!(variants.len(), 2, "{kind:?}");
            assert_eq!(
                variants.iter().filter(|&&mv| mv == non_promoting).count(),
                1
            );
            assert_eq!(variants.iter().filter(|&&mv| mv == promoting).count(), 1);
        }

        // P2だけでも敵陣内の非捕獲移動として同じ選択肢を与えるため、P3/P4の
        // 追加採用は香車・仲人の最奥段着手を3通り以上へ増やさない。
        for kind in [PieceKind::Lance, PieceKind::GoBetween] {
            let position = position(Color::Black, &[(sq(4, 10), Color::Black, kind)]);
            let mut variants = Vec::new();
            MoveGenerator::new(p2).generate_moves(&position, &mut variants);
            variants.retain(|mv| mv.from == sq(4, 10) && mv.to == sq(4, 11));
            assert_eq!(variants.len(), 2, "{kind:?}");
        }
    }

    #[test]
    fn article_30_p1_deferred_promotion_toggles_after_one_piece_move() {
        let mut position = position(
            Color::Black,
            &[
                (sq(4, 7), Color::Black, PieceKind::SilverGeneral),
                (sq(10, 10), Color::White, PieceKind::GoldGeneral),
            ],
        );
        let p1 = Rules::from_codes(&[RuleCode::P1]).unwrap();
        let enter = Move {
            from: sq(4, 7),
            mid: None,
            to: sq(4, 8),
            promote: false,
        };
        let white_forward = Move {
            from: sq(10, 10),
            mid: None,
            to: sq(10, 9),
            promote: false,
        };
        let recover = Move {
            from: sq(4, 8),
            mid: None,
            to: sq(4, 9),
            promote: false,
        };
        let white_back = Move {
            from: sq(10, 9),
            mid: None,
            to: sq(10, 10),
            promote: false,
        };
        let use_recovered_right = Move {
            from: sq(4, 9),
            mid: None,
            to: sq(4, 10),
            promote: false,
        };
        let deferred_again = Move {
            from: sq(4, 10),
            mid: None,
            to: sq(4, 11),
            promote: false,
        };

        assert!(is_generated_with_rules(p1, &position, enter));
        assert!(is_generated_with_rules(
            p1,
            &position,
            Move {
                promote: true,
                ..enter
            }
        ));
        position.make_move_unchecked(enter, p1);
        assert_eq!(
            position.promotion_deferred(),
            Bitboard::from_square(sq(4, 8))
        );

        position.make_move_unchecked(white_forward, p1);
        assert!(is_generated_with_rules(p1, &position, recover));
        assert!(!is_generated_with_rules(
            p1,
            &position,
            Move {
                promote: true,
                ..recover
            }
        ));
        position.make_move_unchecked(recover, p1);
        assert!(position.promotion_deferred().is_empty());

        position.make_move_unchecked(white_back, p1);
        assert!(is_generated_with_rules(p1, &position, use_recovered_right));
        assert!(is_generated_with_rules(
            p1,
            &position,
            Move {
                promote: true,
                ..use_recovered_right
            }
        ));
        position.make_move_unchecked(use_recovered_right, p1);
        assert_eq!(
            position.promotion_deferred(),
            Bitboard::from_square(sq(4, 10))
        );

        position.make_move_unchecked(white_forward, p1);
        assert!(is_generated_with_rules(p1, &position, deferred_again));
        assert!(!is_generated_with_rules(
            p1,
            &position,
            Move {
                promote: true,
                ..deferred_again
            }
        ));
    }

    #[test]
    fn articles_18_2_a_18_5_and_30_p1_capture_restarts_or_consumes_deferral() {
        let deferred = sq(4, 9);
        let destination = sq(4, 10);
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(
                deferred,
                PieceCode::new(Color::Black, PieceKind::SilverGeneral),
            )
            .unwrap();
        builder
            .put(destination, PieceCode::new(Color::White, PieceKind::Pawn))
            .unwrap();
        builder.mark_promotion_deferred(deferred).unwrap();
        let position = builder.finish().unwrap();
        let p1 = Rules::from_codes(&[RuleCode::P1]).unwrap();
        let capture = Move {
            from: deferred,
            mid: None,
            to: destination,
            promote: false,
        };
        let promoting_capture = Move {
            promote: true,
            ..capture
        };

        // 第18条2項a・5項の捕獲による成り選択肢はP1保留中でも失われない。
        assert!(is_generated_with_rules(p1, &position, capture));
        assert!(is_generated_with_rules(p1, &position, promoting_capture));

        let mut declined = position.clone();
        declined.make_move_unchecked(capture, p1);
        // 捕獲時に再び不成を選べば、第30条P1の待機を到達升からやり直す。
        assert_eq!(
            declined.promotion_deferred(),
            Bitboard::from_square(destination)
        );

        let mut promoted = position;
        promoted.make_move_unchecked(promoting_capture, p1);
        // 捕獲時に成れば保留権は消費され、成駒へ保留ビットを残さない。
        assert!(promoted.promotion_deferred().is_empty());
        assert_eq!(promoted.rights_zobrist(), 0);
    }

    #[test]
    fn articles_7_3_12_4_12_8_24_1_d_and_30_p1_igui_clears_captured_piece_right() {
        let lion = sq(4, 9);
        let victim = sq(4, 10);
        let mut builder = PositionBuilder::new(Color::White);
        builder
            .put(lion, PieceCode::new(Color::White, PieceKind::Lion))
            .unwrap();
        builder
            .put(
                victim,
                PieceCode::new(Color::Black, PieceKind::SilverGeneral),
            )
            .unwrap();
        builder.mark_promotion_deferred(victim).unwrap();
        let mut position = builder.finish().unwrap();
        let p1 = Rules::from_codes(&[RuleCode::P1]).unwrap();
        let igui = Move {
            from: lion,
            mid: Some(victim),
            to: lion,
            promote: false,
        };

        // 第12条4・8項の中間升捕獲も第7条3項の捕獲であり、取られた駒の
        // 第24条1項d・第30条P1の一時権利を盤上から同時に除去する。
        assert!(is_generated_with_rules(p1, &position, igui));
        position.make_move_unchecked(igui, p1);
        assert_eq!(position.piece_at(victim), None);
        assert!(position.promotion_deferred().is_empty());
        assert_eq!(position.rights_zobrist(), 0);
    }

    #[test]
    fn article_30_p1_deferred_state_is_normalized_after_capture_exit_or_promotion() {
        let p1 = Rules::from_codes(&[RuleCode::P1]).unwrap();

        let deferred = sq(4, 9);
        let mut captured_builder = PositionBuilder::new(Color::White);
        captured_builder
            .put(
                deferred,
                PieceCode::new(Color::Black, PieceKind::SilverGeneral),
            )
            .unwrap();
        captured_builder
            .put(
                sq(4, 11),
                PieceCode::new(Color::White, PieceKind::ReverseChariot),
            )
            .unwrap();
        captured_builder.mark_promotion_deferred(deferred).unwrap();
        let mut captured = captured_builder.finish().unwrap();
        captured.make_move_unchecked(
            Move {
                from: sq(4, 11),
                mid: None,
                to: deferred,
                promote: false,
            },
            p1,
        );
        assert!(captured.promotion_deferred().is_empty());
        assert_eq!(captured.rights_zobrist(), 0);

        let mut exit_builder = PositionBuilder::new(Color::Black);
        exit_builder
            .put(
                sq(4, 8),
                PieceCode::new(Color::Black, PieceKind::SilverGeneral),
            )
            .unwrap();
        exit_builder.mark_promotion_deferred(sq(4, 8)).unwrap();
        let mut exit = exit_builder.finish().unwrap();
        exit.make_move_unchecked(
            Move {
                from: sq(4, 8),
                mid: None,
                to: sq(3, 7),
                promote: false,
            },
            p1,
        );
        assert!(exit.promotion_deferred().is_empty());
        assert_eq!(exit.rights_zobrist(), 0);

        let mut promotion_builder = PositionBuilder::new(Color::Black);
        promotion_builder
            .put(
                deferred,
                PieceCode::new(Color::Black, PieceKind::SilverGeneral),
            )
            .unwrap();
        promotion_builder
            .put(sq(4, 10), PieceCode::new(Color::White, PieceKind::Pawn))
            .unwrap();
        promotion_builder.mark_promotion_deferred(deferred).unwrap();
        let mut promotion = promotion_builder.finish().unwrap();
        promotion.make_move_unchecked(
            Move {
                from: deferred,
                mid: None,
                to: sq(4, 10),
                promote: true,
            },
            p1,
        );
        assert!(promotion.promotion_deferred().is_empty());
        assert_eq!(promotion.rights_zobrist(), 0);
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
