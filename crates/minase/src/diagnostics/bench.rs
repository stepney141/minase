//! 固定局面の探索で決定性と毎秒ノード数（NPS）を測る。
//!
//! 規則は`JCSA`に固定し、深さは`depth`で既定3、ワーカー数は`threads`で指定する。
//! `repetitions 1`では各局面の`position=<名前> depth=... nodes=... best=... score=... elapsed=...`、
//! `summary:`、および`accumulators:`の行を出す。
//! 2回以上では計測外のウォームアップを1回行い、反復ごとの`run=`行と最後の`median:`行を出す。
//! この場合も累算器の統計を反復ごとと全反復の合計について出す。
//! `search-stats`が有効な場合だけ探索統計の`stats:`行を加える。
//!
//! 置換表は1つを使い回し、局面ごとに計測外で消去する。NPSは各局面の探索時間の和から求める。
//! 同じビルドの`Threads=1`の固定深さでは、ノード数、最善手、および評価値は再実行間で一致する。
//! 特定の総ノード数は固定しない。`scripts/bench_compare.py`は局面ごとの深さ、ノード数、最善手、評価値を比較する。
//! 固定局面の共有は[`BENCH_POSITIONS`]に従う。

use std::fmt;
use std::io;
use std::num::{NonZeroU32, NonZeroUsize};
use std::time::Instant;

use crate::eval::Nnue;
use crate::eval::nnue::AccumulatorStats;
use crate::search::{MAX_PLY, SearchLimits, SearchSnapshot, TranspositionTable, search};
use minase_core::notation::usi;
use minase_core::{Game, MoveGenerator, Rules, parse_sfen};

/// benchの検査済み引数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arguments {
    depth: NonZeroU32,
    threads: NonZeroUsize,
    repetitions: NonZeroUsize,
}

/// benchの引数の範囲違反。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgumentError {
    /// 探索深さが上限を超えた。
    Depth,
    /// ワーカー数が256を超えた。
    Threads,
}

impl fmt::Display for ArgumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Depth => write!(f, "depth must be from 1 to {MAX_PLY}"),
            Self::Threads => f.write_str("threads must be from 1 to 256"),
        }
    }
}

impl std::error::Error for ArgumentError {}

impl Arguments {
    /// 探索深さとワーカー数を検査して構築する。各値は正数でなければならない。
    pub fn new(
        depth: NonZeroU32,
        threads: NonZeroUsize,
        repetitions: NonZeroUsize,
    ) -> Result<Self, ArgumentError> {
        if depth.get() > MAX_PLY {
            return Err(ArgumentError::Depth);
        }
        if threads.get() > 256 {
            return Err(ArgumentError::Threads);
        }
        Ok(Self {
            depth,
            threads,
            repetitions,
        })
    }
}

/// 省略時の探索深さ。
pub const DEFAULT_DEPTH: NonZeroU32 = const { NonZeroU32::new(3).unwrap() };

/// 固定局面の探索または結果の出力に失敗した原因。
#[derive(Debug)]
pub enum Error {
    /// 埋め込みの評価関数を読み込めなかった。
    Weights(crate::eval::nnue::Error),
    /// 固定局面の表記が不正だった。
    Position(minase_core::notation::sfen::SfenError),
    /// 探索の入力が不正だった。
    Search(crate::search::SearchError),
    /// 置換表を確保できなかった。
    Table(crate::search::TranspositionTableError),
    /// 探索で得た最善手を表記できなかった。
    BestMove(minase_core::IllegalMove),
    /// 総ノード数がu64に収まらなかった。
    CountOverflow,
    /// 反復ごとの測定結果を保存する領域を確保できなかった。
    Allocation(std::collections::TryReserveError),
    /// 出力先への書き込みに失敗した。
    Output(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Weights(error) => write!(f, "embedded evaluation weights are invalid: {error}"),
            Self::Position(error) => write!(f, "bench position is invalid: {error}"),
            Self::Search(error) => write!(f, "bench search failed: {error}"),
            Self::Table(error) => write!(f, "bench transposition table failed: {error}"),
            Self::BestMove(error) => write!(f, "bench best move is invalid: {error}"),
            Self::CountOverflow => f.write_str("bench total node count overflow"),
            Self::Output(error) => write!(f, "bench output failed: {error}"),
            Self::Allocation(error) => write!(f, "bench allocation failed: {error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Weights(error) => Some(error),
            Self::Position(error) => Some(error),
            Self::Search(error) => Some(error),
            Self::Table(error) => Some(error),
            Self::BestMove(error) => Some(error),
            Self::Output(error) => Some(error),
            Self::Allocation(error) => Some(error),
            Self::CountOverflow => None,
        }
    }
}

