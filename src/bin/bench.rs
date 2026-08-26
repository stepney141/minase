//! 固定局面の探索ベンチ。ノード数の決定性検証とNPS計測に使う。

use std::num::NonZeroUsize;
use std::time::Instant;

use clap::Parser;
use minase::core::rules::parse_rule_set;
use minase::eval::Pst;
use minase::search::{
    DEFAULT_THREADS, MAX_PLY, SearchLimits, SearchSnapshot, TranspositionTable, search,
};
use minase::{Game, Rules, parse_sfen};

/// グローバルアロケータ。benchの実測（docs/plans/search.md 実施状況）に基づきmimallocを使う。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// 既定の探索深さ。
const DEFAULT_DEPTH: u32 = 3;

/// 固定局面探索ベンチのコマンドライン引数。
#[derive(Parser)]
#[command(name = "bench")]
struct Arguments {
    /// 全局面を探索する固定深さ。
    #[arg(long, default_value_t = DEFAULT_DEPTH, value_parser = parse_positive_u32)]
    depth: u32,
    /// 探索ワーカーの総数。
    #[arg(long, default_value_t = DEFAULT_THREADS, value_parser = parse_threads)]
    threads: NonZeroUsize,
    /// 計測する反復回数。2回以上では計測外のウォームアップを先に行う。
    #[arg(long, default_value = "1", value_parser = parse_repetitions)]
    repetitions: NonZeroUsize,
}

/// ベンチ対象の名前と2欄SFEN。
struct BenchPosition {
    /// 出力に使う局面名。
    name: &'static str,
    /// 局面の2欄基本形SFEN。
    sfen: &'static str,
}

/// ベンチ対象の局面集。初期局面と、ランダム対局から採った中盤・終盤局面。
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

/// 0より大きい探索深さを解析する。
fn parse_positive_u32(text: &str) -> Result<u32, String> {
    let depth = text
        .parse::<u32>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if depth == 0 {
        return Err("depth must be at least 1".to_owned());
    }
    if depth > MAX_PLY {
        return Err(format!("depth must not exceed {MAX_PLY}"));
    }
    Ok(depth)
}

/// 1以上256以下のワーカー数を解析する。
fn parse_threads(text: &str) -> Result<NonZeroUsize, String> {
    let threads = text
        .parse::<usize>()
        .map_err(|error| format!("invalid worker count '{text}': {error}"))?;
    NonZeroUsize::new(threads)
        .filter(|threads| threads.get() <= 256)
        .ok_or_else(|| "threads must be from 1 to 256".to_owned())
}

/// 1以上の反復回数を解析する。
fn parse_repetitions(text: &str) -> Result<NonZeroUsize, String> {
    let repetitions = text
        .parse::<usize>()
        .map_err(|error| format!("invalid repetition count '{text}': {error}"))?;
    NonZeroUsize::new(repetitions).ok_or_else(|| "repetitions must be at least 1".to_owned())
}

/// bench 1回分の集計。
struct BenchRun {
    /// 全局面が到達した深さ。
    depth: u32,
    /// 全局面の総ノード数。
    nodes: u64,
    /// 各局面の探索時間の合計。
    elapsed: f64,
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
    pst: &Pst,
    rules: Rules,
    limits: &SearchLimits,
    threads: NonZeroUsize,
    transposition_table: &mut TranspositionTable,
    print_positions: bool,
) -> BenchRun {
    let mut reached_depth = u32::MAX;
    let mut total_nodes = 0_u64;
    let mut total_elapsed = 0.0_f64;

    for bench_position in BENCH_POSITIONS {
        let position = parse_sfen(bench_position.sfen).expect("embedded SFEN must be valid");
        let game = Game::from_position(rules, position);
        let snapshot = SearchSnapshot::from_game(&game).expect("bench position must have moves");
        transposition_table.clear();
        let position_start = Instant::now();
        let result = search(pst, &snapshot, limits, threads, transposition_table)
            .expect("bench search input must be valid");
        let elapsed = position_start.elapsed();
        reached_depth = reached_depth.min(result.depth);
        total_elapsed += elapsed.as_secs_f64();
        total_nodes = total_nodes
            .checked_add(result.nodes)
            .expect("total node count overflow");
        if print_positions {
            println!(
                "position={} depth={} nodes={} elapsed={:.6}s",
                bench_position.name,
                result.depth,
                result.nodes,
                elapsed.as_secs_f64()
            );
        }
    }

    BenchRun {
        depth: reached_depth,
        nodes: total_nodes,
        elapsed: total_elapsed,
    }
}

