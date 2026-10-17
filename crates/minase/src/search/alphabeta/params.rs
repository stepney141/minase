//! 探索と時間管理で使う整数の調整係数。
//!
//! SPSA（同時摂動確率近似）で調整した値を既定値とし、未調整の係数は各項目で区別する。
//! 百分率は対象の量に係数を掛けてから100で割り、整数除算は0方向へ丸める。
//! 通常ビルドでは定数関数、`tuning`ビルドでは範囲を検査して変更できる原子値として扱う。
//! 深さと手数は、特記しない限り片側の1着手を1手と数える。

#[cfg(feature = "tuning")]
use std::sync::atomic::{AtomicI32, Ordering};

/// 係数の宣言から、通常版の定数関数と調整版の読み出し・設定を生成する。
macro_rules! parameters {
    ($( $(#[$doc:meta])* $name:ident($accessor:ident): $default:literal, $min:literal, $max:literal; )*) => {
        $(
            $(#[$doc])*
            #[cfg(not(feature = "tuning"))]
            #[inline]
            pub(crate) const fn $accessor() -> i32 { $default }

            #[cfg(feature = "tuning")]
            mod $accessor {
                pub(super) static VALUE: super::AtomicI32 = super::AtomicI32::new($default);
            }

            $(#[$doc])*
            #[cfg(feature = "tuning")]
            #[inline]
            pub(crate) fn $accessor() -> i32 {
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
    /// 後順位の静かな手の減深（LMR）に使う対数積の除数を百分率で表す。
    /// 値はSPSAで調整した。
    LmrDivisor(lmr_divisor): 156, 100, 400;
    /// LMRの減深量を1増減するbutterfly historyの閾値。単位は履歴点である。
    /// 値はSPSAで調整した。
    LmrHistoryThreshold(lmr_history_threshold): 146, 0, 512;
    /// LMRの減深量の上限。単位は手である。
    /// StockfishとYaneuraOuには減深量の上限がなく、換算する元がないので、従来の定数と同じ3を既定値とする。
    /// 範囲の上限8は暫定の値である。SPSAの調整対象であり、この係数としての値は未調整である。
    /// 上限を設けた判断の根拠は`docs/adr/reduce-lmr-by-log-table-with-stepped-history-adjustment.md`にある。
    LmrMaxReduction(lmr_max_reduction): 3, 1, 8;
    /// 残り深さ1で補正後の静的評価に加えるfutility余裕値。単位は歩兵価値に対する百分率である。
    /// 良化していないノードでは[`non_improving_futility1()`]の倍率で縮める。
    /// 値はSPSAで調整した。
    FutilityMargin1(futility_margin1): 165, 0, 400;
    /// 残り深さ2で補正後の静的評価に加えるfutility余裕値。単位は歩兵価値に対する百分率である。
    /// 良化していないノードでは[`non_improving_futility2()`]の倍率で縮める。
    /// 値はSPSAで調整した。
    FutilityMargin2(futility_margin2): 239, 0, 400;
    /// 残り深さ3で補正後の静的評価に加えるfutility余裕値。単位は歩兵価値に対する百分率である。
    /// 良化していないノードでは[`non_improving_futility3()`]の倍率で縮める。
    /// 値はSPSAで調整した。
    FutilityMargin3(futility_margin3): 283, 0, 400;
    /// 良化していない残り深さ1のノードでfutility余裕値に掛ける倍率。単位は百分率である。
    /// 値はSPSAで調整した。
    NonImprovingFutility1(non_improving_futility1): 71, 0, 100;
    /// 良化していない残り深さ2のノードでfutility余裕値に掛ける倍率。単位は百分率である。
    /// 値はSPSAで調整した。
    NonImprovingFutility2(non_improving_futility2): 72, 0, 100;
    /// 良化していない残り深さ3のノードでfutility余裕値に掛ける倍率。単位は百分率である。
    /// 値はSPSAで調整した。
    NonImprovingFutility3(non_improving_futility3): 86, 0, 100;
    /// late move pruningで数える静かな手の上限の切片。単位は1/100手である。
    /// 値はSPSAで調整した。
    LmpBase(lmp_base): 382, 0, 8600;
    /// late move pruningの上限に残り深さの2乗を掛けて加える傾き。単位は残り深さの2乗あたり1/100手である。
    /// 値はSPSAで調整した。
    LmpSlope(lmp_slope): 133, 0, 3600;
    /// reverse futilityのmultの定数。単位は歩兵価値に対する百分率である。
    /// Stockfishは45/208で22（src/search.cpp:1025）、YaneuraOuは76/90で84である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityMultBase(reverse_futility_mult_base): 22, 0, 450;
    /// reverse futilityのmultの深さ1あたりの増分。単位は歩兵価値に対する百分率である。
    /// Stockfishは4/208で2（src/search.cpp:1025）、YaneuraOuは深さの項を持たず0である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityMultSlope(reverse_futility_mult_slope): 2, 0, 12;
    /// reverse futilityの置換表の照合に失敗した場合のmultの減算。単位は歩兵価値に対する百分率である。
    /// Stockfishは20/208で10（src/search.cpp:1026）、YaneuraOuは21/90で23である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityNoTtHit(reverse_futility_no_tt_hit): 10, 0, 50;
    /// reverse futilityのimprovingによる減算。単位はmultに対する百分率である。
    /// Stockfishは2789/1024で272（src/search.cpp:1029）、YaneuraOuは2686/1024で262である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityImproving(reverse_futility_improving): 272, 0, 400;
    /// reverse futilityのopponentWorseningによる減算。単位はmultに対する百分率である。
    /// Stockfishは335/1024で33（src/search.cpp:1029）、YaneuraOuは362/1024で35である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityOpponentWorsening(reverse_futility_opponent_worsening): 33, 0, 100;
    /// reverse futilityの補正による加算。単位は補正の値の絶対値に対する百分率である。
    /// Stockfishは131072/198435で66（src/search.cpp:1030）、YaneuraOuは131072/180600で73である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityCorrection(reverse_futility_correction): 66, 0, 200;
    /// 残り深さ1で静的評価に加えるrazoring余裕値。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    RazoringMargin1(razoring_margin1): 1606, 0, 2668;
    /// 残り深さ2で静的評価に加えるrazoring余裕値。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    RazoringMargin2(razoring_margin2): 2015, 0, 2885;
    /// 残り深さ1の静的交換評価（SEE）で許容する損の余裕値。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    SeeMargin1(see_margin1): 10, 0, 400;
    /// 残り深さ2の静的交換評価（SEE）で許容する損の余裕値。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    SeeMargin2(see_margin2): 192, 0, 400;
    /// 残り深さ3の静的交換評価（SEE）で許容する損の余裕値。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    SeeMargin3(see_margin3): 13, 0, 400;
    /// aspiration windowの初期半幅。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    AspirationDelta(aspiration_delta): 55, 10, 200;
    /// aspiration windowが次に外れた際の拡大量に掛ける倍率。単位は百分率である。
    /// 値はSPSAで調整した。
    AspirationGrowth(aspiration_growth): 190, 125, 400;
    /// null moveの基本減深量 `(切片 + 深さ × 傾き) / 1200` の切片。単位は1/1200手である。
    /// 値はSPSAで調整した。
    NullMoveBase(null_move_base): 3617, 1200, 4800;
    /// null moveの基本減深量の傾き。単位は残り深さ1手あたり1/1200手である。
    /// 値はSPSAで調整した。
    NullMoveSlope(null_move_slope): 257, 100, 400;
    /// 静的評価とβの差から求めるnull moveの追加減深の倍率。単位は百分率である。
    /// 値はSPSAで調整した。
    NullMoveEvalScale(null_move_eval_scale): 56, 0, 400;
    /// butterfly historyの絶対値の上限。単位は履歴点であり、重力式の除数にも使う。
    /// 値はSPSAで調整した。
    HistoryLimit(history_limit): 17408, 4096, 65536;
    /// βカットを起こした静かな手へ深さ²に比例して加点する尺度。単位は百分率である。
    /// SPSAの調整対象であり、この係数としての値は未調整である。
    HistoryBonusScale(history_bonus_scale): 100, 0, 400;
    /// 同じノードで先に探索した静かな手を深さ²に比例して減点する尺度。単位は百分率である。
    /// SPSAの調整対象であり、この係数としての値は未調整である。
    HistoryMalusScale(history_malus_scale): 100, 0, 400;
    /// 新しい根の探索へ持ち越すbutterfly historyの割合。単位は百分率である。
    /// 値はSPSAで調整した。
    HistoryDecay(history_decay): 23, 0, 100;
    /// 新しい根の探索へ持ち越す捕獲履歴の割合。単位は百分率である。
    /// SPSAの調整対象であり、この係数としての値は未調整である。
    CaptureHistoryDecay(capture_history_decay): 23, 0, 100;
    /// 捕獲履歴の絶対値の上限。単位は履歴点であり、重力式と駒価値への換算の除数にも使う。
    /// 値はSPSAで調整した。
    CaptureHistoryLimit(capture_history_limit): 16950, 4096, 65536;
    /// 整列キーへ加える捕獲履歴の換算値の絶対値の上限。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    CaptureHistoryScale(capture_history_scale): 49, 0, 400;
    /// correction historyの補正値の絶対値の上限。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    CorrectionCap(correction_cap): 220, 50, 400;
    /// correction historyの更新重み `min(深さ, 8) × 係数 / 1024` の係数。単位は1/1024である。
    /// 値はSPSAで調整した。
    CorrectionWeight(correction_weight): 37, 8, 128;
    /// 静止探索のdelta pruningの余裕値。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    DeltaMargin(delta_margin): 355, 50, 500;
    /// 静止探索で通常読む捕獲手数の上限。単位は手であり、例外は[`super::searcher::Searcher::quiesce`]に従う。
    /// 値はSPSAで調整した。
    QsearchMoveLimit(qsearch_move_limit): 4, 1, 7;
    /// 1局の開始から終局までに見込む両側の合計着手数。単位は手である。
    /// 値はSPSAで調整した。
    ExpectedPlies(expected_plies): 432, 250, 700;
    /// 時計の配分で見込む手番側の残り着手数の下限。単位は手である。
    /// 値はSPSAで調整した。
    MinMoves(min_moves): 88, 40, 200;
    /// 1手のsoft limitへ配分する加算時間の割合。単位は百分率である。
    /// 値はSPSAで調整した。
    IncrementShare(increment_share): 76, 30, 100;
    /// hard limitをsoft limitの基礎値の何倍までとするかを表す。単位は百分率である。
    /// 値はSPSAで調整した。
    HardSoftRatio(hard_soft_ratio): 451, 150, 800;
    /// hard limitへ配分できる残り持ち時間の割合。単位は百分率である。
    /// 値はSPSAで調整した。
    HardRemainingShare(hard_remaining_share): 27, 10, 50;
    /// 次の反復の完了時刻を予測する、直前の反復の累積経過時間に対する倍率。単位は百分率である。
    /// 値はSPSAで調整した。
    IterationRatio(iteration_ratio): 263, 150, 400;
    /// 因子分解機（FM）の評価補正に掛ける倍率。単位は1/1024であり、調整用ビルドの`Fm::correction`だけが参照する。
    /// 倍率はSPSAで調整して重みに反映するため、係数の既定値は補正を等倍に保つ。
    #[cfg_attr(not(feature = "tuning"), allow(dead_code))]
    FmScale(fm_scale): 1024, 0, 1024;
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