/// ベンチ対象の名前と2欄SFEN。
struct BenchPosition {
    /// 出力に使う局面名。
    name: &'static str,
    /// 局面の2欄基本形SFEN。
    sfen: &'static str,
}

/// 初期局面と中盤、終盤を含む固定15局面。
/// `minase_core::test_util::BENCH_SFENS`は試験が共有する写しとして同じ15局面を同じ順で持つ。
const BENCH_POSITIONS: &[BenchPosition] = &[
    BenchPosition {
        name: "initial",
        sfen: "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b",
    },
    // 第2局 Lqyj1bLC、81手中40手目。
    BenchPosition {
        name: "game-2-ply-40",
        sfen: "lfcsgekgsc1l/a1b1txot1bfa/mvrhd1d4r/1p1p1p2+INpp/p1pi2pp4/4p1n5/9P2/3IP2P4/PPPP1PP1POPP/MVR1D2D1RVM/A1B1T1XT1B1A/LFCSGKEGSCFL b",
    },
    // 第2局 Lqyj1bLC、81手中60手目。
    BenchPosition {
        name: "game-2-ply-60",
        sfen: "lfcsgekg1c1l/a1b1txo2s1a/mvrh1d6/1p1p1p2+IN+Op/p2i2ppB3/2p1p1n5/9P2/3IP2P4/PPPP1PP1P1PP/MVR1D2D1RVM/A3T1XT1B1A/LFCSGKEGSCFL b",
    },
    // 第3局 msxNDjN8、620手中310手目。
    BenchPosition {
        name: "game-3-ply-310",
        sfen: "1q1b2ko3l/1d1h1ett1b1a/1dm7r1/1+g1psn1xpmv1/2pisghcip2/3+p5fpp/6N5/2P3PH4/4R2TMV2/3T1DBDG3/2H2OXS3A/Q2SGKE1FC1L b",
    },
    // 第3局 msxNDjN8、620手中465手目。
    BenchPosition {
        name: "game-3-ply-465",
        sfen: "3+s1ok5/5ett4/12/3ps1x+bpm1+r/3i1g1cip+p1/4+b+pP5/4n2N4/7E4/12/12/4S+VD5/5KS1FC2 w",
    },
    // 第4局 hgNaEt5P、328手中164手目。
    BenchPosition {
        name: "game-4-ply-164",
        sfen: "2q1gekog3/4tddtsb2/fvrbs3h3/2mp1c1xp1v1/1ppip1ppi2m/5p6/3I5p2/1P2P1PPI3/1MPP4PN2/2VRDQD1+c3/2BHTOXR4/1FCSGKEGS1n1 b",
    },
    // 第4局 hgNaEt5P、328手中246手目。
    BenchPosition {
        name: "game-4-ply-246",
        sfen: "4gekog3/5d1ts3/1v1t8/2mp1c2p3/Xf1ipsppmx2/1pp9/3I8/1P2PpPN4/1MPP8/2V1OGHE3+d/R3T7/1FCSGK2S1+v+r b",
    },
    // 第5局 VoPpZxG5、341手中170手目。
    BenchPosition {
        name: "game-5-ply-170",
        sfen: "1hcs1ek3+P1/1f2gxgq4/m1rd1t2ton1/1v1pp3p1s1/2p3N5/1p3p2X3/3I4H3/1PP1PP2IQ2/L2P4P2+L/VC4D3V1/2F1TO1TC3/3SGKEGS1F1 b",
    },
    // 第5局 VoPpZxG5、341手中255手目。
    BenchPosition {
        name: "game-5-ply-255",
        sfen: "2cs1keh4/1f4g3o1/m2dtg4+H1/1v1pp7/11O/1pp9/3I1pQ4+V/1PPPPP2I3/L7P3/VC4+L5/2F1TEGT1F2/3SGK2S3 w",
    },
    // 第6局 OGH2sJc2、103手中51手目。
    BenchPosition {
        name: "game-6-ply-51",
        sfen: "l1c1gekgsc1l/a1bst2t1b1a/1vr1dq1dhrv1/mf1p1poxp1fm/pppih1ppippp/4pn6/1P1N8/P1PIPPPPIPPP/MF1PX1C1P3/2RHD1Q1HRVM/ACB1TODT1B1A/LV1SGKEGS1FL w",
    },
    // 第6局 OGH2sJc2、103手中77手目。
    BenchPosition {
        name: "game-6-ply-77",
        sfen: "l1c1gekgsc1l/3st2t1b1a/1vr1dq1dhrv1/a2p1poxp1fm/1p2h1ppippp/p4n6/1PFbf7/PN3PPPIPPP/MC1P1OC1P3/2RHD1Q1HRVM/A1B1T1DT1B1A/LV1SGKEGS1FL w",
    },
    // 第8局 mU1oGkUg、367手中183手目。
    BenchPosition {
        name: "game-8-ply-183",
        sfen: "l3g1k4l/a2stegt3a/m7d2m/p2+I+D+P+S4p/1F9h/10s1/P8f2/3P1PP2nc1/2R8P/4CTG1TF1A/A5XE1C2/L4K2Q2L w",
    },
    // 第8局 mU1oGkUg、367手中275手目。
    BenchPosition {
        name: "game-8-ply-275",
        sfen: "l5k4l/a2sge1t3a/12/p4+P5p/3+f3m2E1/1+I1+F5T2/P2C2P5/3P8/11P/11A/A3K7/L10L w",
    },
    // 第10局 2u7dwJf9、555手中277手目。
    BenchPosition {
        name: "game-10-ply-277",
        sfen: "+l+d4k4v/4exqhob2/1+rr5mc2/3tt5f1/5ppNp1p1/1m7p2/5P4P1/4O2Pi3/2+a3P3M1/3T2GDR2F/2S2GXEHV2/2Q3K1SC2 w",
    },
    // 第10局 2u7dwJf9、555手中416手目。
    BenchPosition {
        name: "game-10-ply-416",
        sfen: "4+l1k2+b1v/2+d3e5/6+m5/2+r9/8pNp1/5p3p2/6p3P1/2o9/5G6/6E4F/8+R1VC/6QKS3 b",
    },
];

