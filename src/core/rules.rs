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
    /// HaChu式の先獅子の非獅子限定(第29条)。
    L4,
    /// 標準規則の成りを表す識別子(第30条)。
    P0,
    /// Hodges式の成り権回復(第30条)。
    P1,
    /// 旧英語版Wikipedia式の成り(第30条)。
    P2,
    /// 香車の最奥段救済(第30条)。
    P3,
    /// 仲人の最奥段救済(第30条)。
    P4,
    /// HaChu式の歩兵の成り(第30条)。
    P5,
    /// HaChu式の前進専用駒の最奥段強制成り(第30条)。
    P6,
    /// Lishogi式の4回反復裁定(第31条)。
    R1,
    /// 既出局面の再現禁止(第31条)。
    R2,
    /// 既出局面の4回目の出現を生じさせる着手の禁止(第31条)。
    R3,
    /// 標準規則の駒枯れを表す識別子(第32条)。
    E0,
    /// 王駒実捕獲による終局(第32条)。
    E1,
    /// 駒枯れ不採用(第32条)。
    E2,
    /// Lishogi式裸玉即時裁定(第32条)。
    E3,
}

impl RuleCode {
    /// 全ローカルルールコード。
    pub const ALL: [Self; 19] = [
        Self::L0,
        Self::L1,
        Self::L2,
        Self::L3,
        Self::L4,
        Self::P0,
        Self::P1,
        Self::P2,
        Self::P3,
        Self::P4,
        Self::P5,
        Self::P6,
        Self::R1,
        Self::R2,
        Self::R3,
        Self::E0,
        Self::E1,
        Self::E2,
        Self::E3,
    ];

    /// コードの表示名を返す。
    const fn text(self) -> &'static str {
        match self {
            Self::L0 => "L0",
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::L3 => "L3",
            Self::L4 => "L4",
            Self::P0 => "P0",
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::P3 => "P3",
            Self::P4 => "P4",
            Self::P5 => "P5",
            Self::P6 => "P6",
            Self::R1 => "R1",
            Self::R2 => "R2",
            Self::R3 => "R3",
            Self::E0 => "E0",
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
    type Err = RuleCodeParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|code| input.eq_ignore_ascii_case(code.text()))
            .ok_or_else(|| RuleCodeParseError {
                input: input.to_owned(),
            })
    }
}

/// 規則コード文字列が既知のコードでないことを表すエラー。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuleCodeParseError {
    input: String,
}

impl RuleCodeParseError {
    /// 解釈できなかった入力を返す。
    pub fn input(&self) -> &str {
        &self.input
    }
}

impl fmt::Display for RuleCodeParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown rule code '{}'", self.input)
    }
}

impl std::error::Error for RuleCodeParseError {}

/// 規則セット文字列の構文エラー。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RuleSetParseError {
    /// 未知の規則コードが含まれている。
    UnknownCode(RuleCodeParseError),
    /// 規則セットのプリセット名が別の要素と併記されている。
    PresetMustBeAlone {
        /// 併記されたプリセット名。
        preset: &'static str,
    },
}

impl fmt::Display for RuleSetParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCode(error) => error.fmt(formatter),
            Self::PresetMustBeAlone { preset } => {
                write!(
                    formatter,
                    "rule set preset '{preset}' must be specified alone"
                )
            }
        }
    }
}

impl std::error::Error for RuleSetParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnknownCode(error) => Some(error),
            Self::PresetMustBeAlone { .. } => None,
        }
    }
}

impl From<RuleCodeParseError> for RuleSetParseError {
    fn from(error: RuleCodeParseError) -> Self {
        Self::UnknownCode(error)
    }
}

/// 規則セット名と、名前が表す規則集合の対応表。
///
/// `engine-default`は[`Rules::ENGINE_DEFAULT`]と同じ規則を表す。
/// `lishogi`の組合せはRULES.md第33条第6項に基づき、名前と組合せの一致は
/// `tests/lishogi_replay.rs`の棋譜リプレイ照合が検証する。
const RULE_SET_PRESETS: &[(&str, Rules)] = &[
    ("engine-default", Rules::ENGINE_DEFAULT),
    ("lishogi", Rules::LISHOGI),
];

