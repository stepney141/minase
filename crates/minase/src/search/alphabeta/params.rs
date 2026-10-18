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
    /// 既定値はStockfishの表の積が約0.49 × ln d × ln m手となることから換算した値である
    /// （src/search.cpp:80-82, 1917-1920）。この式の形での値はSPSAで未調整である。
    LmrDivisor(lmr_divisor): 147, 100, 400;
    /// LMRの減深量を1増減するbutterfly historyの閾値。単位は履歴点である。
    /// 値はSPSAで調整した。
    LmrHistoryThreshold(lmr_history_threshold): 35, 0, 512;
    /// LMRの減深量の上限。単位は手である。
    /// StockfishとYaneuraOuには減深量の上限がなく、換算する元がないので、従来の定数と同じ3を既定値とする。
    /// 範囲の上限8は暫定の値である。SPSAの調整対象であり、この係数としての値は未調整である。
    /// 上限を設けた判断の根拠は`docs/adr/reduce-lmr-by-log-table-history-steps-and-stockfish-terms.md`にある。
    LmrMaxReduction(lmr_max_reduction): 3, 1, 8;
    /// LMRの基数。単位は1/100手である。
    /// Stockfishは948/1024で93（src/search.cpp:1919）、YaneuraOuは1133/1024で111。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrBase(lmr_base): 127, 0, 200;
    /// LMRの後半の加算量。単位は1/100手である。
    /// Stockfishは697/1024で68（src/search.cpp:1350）、YaneuraOuは691/1024で67。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrOffset(lmr_offset): 106, 0, 200;
    /// LMRの良化していない場合の表の値の増分。単位は百分率である。
    /// Stockfishは197/512で38（src/search.cpp:1919）、YaneuraOuは206/512で40。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrNonImproving(lmr_non_improving): 3, 0, 100;
    /// LMRのPVノードの減算量。単位は1/100手である。
    /// Stockfishは512/1024で50（src/search.cpp:1186-1187）、YaneuraOuは窓の幅とttPvに依存するため単一の対応値がない。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrPvReduction(lmr_pv_reduction): 6, 0, 200;
    /// LMRのttPvの前半の加算量。単位は1/100手である。
    /// Stockfishは929/1024で91（src/search.cpp:1191-1192）、YaneuraOuは1013/1024で99。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtPv(lmr_tt_pv): 70, 0, 200;
    /// LMRのttPvの後半の減算量の基数。単位は1/100手である。
    /// Stockfishは3023/1024で295（src/search.cpp:1346）、YaneuraOuは2819/1024で275。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtPvBase(lmr_tt_pv_base): 337, 0, 500;
    /// LMRのttPvかつPVノードの減算量。単位は1/100手である。
    /// Stockfishは1004/1024で98（src/search.cpp:1346）、YaneuraOuは973/1024で95。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtPvPv(lmr_tt_pv_pv): 13, 0, 200;
    /// LMRのttPvかつ置換表の値がαを超える場合の減算量。単位は1/100手である。
    /// Stockfishは885/1024で86（src/search.cpp:1346）、YaneuraOuは905/1024で88。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtPvValue(lmr_tt_pv_value): 95, 0, 200;
    /// LMRのttPvかつ置換表の深さが足りる場合の減算量。単位は1/100手である。
    /// Stockfishは816/1024で80（src/search.cpp:1347）、YaneuraOuは935/1024で91。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtPvDepth(lmr_tt_pv_depth): 77, 0, 200;
    /// LMRのttPvかつ置換表の深さが足りるcutNodeの減算量。単位は1/100手である。
    /// Stockfishは940/1024で92（src/search.cpp:1347）、YaneuraOuは959/1024で94。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtPvDepthCutNode(lmr_tt_pv_depth_cut_node): 90, 0, 200;
    /// LMRの手番号1つあたりの減算量。単位は1/100手である。
    /// Stockfishは65/1024で6（src/search.cpp:1352）、YaneuraOuは65/1024で6。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrMoveCount(lmr_move_count): 0, 0, 20;
    /// LMRのcutNodeの加算量。単位は1/100手である。
    /// Stockfishは4026/1024で393（src/search.cpp:1356-1357）、YaneuraOuは3611/1024で353。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrCutNode(lmr_cut_node): 164, 0, 600;
    /// LMRの置換表の手がないcutNodeの加算量。単位は1/100手である。
    /// Stockfishは933/1024で91（src/search.cpp:1356-1357）、YaneuraOuは985/1024で96。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrCutNodeNoTtMove(lmr_cut_node_no_tt_move): 59, 0, 200;
    /// LMRの置換表の手が捕獲手の場合の加算量。単位は1/100手である。
    /// Stockfishは1079/1024で105（src/search.cpp:1360-1361）、YaneuraOuは1054/1024で103。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtCapture(lmr_tt_capture): 0, 0, 250;
    /// LMRの子のβ打ち切り数が1より多い場合の加算量。単位は1/100手である。
    /// Stockfishは264/1024で26（src/search.cpp:1364-1365）、YaneuraOuは251/1024で25。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrCutoffCount(lmr_cutoff_count): 48, 0, 100;
    /// LMRの子のβ打ち切り数が2より多い場合の追加量。単位は1/100手である。
    /// Stockfishは1095/1024で107（src/search.cpp:1364-1365）、YaneuraOuは1124/1024で110。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrCutoffCountMany(lmr_cutoff_count_many): 232, 0, 250;
    /// LMRの子のβ打ち切り数が1より多いallNodeの追加量。単位は1/100手である。
    /// Stockfishは1138/1024で111（src/search.cpp:1364-1365）、YaneuraOuは1042/1024で102。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrCutoffCountAllNode(lmr_cutoff_count_all_node): 129, 0, 250;
    /// LMRの子のβ打ち切り数が1以下の置換表の手の減算量。単位は1/100手である。
    /// Stockfishは2179/1024で213（src/search.cpp:1368-1369）、YaneuraOuは2239/1024で219。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtMove(lmr_tt_move): 269, 0, 500;
    /// LMRのαと判定用の評価値の差が歩兵価値1枚分の場合の加算量。単位は1/100手である。
    /// Stockfishは3 × 208/1024で61（src/search.cpp:1383-1384）、YaneuraOuはこの項を持たず0。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrAlphaEval(lmr_alpha_eval): 46, 0, 150;
    /// LMRの分岐を通らず置換表の手がない場合の加算量。単位は1/100手である。
    /// Stockfishは1127/1024で110（src/search.cpp:1431-1432）、YaneuraOuは1057/1024で103。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrNoTtMove(lmr_no_tt_move): 57, 0, 250;
    /// LMRのallNodeで減深量に掛けて足す割合の分子。単位は百分率である。
    /// Stockfishは276/256で108（src/search.cpp:1387-1388）、YaneuraOuは273/256で107。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrAllNode(lmr_all_node): 142, 0, 200;
    /// LMRの再探索を1手深くするための余裕値。単位は歩兵価値に対する百分率である。
    /// Stockfishは53/208で25（src/search.cpp:1414）、YaneuraOuは48/90で53。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrDeeperMargin(lmr_deeper_margin): 82, 0, 100;
    /// LMRの再探索を1手浅くするための余裕値。単位は歩兵価値に対する百分率である。
    /// Stockfishは8/208で4（src/search.cpp:1415）、YaneuraOuは9/90で10。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrShallowerMargin(lmr_shallower_margin): 26, 0, 40;
    /// 静かな手のfutility見積りに加える定数。単位は歩兵価値に対する百分率である。
    /// Stockfishは164/208で79（src/search.cpp:1242-1243）、YaneuraOuは42/90で47（yaneuraou-search.cpp:3663-3664）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    FutilityBase(futility_base): 79, 0, 400;
    /// 静かな手のfutility見積りでlmrDepth 1あたりに加える量。単位は歩兵価値に対する百分率である。
    /// Stockfishは119/208で57（src/search.cpp:1242-1243）、YaneuraOuは120/90で133（yaneuraou-search.cpp:3663-3664）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    FutilityLmrDepth(futility_lmr_depth): 57, 0, 300;
    /// 静的評価がαより大きいときにfutility見積りへ加える量。単位は歩兵価値に対する百分率である。
    /// Stockfishは90/208で43（src/search.cpp:1242-1243）、YaneuraOuは86/90で96（yaneuraou-search.cpp:3663-3664）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    FutilityAboveAlpha(futility_above_alpha): 43, -100, 300;
    /// 静かな手のSEEでlmrDepthの2乗あたりに許容する損。単位は歩兵価値に対する百分率である。
    /// Stockfishは23/208で11、YaneuraOuは25/90で28である（Stockfish src/search.cpp:1255-1258、YaneuraOu yaneuraou-search.cpp:3691-3697）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    QuietSeeMargin(quiet_see_margin): 11, 0, 100;
    /// late move pruningで数える静かな手の上限の切片。単位は1/100手である。
    /// 既定値はStockfishとYaneuraOuの`3 + d²`（Stockfish src/search.cpp:1199、YaneuraOu yaneuraou-search.cpp:3593）を換算した値であり、
    /// この式の形での値はSPSAで未調整である。
    LmpBase(lmp_base): 1799, 0, 8600;
    /// late move pruningの上限に残り深さの2乗を掛けて加える傾き。単位は残り深さの2乗あたり1/100手である。
    /// 既定値はStockfishとYaneuraOuの`3 + d²`（Stockfish src/search.cpp:1199、YaneuraOu yaneuraou-search.cpp:3593）を換算した値であり、
    /// この式の形での値はSPSAで未調整である。
    LmpSlope(lmp_slope): 48, 0, 3600;
    /// improvingでないときにlate move pruningの上限へ掛ける倍率。単位は百分率である。
    /// StockfishとYaneuraOuの`/(2 − improving)`（Stockfish src/search.cpp:1199、YaneuraOu yaneuraou-search.cpp:3593）に当たり、
    /// 両エンジンとも50に相当する。倍率を掛けない現行の式は100に相当する。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmpNonImproving(lmp_non_improving): 31, 25, 150;
    /// reverse futilityのmultの定数。単位は歩兵価値に対する百分率である。
    /// Stockfishは45/208で22（src/search.cpp:1025）、YaneuraOuは76/90で84である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityMultBase(reverse_futility_mult_base): 235, 0, 450;
    /// reverse futilityのmultの深さ1あたりの増分。単位は歩兵価値に対する百分率である。
    /// Stockfishは4/208で2（src/search.cpp:1025）、YaneuraOuは深さの項を持たず0である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityMultSlope(reverse_futility_mult_slope): 4, 0, 12;
    /// reverse futilityの置換表の照合に失敗した場合のmultの減算。単位は歩兵価値に対する百分率である。
    /// Stockfishは20/208で10（src/search.cpp:1026）、YaneuraOuは21/90で23である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityNoTtHit(reverse_futility_no_tt_hit): 0, 0, 50;
    /// reverse futilityのimprovingによる減算。単位はmultに対する百分率である。
    /// Stockfishは2789/1024で272（src/search.cpp:1029）、YaneuraOuは2686/1024で262である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityImproving(reverse_futility_improving): 144, 0, 400;
    /// reverse futilityのopponentWorseningによる減算。単位はmultに対する百分率である。
    /// Stockfishは335/1024で33（src/search.cpp:1029）、YaneuraOuは362/1024で35である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityOpponentWorsening(reverse_futility_opponent_worsening): 20, 0, 100;
    /// reverse futilityの補正による加算。単位は補正の値の絶対値に対する百分率である。
    /// Stockfishは131072/198435で66（src/search.cpp:1030）、YaneuraOuは131072/180600で73である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityCorrection(reverse_futility_correction): 124, 0, 200;
    /// 残り深さ1で静的評価に加えるrazoring余裕値。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    RazoringMargin1(razoring_margin1): 2622, 0, 2668;
    /// 残り深さ2で静的評価に加えるrazoring余裕値。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    RazoringMargin2(razoring_margin2): 2885, 0, 2885;
    /// 静的交換評価（SEE）で許容する損の残り深さ1あたりの余裕値。単位は歩兵価値に対する百分率である。
    /// Stockfishは177/208で85、YaneuraOuは167/90で186である（Stockfish src/search.cpp:1222、YaneuraOu yaneuraou-search.cpp:3631）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    SeeDepthMargin(see_depth_margin): 85, 0, 400;
    /// SEEの余裕値に加えるcapture historyの重み。単位は履歴値に対する1/1024である。
    /// StockfishとYaneuraOuはともに34である（Stockfish src/search.cpp:1222、YaneuraOu yaneuraou-search.cpp:3631）。
    /// 開始値はStockfishの値であり、SPSAで未調整である。
    SeeCaptureHistory(see_capture_history): 34, -50, 200;
    /// 捕獲手のfutility見積りに加える定数。単位は歩兵価値に対する百分率である。
    /// Stockfishは234/208で113、YaneuraOuは218/90で242である（Stockfish src/search.cpp:1213、YaneuraOu yaneuraou-search.cpp:3616）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    CaptureFutilityBase(capture_futility_base): 113, 0, 600;
    /// 捕獲手のfutility見積りでlmrDepth 1あたりに加える量。単位は歩兵価値に対する百分率である。
    /// Stockfishは247/208で119、YaneuraOuは223/90で248である（Stockfish src/search.cpp:1213、YaneuraOu yaneuraou-search.cpp:3616）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    CaptureFutilityLmrDepth(capture_futility_lmr_depth): 119, 0, 500;
    /// 捕獲手のfutility見積りに加えるcapture historyの重み。単位は履歴値に対する1/1024である。
    /// Stockfishは134、YaneuraOuは131である（Stockfish src/search.cpp:1214、YaneuraOu yaneuraou-search.cpp:3617）。
    /// 開始値はStockfishの値であり、SPSAで未調整である。
    CaptureFutilityHistory(capture_futility_history): 134, -100, 600;
    /// aspiration windowの初期半幅。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    AspirationDelta(aspiration_delta): 57, 10, 200;
    /// aspiration windowが次に外れた際の拡大量に掛ける倍率。単位は百分率である。
    /// 値はSPSAで調整した。
    AspirationGrowth(aspiration_growth): 168, 125, 400;
    /// null moveの基本減深量 `(切片 + 深さ × 傾き) / 1200` の切片。単位は1/1200手である。
    /// 値はSPSAで調整した。
    NullMoveBase(null_move_base): 4799, 1200, 4800;
    /// null moveの基本減深量の傾き。単位は残り深さ1手あたり1/1200手である。
    /// 値はSPSAで調整した。
    NullMoveSlope(null_move_slope): 393, 100, 400;
    /// 静的評価とβの差から求めるnull moveの追加減深の倍率。単位は百分率である。
    /// 値はSPSAで調整した。
    NullMoveEvalScale(null_move_eval_scale): 133, 0, 400;
    /// butterfly historyの絶対値の上限。単位は履歴点であり、重力式の除数にも使う。
    /// 値はSPSAで調整した。
    HistoryLimit(history_limit): 5808, 4096, 65536;
    /// βカットを起こした静かな手へ深さ²に比例して加点する尺度。単位は百分率である。
    /// SPSAの調整対象であり、この係数としての値は未調整である。
    HistoryBonusScale(history_bonus_scale): 207, 0, 400;
    /// 同じノードで先に探索した静かな手を深さ²に比例して減点する尺度。単位は百分率である。
    /// SPSAの調整対象であり、この係数としての値は未調整である。
    HistoryMalusScale(history_malus_scale): 110, 0, 400;
    /// 新しい根の探索へ持ち越すbutterfly historyの割合。単位は百分率である。
    /// 値はSPSAで調整した。
    HistoryDecay(history_decay): 37, 0, 100;
    /// 新しい根の探索へ持ち越す捕獲履歴の割合。単位は百分率である。
    /// SPSAの調整対象であり、この係数としての値は未調整である。
    CaptureHistoryDecay(capture_history_decay): 13, 0, 100;
    /// 捕獲履歴の絶対値の上限。単位は履歴点であり、重力式と駒価値への換算の除数にも使う。
    /// 値はSPSAで調整した。
    CaptureHistoryLimit(capture_history_limit): 8376, 4096, 65536;
    /// 整列キーへ加える捕獲履歴の換算値の絶対値の上限。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    CaptureHistoryScale(capture_history_scale): 155, 0, 400;
    /// correction historyの補正値の絶対値の上限。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    CorrectionCap(correction_cap): 181, 50, 400;
    /// correction historyの更新重み `min(深さ, 8) × 係数 / 1024` の係数。単位は1/1024である。
    /// 値はSPSAで調整した。
    CorrectionWeight(correction_weight): 76, 8, 128;
    /// 静止探索のdelta pruningの余裕値。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    DeltaMargin(delta_margin): 500, 50, 500;
    /// 静止探索で通常読む捕獲手数の上限。単位は手であり、例外は[`super::searcher::Searcher::quiesce`]に従う。
    /// 値はSPSAで調整した。
    QsearchMoveLimit(qsearch_move_limit): 6, 1, 7;
    /// 1局の開始から終局までに見込む両側の合計着手数。単位は手である。
    /// 値はSPSAで調整した。
    ExpectedPlies(expected_plies): 458, 250, 700;
    /// 時計の配分で見込む手番側の残り着手数の下限。単位は手である。
    /// 値はSPSAで調整した。
    MinMoves(min_moves): 40, 40, 200;
    /// 1手のsoft limitへ配分する加算時間の割合。単位は百分率である。
    /// 値はSPSAで調整した。
    IncrementShare(increment_share): 87, 30, 100;
    /// hard limitをsoft limitの基礎値の何倍までとするかを表す。単位は百分率である。
    /// 値はSPSAで調整した。
    HardSoftRatio(hard_soft_ratio): 730, 150, 800;
    /// hard limitへ配分できる残り持ち時間の割合。単位は百分率である。
    /// 値はSPSAで調整した。
    HardRemainingShare(hard_remaining_share): 39, 10, 50;
    /// 次の反復の完了時刻を予測する、直前の反復の累積経過時間に対する倍率。単位は百分率である。
    /// 値はSPSAで調整した。
    IterationRatio(iteration_ratio): 277, 150, 400;
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