/// bench 1回分の集計。
struct BenchRun {
    /// 全探索ワーカーの累算器更新・再計算の回数。
    accumulators: AccumulatorStats,
    /// 計測した全局面の探索統計。
    #[cfg(feature = "search-stats")]
    stats: crate::search::SearchStats,
    /// 全局面が到達した深さ。
    depth: u32,
    /// 全局面の総ノード数。
    nodes: u64,
    /// 各局面の探索時間の合計。
    elapsed: f64,
}

/// 全更新を分母とし、根の初期化も含む再計算率を出力する。
fn print_accumulator_stats(
    stats: AccumulatorStats,
    output: &mut dyn FnMut(&str) -> io::Result<()>,
) -> Result<(), Error> {
    let updates = stats.updates();
    let mut line = format!(
        "accumulators: accumulator_updates={updates} accumulator_refreshes={}",
        stats.refreshes
    );
    if updates != 0 {
        line.push_str(&format!(
            " refresh_fraction={:.6}",
            stats.refreshes as f64 / updates as f64
        ));
    }
    output(&line).map_err(Error::Output)
}

/// 回数と、分母が正の項目の比だけを1行に整形する。
#[cfg(feature = "search-stats")]
fn format_stats(stats: &crate::search::SearchStats) -> String {
    let mut line = stats.to_string();
    let nodes = stats.normal_nodes + stats.quiesce_nodes;
    let ranked = stats.best_move_rank_1
        + stats.best_move_rank_2
        + stats.best_move_rank_3
        + stats.best_move_rank_4_plus;
    for (name, numerator, denominator) in [
        ("normal_node_fraction", stats.normal_nodes, nodes),
        ("quiesce_node_fraction", stats.quiesce_nodes, nodes),
        ("tt_hit_rate", stats.tt_hits, stats.tt_probes),
        ("tt_cutoff_per_hit", stats.tt_cutoffs, stats.tt_hits),
        (
            "quiesce_tt_hit_rate",
            stats.quiesce_tt_hits,
            stats.quiesce_tt_probes,
        ),
        (
            "quiesce_tt_cutoff_per_hit",
            stats.quiesce_tt_cutoffs,
            stats.quiesce_tt_hits,
        ),
        (
            "first_move_beta_fraction",
            stats.first_move_beta_cutoffs,
            stats.beta_cutoffs,
        ),
        (
            "null_move_verification_failure_fraction",
            stats.null_move_verification_failures,
            stats.null_move_verifications,
        ),
        ("best_move_rank_1_fraction", stats.best_move_rank_1, ranked),
        ("best_move_rank_2_fraction", stats.best_move_rank_2, ranked),
        ("best_move_rank_3_fraction", stats.best_move_rank_3, ranked),
        (
            "best_move_rank_4_plus_fraction",
            stats.best_move_rank_4_plus,
            ranked,
        ),
    ] {
        if denominator != 0 {
            line.push_str(&format!(
                " {name}={:.6}",
                numerator as f64 / denominator as f64
            ));
        }
    }
    line
}