/// 規則セット値を、プリセット名またはコンマ区切りの規則コード列として解析する。
///
/// プリセット名は単独で指定し、規則コードとの併記は認めない(第33条第5項・第6項)。
/// 重複および排他制約の検証は`Rules::from_codes`が担う。
pub fn parse_rule_set(input: &str) -> Result<Vec<RuleCode>, RuleSetParseError> {
    if let Some((_, rules)) = RULE_SET_PRESETS
        .iter()
        .find(|(name, _)| input.eq_ignore_ascii_case(name))
    {
        return Ok((*rules).into());
    }

    input
        .split(',')
        .map(|element| {
            if let Some((name, _)) = RULE_SET_PRESETS
                .iter()
                .find(|(name, _)| element.eq_ignore_ascii_case(name))
            {
                Err(RuleSetParseError::PresetMustBeAlone { preset: name })
            } else {
                element.parse().map_err(RuleSetParseError::from)
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

/// 採用する獅子規則。L0およびL1は相互に排他である(第29条・第33条第2項)。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LionRule {
    /// 岡崎式の足条件付き先獅子を採用する(第29条L0)。
    L0 {
        /// 先獅子の禁止を非獅子の駒による捕獲だけに限定する(第29条L4)。
        l4: bool,
    },
    /// 足条件なしで非獅子による取り返しを禁じる(第29条L1)。
    L1,
}

/// 採用する成り規則。P0、P1およびP2は相互に排他である(第30条・第33条第2項)。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PromotionRule {
    /// 標準規則の成りを採用する(第30条P0)。
    P0,
    /// Hodges式の成り権回復を採用する(第30条P1)。
    P1,
    /// 旧英語版Wikipedia式の成りを採用する(第30条P2)。
    P2,
}

/// 採用する駒枯れ規則。E0、E2およびE3は相互に排他である(第32条・第33条第2項)。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ExhaustionRule {
    /// 標準規則の駒枯れを採用する(第32条E0)。
    E0,
    /// 駒枯れを適用しない(第32条E2)。
    E2,
    /// Lishogi式裸玉即時裁定を採用する(第32条E3)。
    E3,
}

/// 規則コードが属する排他群(第33条第2項)。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RuleGroup {
    /// 獅子規則群。
    Lion,
    /// 成り規則群。
    Promotion,
    /// 反復規則群。
    Repetition,
    /// 駒枯れ規則群。
    Exhaustion,
}

/// ルールコード集合の検証エラー。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RulesError {
    /// 同じコードの重複指定(第33条第4項)。
    Duplicate(RuleCode),
    /// 併用できないコードの組合せ(第33条第9項)。
    Conflicting {
        /// 矛盾する組の一方。
        first: RuleCode,
        /// 矛盾する組のもう一方。
        second: RuleCode,
    },
    /// 必須の排他群が指定されていない(第33条第4項)。
    Missing(RuleGroup),
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
            Self::Missing(group) => {
                let name = match group {
                    RuleGroup::Lion => "lion",
                    RuleGroup::Promotion => "promotion",
                    RuleGroup::Repetition => "repetition",
                    RuleGroup::Exhaustion => "exhaustion",
                };
                write!(formatter, "missing {name} rule")
            }
        }
    }
}

impl std::error::Error for RulesError {}

/// 着手生成と局面更新に用いる規則(第29条・第30条)。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MoveRules {
    /// 採用する獅子規則(第29条L0・L1・L4)。
    pub lion: LionRule,
    /// 麒麟成獅子の同一升例外を採用するかどうか(第29条L2)。
    pub l2: bool,
    /// 段階別の足判定を採用するかどうか(第29条L3)。
    pub l3: bool,
    /// 採用する成り規則(第30条P0・P1・P2)。
    pub promotion: PromotionRule,
    /// 香車の最奥段救済を採用するかどうか(第30条P3)。
    pub p3: bool,
    /// 仲人の最奥段救済を採用するかどうか(第30条P4)。
    pub p4: bool,
    /// HaChu式の歩兵の成りを採用するかどうか(第30条P5)。
    pub p5: bool,
    /// 前進専用駒の最奥段強制成りを採用するかどうか(第30条P6)。
    pub p6: bool,
}

