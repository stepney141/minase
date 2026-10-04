//! 対局で採用する規則セットの型と標準規則。

use core::fmt;

use super::RuleCode;

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