impl BenchRun {
    /// 1秒あたりの探索ノード数。
    fn nps(&self) -> f64 {
        self.nodes as f64 / self.elapsed
    }
}

/// 整数標本の中央値を返す。偶数件では中央2値の算術平均を切り捨てる。
fn median_u64(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        values[middle]
    } else {
        values[middle - 1] + (values[middle] - values[middle - 1]) / 2
    }
}

/// 浮動小数点標本の中央値を返す。
fn median_f64(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        values[middle]
    } else {
        values[middle - 1].midpoint(values[middle])
    }
}

/// 固定局面集を1回探索する。
fn run_bench(
    nnue: &Nnue,
    rules: Rules,
    limits: &SearchLimits,
    threads: NonZeroUsize,
    transposition_table: &mut TranspositionTable,
    print_positions: bool,
    output: &mut dyn FnMut(&str) -> io::Result<()>,
) -> Result<BenchRun, Error> {
    let mut histories = crate::search::HistoryTables::new(threads);
    let mut reached_depth = u32::MAX;
    let mut total_nodes = 0_u64;
    let mut total_elapsed = 0.0_f64;
    let mut accumulators = AccumulatorStats::default();
    // 前の探索や診断の計数を計測へ持ち込まない。
    nnue.take_accumulator_stats();
    #[cfg(feature = "search-stats")]
    let mut stats = crate::search::SearchStats::default();

    for bench_position in BENCH_POSITIONS {
        let position = parse_sfen(bench_position.sfen).map_err(Error::Position)?;
        let game = Game::from_position(rules, position);
        let snapshot = SearchSnapshot::from_game(&game).map_err(Error::Search)?;
        transposition_table.clear();
        histories.clear();
        let position_start = Instant::now();
        let result = search(
            nnue,
            &snapshot,
            limits,
            threads,
            transposition_table,
            &mut histories,
        )
        .map_err(Error::Search)?;
        let elapsed = position_start.elapsed();
        // searchは全補助ワーカーをjoinして戻る。主ワーカー分も計測外で回収する。
        accumulators += nnue.take_accumulator_stats();
        #[cfg(feature = "search-stats")]
        {
            stats += result.stats;
        }
        reached_depth = reached_depth.min(result.depth);
        total_elapsed += elapsed.as_secs_f64();
        total_nodes = total_nodes
            .checked_add(result.nodes)
            .ok_or(Error::CountOverflow)?;
        if print_positions {
            // 最善手と探索値は計測区間の外で文字列化する。局面別の一致比較に使う。
            let best_move = usi::text(
                game.position(),
                result.best_move,
                &MoveGenerator::new(rules.moves),
            )
            .map_err(Error::BestMove)?;
            output(&format!(
                "position={} depth={} nodes={} best={} score={} elapsed={:.6}s",
                bench_position.name,
                result.depth,
                result.nodes,
                best_move,
                result.score,
                elapsed.as_secs_f64()
            ))
            .map_err(Error::Output)?;
            #[cfg(feature = "search-stats")]
            output(&format_stats(&result.stats)).map_err(Error::Output)?;
        }
    }

    Ok(BenchRun {
        accumulators,
        #[cfg(feature = "search-stats")]
        stats,
        depth: reached_depth,
        nodes: total_nodes,
        elapsed: total_elapsed,
    })
}