impl MoveRules {
    /// L0とP0からなる標準の着手規則を返す(第29条L0・第30条P0)。
    pub const fn standard() -> Self {
        Self {
            lion: LionRule::L0 { l4: false },
            l2: false,
            l3: false,
            promotion: PromotionRule::P0,
            p3: false,
            p4: false,
            p5: false,
            p6: false,
        }
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
        let reaches_last_rank = match color {
            Color::Black => mv.to.rank() == BOARD_RANKS - 1,
            Color::White => mv.to.rank() == 0,
        };
        let deferred = position.promotion_deferred().contains(mv.from);

        if self.p5 && moving_kind == PieceKind::Pawn && deferred {
            // 保留歩兵は、採用中の成り規則で成れる着手のうち到達升が最奥段である
            // 着手でのみ、必ず成る(第30条P5)。標準規則では敵陣内の捕獲着手に
            // 限られ、P2では非捕獲着手も該当する。
            let promotable = capture_in_or_from_zone
                || (self.promotion == PromotionRule::P2
                    && !has_capture
                    && (from_in_zone || to_in_zone));
            return if promotable && reaches_last_rank {
                PromotionChoice::PromotionForced
            } else {
                PromotionChoice::NoPromotion
            };
        }

        let piece_reaches_last_rank_without_capture = !has_capture
            && reaches_last_rank
            && match moving_kind {
                PieceKind::Pawn => !self.p5,
                PieceKind::Lance => self.p3,
                PieceKind::GoBetween => self.p4,
                _ => false,
            };
        let p2_waiting_promotion = self.promotion == PromotionRule::P2
            && !has_capture
            && (from_in_zone || to_in_zone)
            && !deferred;
        let p1_recovered_promotion = self.promotion == PromotionRule::P1
            && from_in_zone
            && to_in_zone
            && !has_capture
            && !deferred;

        if enters_zone
            || capture_in_or_from_zone
            || piece_reaches_last_rank_without_capture
            || p2_waiting_promotion
            || p1_recovered_promotion
        {
            if self.p6
                && matches!(moving_kind, PieceKind::Pawn | PieceKind::Lance)
                && reaches_last_rank
            {
                PromotionChoice::PromotionForced
            } else {
                PromotionChoice::PromotionOptional
            }
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

        // 獅子による獅子の捕獲制限(第14条・第16条)。
        let moving_kind = position.piece_at(mv.from).and_then(|piece| piece.kind());
        if moving_kind == Some(PieceKind::Lion)
            && captured_lions
                .into_iter()
                .flatten()
                .any(|lion| !lion_capture_is_legal(self, position, mv, lion))
        {
            return false;
        }

        // 先獅子による直後の捕獲禁止(第15条)。付け喰いは先獅子より優先する
        // (第16条第7項)ため、付け喰いが成立する着手には適用しない。
        let move_is_tsukegui = captured_lions
            .into_iter()
            .flatten()
            .any(|lion| is_tsukegui(position, mv, lion));
        if !move_is_tsukegui
            && let Some(trigger) = position.lion_taken_by_non_lion()
            && captured_lions.into_iter().flatten().any(|lion| {
                // L2採用時、麒麟が成った獅子への直後の取り返しは禁止しない(第29条L2)。
                let l2_exemption = self.l2
                    && trigger.by_kirin_promotion
                    && lion == trigger.square
                    && position.piece_at(lion).is_some_and(PieceCode::is_promoted);
                // L1は足の有無にかかわらず非獅子による取り返しを禁じ、
                // 標準規則(L0)は取った側に残る獅子に足がある場合だけ禁じる(第29条)。
                !l2_exemption
                    && match self.lion {
                        LionRule::L1 => moving_kind != Some(PieceKind::Lion),
                        LionRule::L0 { l4 } => {
                            // L4は禁止を非獅子の駒による捕獲に限定する(第29条L4)。
                            !(l4 && moving_kind == Some(PieceKind::Lion))
                                && lion_has_foot_after_capture(self, position, mv, lion)
                        }
                    }
            })
        {
            return false;
        }

        true
    }
}

/// 対局で採用する規則の集合(第29条から第33条)。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Rules {
    /// 着手生成と局面更新に用いる規則(第29条・第30条)。
    pub moves: MoveRules,
    /// 採用する反復規則(第31条R1・R2・R3)。
    pub repetition: RepetitionRule,
    /// 最後の王駒を実際に取るまで対局を続けるかどうか(第32条E1)。
    pub e1: bool,
    /// 採用する駒枯れ規則(第32条E0・E2・E3)。
    pub exhaustion: ExhaustionRule,
}

