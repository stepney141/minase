//! ローカルルールコードと、その文字列の解析。

use core::fmt;
use core::str::FromStr;

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