/// 探索ベンチを実行する。
pub fn run(
    arguments: Arguments,
    output: &mut dyn FnMut(&str) -> io::Result<()>,
) -> Result<(), Error> {
    let nnue = crate::eval::weights().map_err(Error::Weights)?;
    let rules = Rules::JCSA;
    let limits =
        SearchLimits::new(Some(arguments.depth.get()), None, None, None).map_err(Error::Search)?;
    // 確保時のページフォルトとクリアのmemsetが計測へ混入しないよう、
    // 置換表は1個を使い回して局面ごとに計測外でクリアし(クリア後は
    // 新品と同一状態)、NPSは各局面の探索時間の合計から計算する。
    let mut transposition_table =
        TranspositionTable::new(crate::search::DEFAULT_TT_SIZE_MB).map_err(Error::Table)?;
    if arguments.repetitions.get() == 1 {
        let result = run_bench(
            &nnue,
            rules,
            &limits,
            arguments.threads,
            &mut transposition_table,
            true,
            output,
        )?;
        output(&format!(
            "summary: positions={} depth={} nodes={} elapsed={:.6}s nps={:.0}",
            BENCH_POSITIONS.len(),
            arguments.depth,
            result.nodes,
            result.elapsed,
            result.nps()
        ))
        .map_err(Error::Output)?;
        print_accumulator_stats(result.accumulators, output)?;
        #[cfg(feature = "search-stats")]
        output(&format_stats(&result.stats)).map_err(Error::Output)?;
        return Ok(());
    }

    let _ = run_bench(
        &nnue,
        rules,
        &limits,
        arguments.threads,
        &mut transposition_table,
        false,
        output,
    )?;
    let mut runs = Vec::new();
    runs.try_reserve_exact(arguments.repetitions.get())
        .map_err(Error::Allocation)?;
    for run in 1..=arguments.repetitions.get() {
        let result = run_bench(
            &nnue,
            rules,
            &limits,
            arguments.threads,
            &mut transposition_table,
            false,
            output,
        )?;
        output(&format!(
            "run={run} depth={} nodes={} elapsed={:.6}s nps={:.0}",
            result.depth,
            result.nodes,
            result.elapsed,
            result.nps()
        ))
        .map_err(Error::Output)?;
        print_accumulator_stats(result.accumulators, output)?;
        #[cfg(feature = "search-stats")]
        output(&format_stats(&result.stats)).map_err(Error::Output)?;
        runs.push(result);
    }
    let mut depths: Vec<_> = runs.iter().map(|run| u64::from(run.depth)).collect();
    let mut nodes: Vec<_> = runs.iter().map(|run| run.nodes).collect();
    let mut elapsed: Vec<_> = runs.iter().map(|run| run.elapsed).collect();
    let mut nps: Vec<_> = runs.iter().map(BenchRun::nps).collect();
    output(&format!(
        "median: depth={} nodes={} elapsed={:.6}s nps={:.0}",
        median_u64(&mut depths),
        median_u64(&mut nodes),
        median_f64(&mut elapsed),
        median_f64(&mut nps)
    ))
    .map_err(Error::Output)?;
    // 中央値ではなく、ウォームアップを除く計測反復の合計を使う。
    let mut accumulators = AccumulatorStats::default();
    for run in &runs {
        accumulators += run.accumulators;
    }
    print_accumulator_stats(accumulators, output)?;
    #[cfg(feature = "search-stats")]
    {
        let mut stats = crate::search::SearchStats::default();
        for run in &runs {
            stats += run.stats;
        }
        // 中央値の回数ではなく、ウォームアップを除く全計測反復の合計。
        output(&format!("{} scope=all_measured_runs", format_stats(&stats)))
            .map_err(Error::Output)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_accepts_only_one_through_max_ply() {
        for depth in [1, MAX_PLY] {
            assert!(
                Arguments::new(
                    NonZeroU32::new(depth).unwrap(),
                    NonZeroUsize::MIN,
                    NonZeroUsize::MIN
                )
                .is_ok()
            );
        }
        assert_eq!(
            Arguments::new(
                NonZeroU32::new(MAX_PLY + 1).unwrap(),
                NonZeroUsize::MIN,
                NonZeroUsize::MIN
            )
            .unwrap_err(),
            ArgumentError::Depth
        );
    }

    /// [`format_stats`]の契約に従い、分母0の比を省き、回数を残す。
    #[cfg(feature = "search-stats")]
    #[test]
    fn stats_omit_ratios_with_zero_denominators() {
        let mut stats = crate::search::SearchStats::default();
        let zero = format_stats(&stats);
        assert!(zero.starts_with("stats: "));
        assert!(zero.contains("tt_probes=0"));
        assert!(!zero.contains("_rate="));
        assert!(!zero.contains("_fraction="));
        assert!(!zero.contains("_per_hit="));
        stats.tt_probes = 4;
        stats.tt_hits = 2;
        stats.tt_cutoffs = 1;
        let nonzero = format_stats(&stats);
        assert!(nonzero.contains("tt_hit_rate=0.500000"));
        assert!(nonzero.contains("tt_cutoff_per_hit=0.500000"));
        assert!(!nonzero.contains("quiesce_tt_hit_rate="));
        stats.null_move_verifications = 4;
        stats.null_move_verification_failures = 1;
        assert!(format_stats(&stats).contains("null_move_verification_failure_fraction=0.250000"));
    }

    #[test]
    fn medians_are_computed_per_metric_for_odd_and_even_samples() {
        let mut odd_integers = [9, 1, 5];
        let mut even_integers = [10, 2, 6, 4];
        let mut odd_floats = [9.0, 1.0, 5.0];
        let mut even_floats = [10.0, 2.0, 6.0, 4.0];

        assert_eq!(median_u64(&mut odd_integers), 5);
        assert_eq!(median_u64(&mut even_integers), 5);
        assert_eq!(median_f64(&mut odd_floats), 5.0);
        assert_eq!(median_f64(&mut even_floats), 5.0);
    }

    // モジュールdocの再現性と[`BENCH_POSITIONS`]の固定局面を検査する。
    #[test]
    fn total_node_counts_are_reproducible_across_runs() {
        let rules = Rules::JCSA;
        let limits = SearchLimits::new(Some(1), None, None, None).unwrap();
        let nnue = crate::eval::weights().unwrap();
        // [`BENCH_POSITIONS`]の15局面を本番のbenchで解析し探索する。
        let mut transposition_table = TranspositionTable::new(1).unwrap();
        let mut run = || {
            run_bench(
                &nnue,
                rules,
                &limits,
                crate::search::DEFAULT_THREADS,
                &mut transposition_table,
                false,
                &mut |_| Ok(()),
            )
            .unwrap()
            .nodes
        };
        let first = run();
        let second = run();
        assert!(first > 0, "the bench search must count nodes");
        assert_eq!(first, second, "total node counts must be deterministic");
    }

    /// 全ワーカーの根の再計算を数え、集計を次の実行へ持ち越さないことを検査する。
    #[test]
    fn accumulator_counts_include_all_workers_and_exclude_previous_runs() {
        use crate::eval::nnue::{
            ActiveFeatureSet,
            feature_set::{FeatureSet, Mirror},
        };

        let nnue = Nnue::decode(include_bytes!("../../nets/nnue.bin")).unwrap();
        let rules = Rules::JCSA;
        let limits = SearchLimits::new(Some(1), None, None, None).unwrap();
        let mut table = TranspositionTable::new(1).unwrap();
        for count in [1, 2] {
            let threads = NonZeroUsize::new(count).unwrap();
            for _ in 0..2 {
                let run = run_bench(
                    &nnue,
                    rules,
                    &limits,
                    threads,
                    &mut table,
                    false,
                    &mut |_| Ok(()),
                )
                .unwrap();
                let roots = (BENCH_POSITIONS.len() * count) as u64;
                // P型では根だけを再計算し、HalfKP型では文脈の変更時にも再計算する。
                if ActiveFeatureSet::MIRROR == Mirror::Shared {
                    assert_eq!(run.accumulators.refreshes, roots);
                } else {
                    assert!(run.accumulators.refreshes >= roots);
                }
                assert!(run.accumulators.incremental_updates > 0);
                assert_eq!(nnue.take_accumulator_stats(), AccumulatorStats::default());
            }
        }
    }
}