impl Rules {
    /// エンジン既定の規則集合L0・P0・R1・E0(第33条第5項)。
    pub const ENGINE_DEFAULT: Self = Self {
        moves: MoveRules::standard(),
        repetition: RepetitionRule::R1,
        e1: false,
        exhaustion: ExhaustionRule::E0,
    };

    /// Lishogiに近い規則集合L1・L2・P0・P3・R1・E1・E3(第33条第6項)。
    pub const LISHOGI: Self = Self {
        moves: MoveRules {
            lion: LionRule::L1,
            l2: true,
            l3: false,
            promotion: PromotionRule::P0,
            p3: true,
            p4: false,
            p5: false,
            p6: false,
        },
        repetition: RepetitionRule::R1,
        e1: true,
        exhaustion: ExhaustionRule::E3,
    };

    /// コード列を意味検証して規則集合を作る(第33条第4項・第9項)。
    pub fn from_codes(codes: &[RuleCode]) -> Result<Self, RulesError> {
        for (index, &code) in codes.iter().enumerate() {
            if codes[..index].contains(&code) {
                return Err(RulesError::Duplicate(code));
            }
        }

        let mut lion = None;
        let mut promotion = None;
        let mut repetition = None;
        let mut exhaustion = None;
        let mut l4 = false;

        for code in RuleCode::ALL {
            if !codes.contains(&code) {
                continue;
            }
            match code {
                RuleCode::L0 => assign_group(&mut lion, LionRule::L0 { l4: false }, code)?,
                RuleCode::L1 => assign_group(&mut lion, LionRule::L1, code)?,
                RuleCode::L2 | RuleCode::L3 => {}
                RuleCode::L4 => {
                    if lion.is_some_and(|(rule, _)| rule == LionRule::L1) {
                        return Err(RulesError::Conflicting {
                            first: RuleCode::L1,
                            second: RuleCode::L4,
                        });
                    }
                    l4 = true;
                }
                RuleCode::P0 => assign_group(&mut promotion, PromotionRule::P0, code)?,
                RuleCode::P1 => assign_group(&mut promotion, PromotionRule::P1, code)?,
                RuleCode::P2 => assign_group(&mut promotion, PromotionRule::P2, code)?,
                RuleCode::P3 | RuleCode::P4 | RuleCode::P5 | RuleCode::P6 => {}
                RuleCode::R1 => assign_group(&mut repetition, RepetitionRule::R1, code)?,
                RuleCode::R2 => assign_group(&mut repetition, RepetitionRule::R2, code)?,
                RuleCode::R3 => assign_group(&mut repetition, RepetitionRule::R3, code)?,
                RuleCode::E0 => assign_group(&mut exhaustion, ExhaustionRule::E0, code)?,
                RuleCode::E1 => {}
                RuleCode::E2 => assign_group(&mut exhaustion, ExhaustionRule::E2, code)?,
                RuleCode::E3 => assign_group(&mut exhaustion, ExhaustionRule::E3, code)?,
            }
        }

        let lion = lion
            .map(|(rule, _)| rule)
            .ok_or(RulesError::Missing(RuleGroup::Lion))?;
        let promotion = promotion
            .map(|(rule, _)| rule)
            .ok_or(RulesError::Missing(RuleGroup::Promotion))?;
        let repetition = repetition
            .map(|(rule, _)| rule)
            .ok_or(RulesError::Missing(RuleGroup::Repetition))?;
        let exhaustion = exhaustion
            .map(|(rule, _)| rule)
            .ok_or(RulesError::Missing(RuleGroup::Exhaustion))?;

        Ok(Self {
            moves: MoveRules {
                lion: match lion {
                    LionRule::L0 { .. } => LionRule::L0 { l4 },
                    LionRule::L1 => LionRule::L1,
                },
                l2: codes.contains(&RuleCode::L2),
                l3: codes.contains(&RuleCode::L3),
                promotion,
                p3: codes.contains(&RuleCode::P3),
                p4: codes.contains(&RuleCode::P4),
                p5: codes.contains(&RuleCode::P5),
                p6: codes.contains(&RuleCode::P6),
            },
            repetition,
            e1: codes.contains(&RuleCode::E1),
            exhaustion,
        })
    }
}

