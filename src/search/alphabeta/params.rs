//! 探索と時間管理の調整係数。単位と演算順序は`docs/plans/spsa.md`に従う。

#[cfg(feature = "tuning")]
use std::sync::atomic::{AtomicI32, Ordering};

/// 係数の宣言から、通常版の定数関数と調整版の読み出し・設定を生成する。
macro_rules! parameters {
    ($( $(#[$doc:meta])* $name:ident($accessor:ident): $default:literal, $min:literal, $max:literal; )*) => {
        $(
            $(#[$doc])*
            #[cfg(not(feature = "tuning"))]
            #[inline]
            pub(super) const fn $accessor() -> i32 { $default }

            #[cfg(feature = "tuning")]
            mod $accessor {
                pub(super) static VALUE: super::AtomicI32 = super::AtomicI32::new($default);
            }

            $(#[$doc])*
            #[cfg(feature = "tuning")]
            #[inline]
            pub(super) fn $accessor() -> i32 {
                $accessor::VALUE.load(Ordering::Relaxed)
            }
        )*

        /// USIへ宣言する係数の名前、既定値、最小値、最大値。
        #[cfg(feature = "tuning")]
        pub(crate) const PARAMETERS: &[(&str, i32, i32, i32)] = &[
            $((stringify!($name), $default, $min, $max),)*
        ];

        /// 名前と範囲を検査し、係数を更新する。
        #[cfg(feature = "tuning")]
        pub(crate) fn set(name: &str, value: i32) -> Result<(), Error> {
            match name {
                $(stringify!($name) => {
                    if !($min..=$max).contains(&value) {
                        return Err(Error::OutOfRange {
                            name: stringify!($name), value, min: $min, max: $max,
                        });
                    }
                    $accessor::VALUE.store(value, Ordering::Relaxed);
                    Ok(())
                },)*
                _ => Err(Error::UnknownName(name.to_owned())),
            }
        }
    };
}

parameters! {
    /// LMRの除数の百分率。`docs/plans/strength-stage5.md`「採用した係数」。
    LmrDivisor(lmr_divisor): 166, 100, 400;
    /// LMRのhistory閾値。`docs/plans/strength-stage5.md`「採用した係数」。
    LmrHistoryThreshold(lmr_history_threshold): 111, 0, 512;
    /// 深さ1の歩兵価値に対する百分率。`docs/plans/strength-stage4.md`「採用した余裕値」。
    FutilityMargin1(futility_margin1): 101, 0, 400;
    /// 深さ2の歩兵価値に対する百分率。`docs/plans/strength-stage4.md`「採用した余裕値」。
    FutilityMargin2(futility_margin2): 196, 0, 400;
    /// 深さ3の歩兵価値に対する百分率。`docs/plans/strength-stage4.md`「採用した余裕値」。
    FutilityMargin3(futility_margin3): 207, 0, 400;
    /// 深さ1のSEE余裕値の百分率。`docs/plans/strength-stage8.md`「採用した閾値」。
    SeeMargin1(see_margin1): 2, 0, 400;
    /// 深さ2のSEE余裕値の百分率。`docs/plans/strength-stage8.md`「採用した閾値」。
    SeeMargin2(see_margin2): 210, 0, 400;
    /// 深さ3のSEE余裕値の百分率。`docs/plans/strength-stage8.md`「採用した閾値」。
    SeeMargin3(see_margin3): 7, 0, 400;
    /// 初期窓幅の百分率。`docs/plans/strength-stage6.md`「窓の適用条件と拡大」。
    AspirationDelta(aspiration_delta): 46, 10, 200;
    /// 窓幅の拡大率の百分率。`docs/plans/strength-stage6.md`「窓の適用条件と拡大」。
    AspirationGrowth(aspiration_growth): 201, 125, 400;
    /// null moveの減深量の切片を1,200分率で表す。
    NullMoveBase(null_move_base): 3529, 1200, 4800;
    /// null moveの減深量の傾きを1,200分率で表す。
    NullMoveSlope(null_move_slope): 238, 100, 400;
    /// History値を全体の半減で抑える上限。
    HistoryLimit(history_limit): 20755, 4096, 65536;
    /// 補正値の上限の百分率。`docs/plans/strength-stage8.md`「静的評価の補正」。
    CorrectionCap(correction_cap): 193, 50, 400;
    /// 補正更新の重みを1,024分率で表す。`docs/plans/strength-stage8.md`「静的評価の補正」。
    CorrectionWeight(correction_weight): 33, 8, 128;
    /// delta pruningの余裕値を歩兵価値に対する百分率で表す。
    DeltaMargin(delta_margin): 258, 50, 500;
    /// 1局の開始から終局までに見込む手数。
    ExpectedPlies(expected_plies): 432, 250, 700;
    /// 1局面で見込む残り手数の下限。
    MinMoves(min_moves): 88, 40, 200;
    /// 加算時間の使用率。`docs/plans/time-management-efficiency.md`「採用した方式」。
    IncrementShare(increment_share): 76, 30, 100;
    /// hardとsoftの比の百分率。`docs/plans/time-management-efficiency.md`「採用した方式」。
    HardSoftRatio(hard_soft_ratio): 451, 150, 800;
    /// hardに使える残り時間の百分率。`docs/plans/time-management-efficiency.md`「採用した方式」。
    HardRemainingShare(hard_remaining_share): 27, 10, 50;
    /// 次の反復の予測時間比の百分率。`docs/plans/strength-stage6.md`「最善手安定時の早期終了」。
    /// 初期値2.5は、段階1の候補で測定した深さ5以上の累積時間比の中央値に基づく。
    IterationRatio(iteration_ratio): 263, 150, 400;
}

/// 調整係数の設定時のエラー。
#[cfg(feature = "tuning")]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    /// 宣言にない係数名。
    UnknownName(String),
    /// 宣言された範囲の外にある値。
    OutOfRange {
        name: &'static str,
        value: i32,
        min: i32,
        max: i32,
    },
}

#[cfg(feature = "tuning")]
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownName(name) => write!(f, "unknown tuning parameter '{name}'"),
            Self::OutOfRange {
                name,
                value,
                min,
                max,
            } => {
                write!(f, "Tune_{name} must be from {min} to {max}, got {value}")
            }
        }
    }
}

#[cfg(feature = "tuning")]
impl std::error::Error for Error {}
