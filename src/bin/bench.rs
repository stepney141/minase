use std::time::Instant;

use clap::Parser;
use minase::core::rules::parse_rule_set;
use minase::search::{MAX_PLY, SearchConfig, search};
use minase::{Game, Rules, parse_sfen};

const DEFAULT_DEPTH: u32 = 3;

/// 固定局面探索ベンチのコマンドライン引数。
#[derive(Parser)]
#[command(name = "bench")]
struct Arguments {
    /// 全局面を探索する固定深さ。
    #[arg(long, default_value_t = DEFAULT_DEPTH, value_parser = parse_positive_u32)]
    depth: u32,
}

/// ベンチ対象の名前と2欄SFEN。
struct BenchPosition {
    name: &'static str,
    sfen: &'static str,
}

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

/// 探索ベンチを実行する。
fn main() {
    let arguments = Arguments::parse();
    let codes = parse_rule_set("engine-default").expect("engine-default preset must resolve");
    let rules = Rules::from_codes(&codes).expect("engine-default rules must be valid");
    let config = SearchConfig {
        depth: arguments.depth,
        nodes: None,
    };
    let start = Instant::now();
    let mut total_nodes = 0_u64;

    for bench_position in BENCH_POSITIONS {
        let position = parse_sfen(bench_position.sfen).expect("embedded SFEN must be valid");
        let game = Game::from_position(rules, position)
            .expect("embedded SFEN must form a game under engine-default rules");
        let legal_moves = game.legal_moves();
        assert!(
            !legal_moves.is_empty(),
            "bench position must have legal moves"
        );
        let position_start = Instant::now();
        let result = search(
            game.position(),
            rules,
            &legal_moves,
            game.search_key_history(),
            &config,
        );
        let elapsed = position_start.elapsed();
        total_nodes = total_nodes
            .checked_add(result.nodes)
            .expect("total node count overflow");
        println!(
            "position={} depth={} nodes={} elapsed={:.6}s",
            bench_position.name,
            result.depth,
            result.nodes,
            elapsed.as_secs_f64()
        );
    }

    let elapsed = start.elapsed();
    let nps = total_nodes as f64 / elapsed.as_secs_f64();
    println!(
        "summary: positions={} depth={} nodes={} elapsed={:.6}s nps={:.0}",
        BENCH_POSITIONS.len(),
        arguments.depth,
        total_nodes,
        elapsed.as_secs_f64(),
        nps
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_positions_are_valid_games_with_legal_moves() {
        let codes = parse_rule_set("engine-default").expect("engine-default preset must resolve");
        let rules = Rules::from_codes(&codes).expect("engine-default rules must be valid");

        assert!((10..=20).contains(&BENCH_POSITIONS.len()));
        for bench_position in BENCH_POSITIONS {
            let position = parse_sfen(bench_position.sfen).unwrap_or_else(|error| {
                panic!("{} has invalid SFEN: {error}", bench_position.name)
            });
            let game = Game::from_position(rules, position).unwrap_or_else(|error| {
                panic!("{} is not a valid game: {error}", bench_position.name)
            });
            assert!(
                !game.legal_moves().is_empty(),
                "{} must have legal moves",
                bench_position.name
            );
        }
    }

    #[test]
    fn arguments_reject_zero_depth() {
        assert!(Arguments::try_parse_from(["bench", "--depth", "0"]).is_err());
        assert!(Arguments::try_parse_from(["bench", "--depth", "257"]).is_err());
    }
}