/// 排他群のスロットへ規則を入れる。既に埋まっていれば先に入れたコードとの`Conflicting`を返す。
fn assign_group<T: Copy>(
    slot: &mut Option<(T, RuleCode)>,
    value: T,
    second: RuleCode,
) -> Result<(), RulesError> {
    if let Some((_, first)) = *slot {
        return Err(RulesError::Conflicting { first, second });
    }
    *slot = Some((value, second));
    Ok(())
}

impl Rules {
    /// 指定コードを採用しているかどうかを返す。L0、P0およびE0も採用コードとして数える。
    const fn adopts(self, code: RuleCode) -> bool {
        match code {
            RuleCode::L0 => matches!(self.moves.lion, LionRule::L0 { .. }),
            RuleCode::L1 => matches!(self.moves.lion, LionRule::L1),
            RuleCode::L2 => self.moves.l2,
            RuleCode::L3 => self.moves.l3,
            RuleCode::L4 => matches!(self.moves.lion, LionRule::L0 { l4: true }),
            RuleCode::P0 => matches!(self.moves.promotion, PromotionRule::P0),
            RuleCode::P1 => matches!(self.moves.promotion, PromotionRule::P1),
            RuleCode::P2 => matches!(self.moves.promotion, PromotionRule::P2),
            RuleCode::P3 => self.moves.p3,
            RuleCode::P4 => self.moves.p4,
            RuleCode::P5 => self.moves.p5,
            RuleCode::P6 => self.moves.p6,
            RuleCode::R1 => matches!(self.repetition, RepetitionRule::R1),
            RuleCode::R2 => matches!(self.repetition, RepetitionRule::R2),
            RuleCode::R3 => matches!(self.repetition, RepetitionRule::R3),
            RuleCode::E0 => matches!(self.exhaustion, ExhaustionRule::E0),
            RuleCode::E1 => self.e1,
            RuleCode::E2 => matches!(self.exhaustion, ExhaustionRule::E2),
            RuleCode::E3 => matches!(self.exhaustion, ExhaustionRule::E3),
        }
    }
}

/// 採用コードを[`RuleCode::ALL`]の順に並べる。L0、P0およびE0を含む(第33条第4項)。
impl From<Rules> for Vec<RuleCode> {
    fn from(rules: Rules) -> Self {
        RuleCode::ALL
            .into_iter()
            .filter(|&code| rules.adopts(code))
            .collect()
    }
}

impl fmt::Display for Rules {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for code in RuleCode::ALL.into_iter().filter(|&code| self.adopts(code)) {
            if !first {
                formatter.write_str(",")?;
            }
            write!(formatter, "{code}")?;
            first = false;
        }
        Ok(())
    }
}

