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
    LmrDivisor(lmr_divisor): 100, 100, 400;
    /// LMRの減深量を1増減するbutterfly historyの閾値。単位は履歴点である。
    /// Stockfishに対応する値はなく、表の上限の変更に合わせて`round(9 × 7183 / 4384)`から15へ換算した。
    /// 開始値は設計書の換算による値であり、この値ではSPSAで未調整である。
    LmrHistoryThreshold(lmr_history_threshold): 15, 0, 2000;
    /// statScoreによってLMRの減深量を減らす倍率。単位は1/4096である。
    /// Stockfishは439（src/search.cpp:1381）、YaneuraOuは428（yaneuraou-search.cpp:3937）。
    /// 開始値はStockfishの値であり、この値ではSPSAで未調整である。
    LmrHistoryScale(lmr_history_scale): 439, 0, 1000;
    /// LMRの減深量の上限。単位は手である。
    /// StockfishとYaneuraOuには減深量の上限がなく、換算する元がないので、従来の定数と同じ3を既定値とする。
    /// 範囲の上限8は暫定の値である。SPSAの調整対象であり、この係数としての値は未調整である。
    /// 上限を設けた判断の根拠は`docs/adr/reduce-all-later-moves-and-root-moves-by-lmr-with-stat-score.md`にある。
    LmrMaxReduction(lmr_max_reduction): 3, 1, 8;
    /// LMRの基数。単位は1/100手である。
    /// Stockfishは948/1024で93（src/search.cpp:1919）、YaneuraOuは1133/1024で111。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrBase(lmr_base): 65, 0, 200;
    /// LMRの後半の加算量。単位は1/100手である。
    /// Stockfishは697/1024で68（src/search.cpp:1350）、YaneuraOuは691/1024で67。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrOffset(lmr_offset): 137, 0, 200;
    /// LMRの良化していない場合の表の値の増分。単位は百分率である。
    /// Stockfishは197/512で38（src/search.cpp:1919）、YaneuraOuは206/512で40。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrNonImproving(lmr_non_improving): 6, 0, 100;
    /// LMRのPVノードの減算量。単位は1/100手である。
    /// Stockfishは512/1024で50（src/search.cpp:1186-1187）、YaneuraOuは窓の幅とttPvに依存するため単一の対応値がない。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrPvReduction(lmr_pv_reduction): 2, 0, 200;
    /// LMRのttPvの前半の加算量。単位は1/100手である。
    /// Stockfishは929/1024で91（src/search.cpp:1191-1192）、YaneuraOuは1013/1024で99。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtPv(lmr_tt_pv): 39, 0, 200;
    /// LMRのttPvの後半の減算量の基数。単位は1/100手である。
    /// Stockfishは3023/1024で295（src/search.cpp:1346）、YaneuraOuは2819/1024で275。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtPvBase(lmr_tt_pv_base): 310, 0, 500;
    /// LMRのttPvかつPVノードの減算量。単位は1/100手である。
    /// Stockfishは1004/1024で98（src/search.cpp:1346）、YaneuraOuは973/1024で95。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtPvPv(lmr_tt_pv_pv): 29, 0, 200;
    /// LMRのttPvかつ置換表の値がαを超える場合の減算量。単位は1/100手である。
    /// Stockfishは885/1024で86（src/search.cpp:1346）、YaneuraOuは905/1024で88。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtPvValue(lmr_tt_pv_value): 121, 0, 200;
    /// LMRのttPvかつ置換表の深さが足りる場合の減算量。単位は1/100手である。
    /// Stockfishは816/1024で80（src/search.cpp:1347）、YaneuraOuは935/1024で91。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtPvDepth(lmr_tt_pv_depth): 47, 0, 200;
    /// LMRのttPvかつ置換表の深さが足りるcutNodeの減算量。単位は1/100手である。
    /// Stockfishは940/1024で92（src/search.cpp:1347）、YaneuraOuは959/1024で94。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtPvDepthCutNode(lmr_tt_pv_depth_cut_node): 158, 0, 200;
    /// LMRの手番号1つあたりの減算量。単位は1/100手である。
    /// Stockfishは65/1024で6（src/search.cpp:1352）、YaneuraOuは65/1024で6。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrMoveCount(lmr_move_count): 0, 0, 20;
    /// LMRのcutNodeの加算量。単位は1/100手である。
    /// Stockfishは4026/1024で393（src/search.cpp:1356-1357）、YaneuraOuは3611/1024で353。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrCutNode(lmr_cut_node): 260, 0, 600;
    /// LMRの置換表の手がないcutNodeの加算量。単位は1/100手である。
    /// Stockfishは933/1024で91（src/search.cpp:1356-1357）、YaneuraOuは985/1024で96。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrCutNodeNoTtMove(lmr_cut_node_no_tt_move): 96, 0, 200;
    /// LMRの置換表の手が捕獲手の場合の加算量。単位は1/100手である。
    /// Stockfishは1079/1024で105（src/search.cpp:1360-1361）、YaneuraOuは1054/1024で103。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtCapture(lmr_tt_capture): 18, 0, 250;
    /// LMRの子のβ打ち切り数が1より多い場合の加算量。単位は1/100手である。
    /// Stockfishは264/1024で26（src/search.cpp:1364-1365）、YaneuraOuは251/1024で25。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrCutoffCount(lmr_cutoff_count): 78, 0, 100;
    /// LMRの子のβ打ち切り数が2より多い場合の追加量。単位は1/100手である。
    /// Stockfishは1095/1024で107（src/search.cpp:1364-1365）、YaneuraOuは1124/1024で110。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrCutoffCountMany(lmr_cutoff_count_many): 205, 0, 250;
    /// LMRの子のβ打ち切り数が1より多いallNodeの追加量。単位は1/100手である。
    /// Stockfishは1138/1024で111（src/search.cpp:1364-1365）、YaneuraOuは1042/1024で102。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrCutoffCountAllNode(lmr_cutoff_count_all_node): 123, 0, 250;
    /// LMRの子のβ打ち切り数が1以下の置換表の手の減算量。単位は1/100手である。
    /// Stockfishは2179/1024で213（src/search.cpp:1368-1369）、YaneuraOuは2239/1024で219。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrTtMove(lmr_tt_move): 227, 0, 500;
    /// LMRのαと判定用の評価値の差が歩兵価値1枚分の場合の加算量。単位は1/100手である。
    /// Stockfishは3 × 208/1024で61（src/search.cpp:1383-1384）、YaneuraOuはこの項を持たず0。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrAlphaEval(lmr_alpha_eval): 10, 0, 150;
    /// LMRの分岐を通らず置換表の手がない場合の加算量。単位は1/100手である。
    /// Stockfishは1127/1024で110（src/search.cpp:1431-1432）、YaneuraOuは1057/1024で103。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrNoTtMove(lmr_no_tt_move): 105, 0, 250;
    /// LMRのallNodeで減深量に掛けて足す割合の分子。単位は百分率である。
    /// Stockfishは276/256で108（src/search.cpp:1387-1388）、YaneuraOuは273/256で107。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrAllNode(lmr_all_node): 158, 0, 200;
    /// LMRの再探索を1手深くするための余裕値。単位は歩兵価値に対する百分率である。
    /// Stockfishは53/208で25（src/search.cpp:1414）、YaneuraOuは48/90で53。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrDeeperMargin(lmr_deeper_margin): 92, 0, 100;
    /// LMRの再探索を1手浅くするための余裕値。単位は歩兵価値に対する百分率である。
    /// Stockfishは8/208で4（src/search.cpp:1415）、YaneuraOuは9/90で10。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmrShallowerMargin(lmr_shallower_margin): 27, 0, 40;
    /// Continuation historyによる枝刈りの、残り深さ1あたりの閾値。単位は履歴点である。
    /// Stockfishは4136（src/search.cpp:1234）、YaneuraOuは4097（yaneuraou-search.cpp:3653）。
    /// 開始値はStockfishの値であり、この値ではSPSAで未調整である。
    ContinuationPruning(continuation_pruning): 4136, 1000, 12000;
    /// 静かな手のfutility見積りに加える定数。単位は歩兵価値に対する百分率である。
    /// Stockfishは164/208で79（src/search.cpp:1242-1243）、YaneuraOuは42/90で47（yaneuraou-search.cpp:3663-3664）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    FutilityBase(futility_base): 106, 0, 400;
    /// 静かな手のfutility見積りでlmrDepth 1あたりに加える量。単位は歩兵価値に対する百分率である。
    /// Stockfishは119/208で57（src/search.cpp:1242-1243）、YaneuraOuは120/90で133（yaneuraou-search.cpp:3663-3664）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    FutilityLmrDepth(futility_lmr_depth): 219, 0, 300;
    /// 静的評価がαより大きいときにfutility見積りへ加える量。単位は歩兵価値に対する百分率である。
    /// Stockfishは90/208で43（src/search.cpp:1242-1243）、YaneuraOuは86/90で96（yaneuraou-search.cpp:3663-3664）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    FutilityAboveAlpha(futility_above_alpha): 132, -100, 300;
    /// 静かな手のSEEでlmrDepthの2乗あたりに許容する損。単位は歩兵価値に対する百分率である。
    /// Stockfishは23/208で11、YaneuraOuは25/90で28である（Stockfish src/search.cpp:1255-1258、YaneuraOu yaneuraou-search.cpp:3691-3697）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    QuietSeeMargin(quiet_see_margin): 48, 0, 100;
    /// late move pruningで数える静かな手の上限の切片。単位は1/100手である。
    /// 既定値はStockfishとYaneuraOuの`3 + d²`（Stockfish src/search.cpp:1199、YaneuraOu yaneuraou-search.cpp:3593）を換算した値であり、
    /// この式の形での値はSPSAで未調整である。
    LmpBase(lmp_base): 6436, 0, 8600;
    /// late move pruningの上限に残り深さの2乗を掛けて加える傾き。単位は残り深さの2乗あたり1/100手である。
    /// 既定値はStockfishとYaneuraOuの`3 + d²`（Stockfish src/search.cpp:1199、YaneuraOu yaneuraou-search.cpp:3593）を換算した値であり、
    /// この式の形での値はSPSAで未調整である。
    LmpSlope(lmp_slope): 346, 0, 3600;
    /// improvingでないときにlate move pruningの上限へ掛ける倍率。単位は百分率である。
    /// StockfishとYaneuraOuの`/(2 − improving)`（Stockfish src/search.cpp:1199、YaneuraOu yaneuraou-search.cpp:3593）に当たり、
    /// 両エンジンとも50に相当する。倍率を掛けない現行の式は100に相当する。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    LmpNonImproving(lmp_non_improving): 26, 25, 150;
    /// reverse futilityのmultの定数。単位は歩兵価値に対する百分率である。
    /// Stockfishは45/208で22（src/search.cpp:1025）、YaneuraOuは76/90で84である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityMultBase(reverse_futility_mult_base): 269, 0, 450;
    /// reverse futilityのmultの深さ1あたりの増分。単位は歩兵価値に対する百分率である。
    /// Stockfishは4/208で2（src/search.cpp:1025）、YaneuraOuは深さの項を持たず0である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityMultSlope(reverse_futility_mult_slope): 6, 0, 12;
    /// reverse futilityの置換表の照合に失敗した場合のmultの減算。単位は歩兵価値に対する百分率である。
    /// Stockfishは20/208で10（src/search.cpp:1026）、YaneuraOuは21/90で23である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityNoTtHit(reverse_futility_no_tt_hit): 0, 0, 50;
    /// reverse futilityのimprovingによる減算。単位はmultに対する百分率である。
    /// Stockfishは2789/1024で272（src/search.cpp:1029）、YaneuraOuは2686/1024で262である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityImproving(reverse_futility_improving): 163, 0, 400;
    /// reverse futilityのopponentWorseningによる減算。単位はmultに対する百分率である。
    /// Stockfishは335/1024で33（src/search.cpp:1029）、YaneuraOuは362/1024で35である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityOpponentWorsening(reverse_futility_opponent_worsening): 23, 0, 100;
    /// reverse futilityの補正による加算。単位は補正の値の絶対値に対する百分率である。
    /// Stockfishは131072/198435で66（src/search.cpp:1030）、YaneuraOuは131072/180600で73である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    ReverseFutilityCorrection(reverse_futility_correction): 91, 0, 200;
    /// razoring余裕値の残り深さ1あたりの量。単位は歩兵価値に対する百分率である。
    /// Stockfishは342/208で164（src/search.cpp:1017-1018）、YaneuraOuは`502 + 306 × depth²`で形が異なる
    /// （yaneuraou-search.cpp:3182-3191）。開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    RazoringDepthMargin(razoring_depth_margin): 2197, 0, 3000;
    /// 静的交換評価（SEE）で許容する損の残り深さ1あたりの余裕値。単位は歩兵価値に対する百分率である。
    /// Stockfishは177/208で85、YaneuraOuは167/90で186である（Stockfish src/search.cpp:1222、YaneuraOu yaneuraou-search.cpp:3631）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    SeeDepthMargin(see_depth_margin): 168, 0, 400;
    /// SEEの余裕値に加えるcapture historyの重み。単位は履歴値に対する1/1024である。
    /// StockfishとYaneuraOuはともに34である（Stockfish src/search.cpp:1222、YaneuraOu yaneuraou-search.cpp:3631）。
    /// 開始値はStockfishの値であり、SPSAで未調整である。
    SeeCaptureHistory(see_capture_history): 122, -50, 200;
    /// 捕獲手のfutility見積りに加える定数。単位は歩兵価値に対する百分率である。
    /// Stockfishは234/208で113、YaneuraOuは218/90で242である（Stockfish src/search.cpp:1213、YaneuraOu yaneuraou-search.cpp:3616）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    CaptureFutilityBase(capture_futility_base): 504, 0, 600;
    /// 捕獲手のfutility見積りでlmrDepth 1あたりに加える量。単位は歩兵価値に対する百分率である。
    /// Stockfishは247/208で119、YaneuraOuは223/90で248である（Stockfish src/search.cpp:1213、YaneuraOu yaneuraou-search.cpp:3616）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    CaptureFutilityLmrDepth(capture_futility_lmr_depth): 148, 0, 500;
    /// 捕獲手のfutility見積りに加えるcapture historyの重み。単位は履歴値に対する1/1024である。
    /// Stockfishは134、YaneuraOuは131である（Stockfish src/search.cpp:1214、YaneuraOu yaneuraou-search.cpp:3617）。
    /// 開始値はStockfishの値であり、SPSAで未調整である。
    CaptureFutilityHistory(capture_futility_history): 475, -100, 600;
    /// aspiration windowの初期半幅。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    AspirationDelta(aspiration_delta): 80, 10, 200;
    /// aspiration windowが次に外れた際の拡大量に掛ける倍率。単位は百分率である。
    /// 値はSPSAで調整した。
    AspirationGrowth(aspiration_growth): 198, 125, 400;
    /// null moveの基本減深量 `(切片 + 深さ × 傾き) / 1200` の切片。単位は1/1200手である。
    /// StockfishとYaneuraOuは、子の1手を除いて6手に相当する7200である
    /// （Stockfish src/search.cpp:1044、YaneuraOu yaneuraou-search.cpp:3252）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    NullMoveBase(null_move_base): 9985, 1200, 10000;
    /// null moveの基本減深量の傾き。単位は残り深さ1手あたり1/1200手である。
    /// StockfishとYaneuraOuは1/3手に相当する400である
    /// （Stockfish src/search.cpp:1044、YaneuraOu yaneuraou-search.cpp:3252）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    NullMoveSlope(null_move_slope): 474, 100, 600;
    /// 静的評価とβの差から求めるnull moveの追加減深の倍率。単位は百分率である。
    /// Stockfishの`(staticEval - beta) / 256`は163に相当し（src/search.cpp:1044）、YaneuraOuはこの項を持たず0である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    NullMoveEvalScale(null_move_eval_scale): 238, 0, 400;
    /// null moveを試す静的評価の余裕値の切片。単位は歩兵価値に対する百分率である。
    /// Stockfishは365/208で175（src/search.cpp:1037-1038）、YaneuraOuは378/90で420（yaneuraou-search.cpp:3236-3237）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    NullMoveEvalMargin(null_move_eval_margin): -847, -1000, 1000;
    /// null moveを試す余裕値から残り深さ1手ごとに引く量。単位は歩兵価値に対する百分率である。
    /// Stockfishは13/208で6（src/search.cpp:1037-1038）、YaneuraOuは16/90で18（yaneuraou-search.cpp:3236-3237）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    NullMoveEvalDepth(null_move_eval_depth): -11, -50, 100;
    /// improvingのときにnull moveを試す余裕値から引く量。単位は歩兵価値に対する百分率である。
    /// Stockfishは47/208で23（src/search.cpp:1037-1038）、YaneuraOuは53/90で59（yaneuraou-search.cpp:3236-3237）。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    NullMoveEvalImproving(null_move_eval_improving): 76, -200, 400;
    /// 同じ親の先行するnull move打ち切り1回あたり、静的評価へ加える量。単位は歩兵価値に対する百分率である。
    /// Stockfishは50/208で24（src/search.cpp:1037-1038）、YaneuraOuはこの項を持たず0である。
    /// 開始値はStockfishの値を換算した値であり、SPSAで未調整である。
    NullMoveFailHighBonus(null_move_fail_high_bonus): 24, 0, 200;
    /// Historyの加点の残り深さ1手あたりの増分。単位は履歴点である。
    /// Stockfishは133（src/search.cpp:2005）、YaneuraOuは128（yaneuraou-search.cpp:5309）。
    /// 開始値はStockfishの値であり、この値ではSPSAで未調整である。
    HistoryBonusSlope(history_bonus_slope): 133, 0, 400;
    /// Historyの加点の1次式から引く切片。単位は履歴点である。
    /// Stockfishは81（src/search.cpp:2005）、YaneuraOuは77（yaneuraou-search.cpp:5309）。
    /// 開始値はStockfishの値であり、この値ではSPSAで未調整である。
    HistoryBonusBase(history_bonus_base): 81, -100, 300;
    /// Historyの加点の上限。単位は履歴点である。
    /// Stockfishは1487（src/search.cpp:2005）、YaneuraOuは1529（yaneuraou-search.cpp:5309）。
    /// 開始値はStockfishの値であり、この値ではSPSAで未調整である。
    HistoryBonusCap(history_bonus_cap): 1487, 200, 7183;
    /// Historyの減点の残り深さ1手あたりの増分。単位は履歴点である。
    /// Stockfishは968（src/search.cpp:2006）、YaneuraOuは882（yaneuraou-search.cpp:5310）。
    /// 開始値はStockfishの値であり、この値ではSPSAで未調整である。
    HistoryMalusSlope(history_malus_slope): 968, 0, 2500;
    /// Historyの減点の1次式から引く切片。単位は履歴点である。
    /// Stockfishは235（src/search.cpp:2006）、YaneuraOuは204（yaneuraou-search.cpp:5310）。
    /// 開始値はStockfishの値であり、この値ではSPSAで未調整である。
    HistoryMalusBase(history_malus_base): 235, -200, 800;
    /// Historyの減点の上限。単位は履歴点である。
    /// Stockfishは2244（src/search.cpp:2006）、YaneuraOuは2122（yaneuraou-search.cpp:5310）。
    /// 開始値はStockfishの値であり、この値ではSPSAで未調整である。
    HistoryMalusCap(history_malus_cap): 2244, 200, 7183;
    /// 探索済みの静かな手1つごとに減点へ掛ける倍率。単位は1/1024である。
    /// Stockfishは921（src/search.cpp:2021）、YaneuraOuは977（yaneuraou-search.cpp:5327）。
    /// 開始値はStockfishの値であり、この値ではSPSAで未調整である。
    HistoryMalusDecay(history_malus_decay): 921, 700, 1024;
    /// 置換表の静かな打ち切り手へ与える、残り深さ1あたりの加点。単位は履歴点である。
    /// Stockfishは131（src/search.cpp:906）、YaneuraOuは傾き130（yaneuraou-search.cpp:2739-2740）。
    /// 開始値はStockfishの値であり、SPSAで未調整である。
    HistoryTtCutoffBonus(history_tt_cutoff_bonus): 131, 0, 300;
    /// 最善手が置換表の手ならhistoryの加点へ加える量。単位は履歴点である。
    /// Stockfishは364（src/search.cpp:2005）、YaneuraOuは353（yaneuraou-search.cpp:5309）。
    /// 開始値はStockfishの値であり、この値ではSPSAで未調整である。
    HistoryTtMoveBonus(history_tt_move_bonus): 364, 0, 1000;
    /// Fail lowで直前の静かな手のbutterfly historyへ与える加点の倍率。単位は1/32768である。
    /// Stockfishは215（src/search.cpp:1630）、YaneuraOuは235（yaneuraou-search.cpp:4336）。
    /// 開始値はStockfishの値であり、SPSAで未調整である。
    HistoryPriorBonus(history_prior_bonus): 215, 0, 600;
    /// 静的評価の変化によるbutterfly historyの更新倍率。
    /// Stockfishは11（src/search.cpp:1008）、YaneuraOuは10（yaneuraou-search.cpp:3129）。
    /// 開始値はStockfishの値であり、SPSAで未調整である。
    HistoryEvalDiff(history_eval_diff): 11, 0, 40;
    /// 新しい根の探索へ持ち越すbutterfly historyの割合。単位は百分率である。
    /// Stockfishの729/1024を71%へ換算した（src/search.cpp:343）。YaneuraOuは100%に当たる。
    /// 開始値は設計書の換算による値であり、この値ではSPSAで未調整である。
    HistoryDecay(history_decay): 71, 0, 100;
    /// 新しい根の探索へ持ち越す捕獲履歴の割合。単位は百分率である。
    /// SPSAの調整対象であり、この係数としての値は未調整である。
    CaptureHistoryDecay(capture_history_decay): 61, 0, 100;
    /// 整列キーへ加える捕獲履歴の換算値の絶対値の上限。単位は歩兵価値に対する百分率である。
    /// Stockfishの`10692 × 100 / (7 × 208)`を734%へ換算した（src/movepick.cpp:225-226）。YaneuraOuは1697%に当たる。
    /// 開始値は設計書の換算による値であり、この値ではSPSAで未調整である。
    CaptureHistoryScale(capture_history_scale): 734, 0, 2000;
    /// SEEが−75以上か判定不能である王手の、静かな手の並べ替え値への加点。
    /// Stockfishは16384（src/movepick.cpp:242）、YaneuraOuも16384（yaneuraou-movepick.cpp:371）である。
    /// 開始値はStockfishの値であり、SPSAで未調整である。
    OrderingCheckBonus(ordering_check_bonus): 16384, 0, 32768;
    /// 動かす駒が歩兵1枚分の価値を持つときの、価値の低い駒による脅威の項の大きさ。
    /// Stockfishは20 × 208で4160（src/movepick.cpp:246-247）。YaneuraOuはこの項を持たず0である。
    /// 開始値はStockfishの値であり、SPSAで未調整である。
    OrderingThreatWeight(ordering_threat_weight): 4160, 0, 12000;
    /// correction historyの補正値の絶対値の上限。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    CorrectionCap(correction_cap): 126, 50, 400;
    /// correction historyの更新重み `min(深さ, 8) × 係数 / 1024` の係数。単位は1/1024である。
    /// 値はSPSAで調整した。
    CorrectionWeight(correction_weight): 82, 8, 128;
    /// 静止探索のdelta pruningの余裕値。単位は歩兵価値に対する百分率である。
    /// 値はSPSAで調整した。
    DeltaMargin(delta_margin): 500, 50, 500;
    /// 静止探索で通常読む捕獲手数の上限。単位は手であり、例外は[`super::searcher::Searcher::quiesce`]に従う。
    /// 値はSPSAで調整した。
    QsearchMoveLimit(qsearch_move_limit): 6, 1, 7;
    /// 1局の開始から終局までに見込む両側の合計着手数。単位は手である。
    /// 値はSPSAで調整した。
    ExpectedPlies(expected_plies): 451, 250, 700;
    /// 時計の配分で見込む手番側の残り着手数の下限。単位は手である。
    /// 値はSPSAで調整した。
    MinMoves(min_moves): 42, 40, 200;
    /// 1手のsoft limitへ配分する加算時間の割合。単位は百分率である。
    /// 値はSPSAで調整した。
    IncrementShare(increment_share): 77, 30, 100;
    /// hard limitをsoft limitの基礎値の何倍までとするかを表す。単位は百分率である。
    /// 値はSPSAで調整した。
    HardSoftRatio(hard_soft_ratio): 631, 150, 800;
    /// hard limitへ配分できる残り持ち時間の割合。単位は百分率である。
    /// 値はSPSAで調整した。
    HardRemainingShare(hard_remaining_share): 37, 10, 50;
    /// 次の反復の完了時刻を予測する、直前の反復の累積経過時間に対する倍率。単位は百分率である。
    /// 値はSPSAで調整した。
    IterationRatio(iteration_ratio): 222, 150, 400;
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