/// 探索ベンチを実行する。
fn main() {
    let pst = minase::eval::weights().unwrap_or_else(|error| {
        eprintln!("error: embedded evaluation weights are invalid: {error}");
        std::process::exit(1);
    });
    let arguments = Arguments::parse();
    let codes = parse_rule_set("engine-default").expect("engine-default preset must resolve");
    let rules = Rules::from_codes(&codes).expect("engine-default rules must be valid");
    let limits = SearchLimits::new(Some(arguments.depth), None, None, None)
        .expect("the CLI parser accepts only valid search depths");
    // 確保時のページフォルトとクリアのmemsetが計測へ混入しないよう、
    // 置換表は1個を使い回して局面ごとに計測外でクリアし(クリア後は
    // 新品と同一状態)、NPSは各局面の探索時間の合計から計算する。
    let mut transposition_table = TranspositionTable::new(minase::search::DEFAULT_TT_SIZE_MB)
        .expect("default transposition table size must be valid");
    if arguments.repetitions.get() == 1 {
        let result = run_bench(
            &pst,
            rules,
            &limits,
            arguments.threads,
            &mut transposition_table,
            true,
        );
        println!(
            "summary: positions={} depth={} nodes={} elapsed={:.6}s nps={:.0}",
            BENCH_POSITIONS.len(),
            arguments.depth,
            result.nodes,
            result.elapsed,
            result.nps()
        );
        return;
    }

    let _ = run_bench(
        &pst,
        rules,
        &limits,
        arguments.threads,
        &mut transposition_table,
        false,
    );
    let mut runs = Vec::with_capacity(arguments.repetitions.get());
    for run in 1..=arguments.repetitions.get() {
        let result = run_bench(
            &pst,
            rules,
            &limits,
            arguments.threads,
            &mut transposition_table,
            false,
        );
        println!(
            "run={run} depth={} nodes={} elapsed={:.6}s nps={:.0}",
            result.depth,
            result.nodes,
            result.elapsed,
            result.nps()
        );
        runs.push(result);
    }
    let mut depths: Vec<_> = runs.iter().map(|run| u64::from(run.depth)).collect();
    let mut nodes: Vec<_> = runs.iter().map(|run| run.nodes).collect();
    let mut elapsed: Vec<_> = runs.iter().map(|run| run.elapsed).collect();
    let mut nps: Vec<_> = runs.iter().map(BenchRun::nps).collect();
    println!(
        "median: depth={} nodes={} elapsed={:.6}s nps={:.0}",
        median_u64(&mut depths),
        median_u64(&mut nodes),
        median_f64(&mut elapsed),
        median_f64(&mut nps)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_accept_thread_and_repetition_boundaries() {
        let minimum =
            Arguments::try_parse_from(["bench", "--threads", "1", "--repetitions", "1"]).unwrap();
        assert_eq!(minimum.threads.get(), 1);
        assert_eq!(minimum.repetitions.get(), 1);

        let maximum_threads =
            Arguments::try_parse_from(["bench", "--threads", "256", "--repetitions", "2"]).unwrap();
        assert_eq!(maximum_threads.threads.get(), 256);
        assert_eq!(maximum_threads.repetitions.get(), 2);
    }

    #[test]
    fn arguments_reject_invalid_threads_and_repetitions() {
        for value in ["0", "257", "not-a-number"] {
            assert!(
                Arguments::try_parse_from(["bench", "--threads", value]).is_err(),
                "threads={value} must be rejected"
            );
        }
        for value in ["0", "not-a-number"] {
            assert!(
                Arguments::try_parse_from(["bench", "--repetitions", value]).is_err(),
                "repetitions={value} must be rejected"
            );
        }
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

    // D8-BENCH-01(search.md bench節・実施状況フェーズ2): 固定局面集は初期局面1
    // ＋lishogiリプレイ7局から抽出した14局面の計15局面で確定している。全局面が
    // パース可能かつengine-default規則の下で合法な対局局面であり、合法手を持つ。
    #[test]
    fn bench_positions_are_the_fifteen_documented_valid_games() {
        let codes = parse_rule_set("engine-default").expect("engine-default preset must resolve");
        let rules = Rules::from_codes(&codes).expect("engine-default rules must be valid");

        // 局面の追加・変更は決定性アンカーを変えるため設計書の改定を伴う
        assert_eq!(BENCH_POSITIONS.len(), 15);
        for bench_position in BENCH_POSITIONS {
            // 各局面は独立に検証する(1局面の破損が他を隠さない)
            let position = parse_sfen(bench_position.sfen).unwrap_or_else(|error| {
                panic!("{} has invalid SFEN: {error}", bench_position.name)
            });
            let game = Game::from_position(rules, position);
            assert!(
                !game.legal_moves().is_empty(),
                "{} must have legal moves",
                bench_position.name
            );
        }
    }

    // D8-BENCH-02(search.md検証節): benchの総ノード数は再実行間で完全に一致する。
    // 特定の総ノード数(217,305など実施状況の値)は各時点の測定記録であり、
    // テストで固定してはならない。契約は「再実行間の一致」だけである。
    #[test]
    fn total_node_counts_are_reproducible_across_runs() {
        let codes = parse_rule_set("engine-default").expect("engine-default preset must resolve");
        let rules = Rules::from_codes(&codes).expect("engine-default rules must be valid");
        let limits = SearchLimits::new(Some(1), None, None, None).unwrap();
        let pst = minase::eval::weights().unwrap();
        let run = || {
            let mut transposition_table =
                TranspositionTable::new(minase::search::DEFAULT_TT_SIZE_MB)
                    .expect("default transposition table size must be valid");
            let mut total_nodes = 0_u64;
            for bench_position in BENCH_POSITIONS {
                let position =
                    parse_sfen(bench_position.sfen).expect("embedded SFEN must be valid");
                let game = Game::from_position(rules, position);
                let snapshot = SearchSnapshot::from_game(&game).unwrap();
                // 本体と同じく局面ごとに置換表をクリアして探索する
                transposition_table.clear();
                let result = search(
                    &pst,
                    &snapshot,
                    &limits,
                    DEFAULT_THREADS,
                    &mut transposition_table,
                )
                .unwrap();
                total_nodes += result.nodes;
            }
            total_nodes
        };
        let first = run();
        let second = run();
        assert!(first > 0, "the bench search must count nodes");
        assert_eq!(first, second, "total node counts must be deterministic");
    }
}