/// 着手に対する成りの選択肢。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PromotionChoice {
    /// この着手では成れない。
    NoPromotion,
    /// 成るか成らないかを選択できる(第18条第5項)。
    PromotionOptional,
    /// 必ず成る。不成の着手は生成しない(第30条P5・P6)。
    PromotionForced,
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
fn lion_capture_is_legal(
    rules: MoveRules,
    position: &Position,
    mv: Move,
    lion_square: Square,
) -> bool {
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
    rules: MoveRules,
    position: &Position,
    mv: Move,
    lion_square: Square,
) -> bool {
    let defending_color = position
        .piece_at(lion_square)
        .and_then(|piece| piece.color())
        .expect("capture square must contain a lion");
    let board = VirtualBoard::after_move(position, mv);

    let captured_pawn_or_go_between_had_foot = if rules.l3 {
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
    // 領域D2「獅子特殊規則とローカルルール」の挙動マトリクス
    // (scratchpad/matrices/d2-lion-local-rules.md、挙動ID D2-*)に基づくテスト。
    // 期待値の根拠はRULES.md第3条(9〜14号)・第13〜16条・第28〜33条だけである。
    // 座標はマトリクスの表記(筋1〜12、段a〜l)をmsqで盤座標へ変換して用いる。
    use super::*;
    use crate::core::movegen::MoveGenerator;
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
    fn article_15_1_senjishi_blocks_the_immediate_capture_of_the_footed_lion() {
        // 第15条1・3項(D2-015-01): 飛車9a×獅子9fの直後、取った側に残る後手獅子
        // 6cには足(銅5b)があるため先獅子が成立し、「飛車6i×獅子6c」は含まれない。
        let mut position = f9(true);
        play(base(), &mut position, mv(msq(9, 'a'), msq(9, 'f')));

        assert!(!is_generated(
            base(),
            &position,
            mv(msq(6, 'i'), msq(6, 'c'))
        ));
        // 禁止は保護された獅子の捕獲だけに掛かる(境界)。
        assert!(is_generated(
            base(),
            &position,
            mv(msq(6, 'i'), msq(6, 'd'))
        ));
    }

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

        play(base(), &mut position, mv(msq(12, 'l'), msq(12, 'k')));
        play(base(), &mut position, mv(msq(1, 'a'), msq(1, 'b')));

        assert!(is_generated(
            base(),
            &position,
            mv(msq(6, 'i'), msq(6, 'c'))
        ));
    }

    #[test]
    fn article_15_6_a_lion_capturing_a_lion_does_not_trigger_senjishi() {
        // 第15条6項・第14条1・5項(D2-015-04): 隣接獅子の捕獲(停止形)は合法で、
        // 獅子が獅子を取ったため先獅子は成立せず、直後の「金5d×獅子6e」が
        // 含まれる。居喰い形との差はarticle_3_14のテストが受け持つ。
        let mut position = f10();
        play(base(), &mut position, mv(msq(6, 'f'), msq(6, 'e')));

        assert!(is_generated(
            base(),
            &position,
            mv(msq(5, 'd'), msq(6, 'e'))
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
    fn article_16_1_passing_an_empty_mid_square_is_not_tsukegui() {
        // 第16条1項・第14条2項(D2-016-02): 付け喰いの成立要件は「第1段階での
        // 価値ある駒の捕獲」であり、2段階移動という形式ではない。経由升6gが空
        // なら足(金7e)により捕獲は含まれない。
        let f12b = f12(None, true);
        assert!(captures_of(base(), &f12b, msq(6, 'f')).is_empty());

        // 金7eを除けば足がなく取れる(第14条3項、境界)。
        let footless = f12(None, false);
        assert!(is_generated(
            base(),
            &footless,
            mv(msq(6, 'h'), msq(6, 'f'))
        ));
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
    fn article_16_11_a_mid_square_off_the_straight_line_still_counts_as_between() {
        // 第16条11項・1・4項(D2-016-04): 銀5gは両獅子を結ぶ直線上にないが、
        // 第1段階の経由升で取られる駒は「間」にある価値ある駒に当たる。唯一の
        // 足が経由捕獲される銀自身でも、付け喰いは足の有無と無関係に成立する。
        let position = fixture(
            Color::Black,
            &[
                (msq(6, 'h'), piece(Color::Black, PieceKind::Lion)),
                (msq(6, 'f'), piece(Color::White, PieceKind::Lion)),
                (msq(5, 'g'), piece(Color::White, PieceKind::SilverGeneral)),
            ],
        );

        assert!(is_generated(
            base(),
            &position,
            mv2(msq(6, 'h'), msq(5, 'g'), msq(6, 'f'))
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

        // F9系の先獅子観測(D2-015-01・02)がL0明示でも同一の結果になる。
        let mut footed = f9(true);
        play(l0, &mut footed, mv(msq(9, 'a'), msq(9, 'f')));
        assert!(!is_generated(l0, &footed, mv(msq(6, 'i'), msq(6, 'c'))));

        let mut footless = f9(false);
        play(l0, &mut footless, mv(msq(9, 'a'), msq(9, 'f')));
        assert!(is_generated(l0, &footless, mv(msq(6, 'i'), msq(6, 'c'))));
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
    fn article_29_l2_allows_immediate_capture_of_the_new_promoted_lion() {
        // 第29条L2・第15条7項(D2-029-04): L2は第15条7項を適用せず、麒麟成獅子
        // への直後の取り返しを認める。標準規則での禁止(D2-015-05)との反転対。
        let l0_l2 = rules_of(&[
            RuleCode::L0,
            RuleCode::L2,
            RuleCode::P0,
            RuleCode::R1,
            RuleCode::E0,
        ]);
        let mut position = f11(true);
        play(l0_l2, &mut position, mvp(msq(6, 'e'), msq(6, 'c')));

        assert!(is_generated(l0_l2, &position, mv(msq(5, 'b'), msq(6, 'c'))));
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

        let mut adjacent_standard = f17();
        play(base(), &mut adjacent_standard, mv(msq(9, 'a'), msq(9, 'f')));
        assert!(!is_generated(
            base(),
            &adjacent_standard,
            mv(msq(6, 'd'), msq(6, 'c'))
        ));

        let mut adjacent_l4 = f17();
        play(l4, &mut adjacent_l4, mv(msq(9, 'a'), msq(9, 'f')));
        assert!(is_generated(l4, &adjacent_l4, mv(msq(6, 'd'), msq(6, 'c'))));

        let mut non_lion = f16();
        play(l4, &mut non_lion, mv(msq(9, 'a'), msq(9, 'f')));
        assert!(!is_generated(l4, &non_lion, mv(msq(2, 'c'), msq(6, 'c'))));
    }

    // ---------- 第30〜33条 規則セットの検証 ----------

    #[test]
    fn article_29_30_all_rule_codes_round_trip_through_text() {
        assert_eq!(RuleCode::ALL.len(), 19);
        for code in RuleCode::ALL {
            assert_eq!(code.to_string().parse::<RuleCode>(), Ok(code));
        }
        assert!("R0".parse::<RuleCode>().is_err());
    }

    #[test]
    fn article_33_4_from_codes_accepts_complete_rule_sets() {
        let codes = [
            RuleCode::L0,
            RuleCode::L2,
            RuleCode::L3,
            RuleCode::L4,
            RuleCode::P1,
            RuleCode::P3,
            RuleCode::P4,
            RuleCode::P5,
            RuleCode::P6,
            RuleCode::R2,
            RuleCode::E1,
            RuleCode::E2,
        ];
        let rules = Rules::from_codes(&codes).unwrap();
        assert_eq!(Vec::<RuleCode>::from(rules), codes);
    }

    #[test]
    fn article_33_4_from_codes_reports_duplicate_conflict_and_missing_in_contract_order() {
        assert_eq!(
            Rules::from_codes(&[
                RuleCode::R1,
                RuleCode::R2,
                RuleCode::R1,
                RuleCode::L0,
                RuleCode::P0,
                RuleCode::E0,
            ]),
            Err(RulesError::Duplicate(RuleCode::R1))
        );
        assert_eq!(
            Rules::from_codes(&[
                RuleCode::L1,
                RuleCode::L4,
                RuleCode::P0,
                RuleCode::R1,
                RuleCode::E0,
            ]),
            Err(RulesError::Conflicting {
                first: RuleCode::L1,
                second: RuleCode::L4,
            })
        );
        assert_eq!(
            Rules::from_codes(&[
                RuleCode::L1,
                RuleCode::L0,
                RuleCode::P0,
                RuleCode::R1,
                RuleCode::E0,
            ]),
            Err(RulesError::Conflicting {
                first: RuleCode::L0,
                second: RuleCode::L1,
            })
        );
        assert_eq!(
            Rules::from_codes(&[
                RuleCode::L0,
                RuleCode::P0,
                RuleCode::P1,
                RuleCode::R1,
                RuleCode::R2,
                RuleCode::E0,
            ]),
            Err(RulesError::Conflicting {
                first: RuleCode::P0,
                second: RuleCode::P1,
            })
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::L4, RuleCode::P0, RuleCode::R1, RuleCode::E0]),
            Err(RulesError::Missing(RuleGroup::Lion))
        );
        assert_eq!(
            Rules::from_codes(&[]),
            Err(RulesError::Missing(RuleGroup::Lion))
        );
        assert_eq!(
            Rules::from_codes(&[RuleCode::R1]),
            Err(RulesError::Missing(RuleGroup::Lion))
        );
    }

    #[test]
    fn article_33_5_and_33_6_presets_expand_from_rule_constants() {
        for name in ["engine-default", "ENGINE-DEFAULT", "Engine-Default"] {
            let codes = parse_rule_set(name).unwrap();
            assert_eq!(Rules::from_codes(&codes), Ok(Rules::ENGINE_DEFAULT));
            assert_eq!(codes, Vec::<RuleCode>::from(Rules::ENGINE_DEFAULT));
        }
        for name in ["lishogi", "LISHOGI", "Lishogi"] {
            let codes = parse_rule_set(name).unwrap();
            assert_eq!(Rules::from_codes(&codes), Ok(Rules::LISHOGI));
            assert_eq!(codes, Vec::<RuleCode>::from(Rules::LISHOGI));
        }
        for invalid in ["engine-default,L1", "engine-default,lishogi", "lishogi,R1"] {
            assert!(parse_rule_set(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn rule_set_parse_errors_preserve_the_failure_kind() {
        let unknown = parse_rule_set("L0,unknown").unwrap_err();
        assert!(matches!(
            unknown,
            RuleSetParseError::UnknownCode(ref error) if error.input() == "unknown"
        ));
        assert!(std::error::Error::source(&unknown).is_some());

        assert_eq!(
            parse_rule_set("lishogi,R1"),
            Err(RuleSetParseError::PresetMustBeAlone { preset: "lishogi" })
        );
    }

    #[test]
    fn rule_set_display_includes_all_four_base_groups() {
        assert_eq!(Rules::ENGINE_DEFAULT.to_string(), "L0,P0,R1,E0");
        assert_eq!(Rules::LISHOGI.to_string(), "L1,L2,P0,P3,R1,E1,E3");
    }

    #[test]
    fn rules_error_display_is_stable() {
        assert_eq!(
            RulesError::Missing(RuleGroup::Lion).to_string(),
            "missing lion rule"
        );
        assert_eq!(
            RulesError::Missing(RuleGroup::Promotion).to_string(),
            "missing promotion rule"
        );
        assert_eq!(
            RulesError::Missing(RuleGroup::Repetition).to_string(),
            "missing repetition rule"
        );
        assert_eq!(
            RulesError::Missing(RuleGroup::Exhaustion).to_string(),
            "missing exhaustion rule"
        );
    }

    #[test]
    fn article_33_6_lishogi_move_rules_reproduce_the_capture_exception() {
        let rules = Rules::LISHOGI.moves;
        let mut position = f11(true);
        play(rules, &mut position, mvp(msq(6, 'e'), msq(6, 'c')));
        assert!(is_generated(rules, &position, mv(msq(5, 'b'), msq(6, 'c'))));
    }
}
