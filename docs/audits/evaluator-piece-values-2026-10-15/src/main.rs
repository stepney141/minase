//! 手書き評価v0、単独PST（候補L）、PST＋FM（J75）のPST部分とFM込み、およびNNUEの駒除去差分を駒状態ごとに集計する。
//! 引数は、MNSD版2の教師ファイル、候補LのMNPT版2、局面の抽出間隔、および抽出する局面数の上限である。
//! J75とNNUEは、このcrateが依存するminaseに埋め込まれた`nets/pst.bin`と`nets/nnue.bin`を使う。
//! 結果はTSVで標準出力へ、抽出件数は標準エラーへ出す。

use std::{fs::File, io::BufReader};

use minase::eval::{
    PIECE_STATE_COUNT, handcrafted,
    nnue::{self, Nnue},
    piece_state_of,
    pst::{self, Pst},
};
use minase::training::records::Reader;
use minase_core::{Color, PieceKind, Position, PositionBuilder, Square};

const LIMIT: i32 = 28_999;
const FEATURES: usize = 13_680;
const NAMES: [&str; 29] = [
    "歩兵", "仲人", "香車", "反車", "横行", "竪行", "角行", "飛車", "龍馬", "龍王", "奔王", "王将",
    "醉象", "猛豹", "盲虎", "銅将", "銀将", "金将", "麒麟", "鳳凰", "獅子", "太子", "白駒", "鯨鯢",
    "飛牛", "奔猪", "飛鹿", "角鷹", "飛鷲",
];
const COLUMNS: usize = 5; // handcrafted, PST-L, PST(J75), PST+FM(J75), NNUE

/// MNPT版2（FM節なし）を読み、FMなしのPstを組み立てる。
fn read_v2(path: &str) -> Pst {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[0..4], b"MNPT");
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
    assert_eq!(bytes.len(), 80 + FEATURES * 4 + PIECE_STATE_COUNT * 4);
    let k = f32::from_le_bytes(bytes[12..16].try_into().unwrap());
    let rule_end = bytes[16..48].iter().position(|&b| b == 0).unwrap_or(32);
    let rule = std::str::from_utf8(&bytes[16..16 + rule_end]).unwrap();
    let body = &bytes[80..];
    let w = |endpoint: usize, f: usize| {
        let o = (endpoint * FEATURES + f) * 2;
        i16::from_le_bytes(body[o..o + 2].try_into().unwrap())
    };
    let weights: Vec<[i16; 2]> = (0..FEATURES).map(|f| [w(0, f), w(1, f)]).collect();
    let pv = std::array::from_fn(|s| {
        let o = FEATURES * 4 + s * 4;
        i32::from_le_bytes(body[o..o + 4].try_into().unwrap())
    });
    Pst::from_weights(&weights, pv, k, rule).unwrap()
}

fn remove_piece(position: &Position, removed: Square) -> Option<Position> {
    let mut builder = PositionBuilder::new(position.side_to_move());
    for square in Square::all() {
        if square != removed
            && let Some(piece) = position.piece_at(square)
        {
            builder.put(square, piece).ok()?;
        }
    }
    let mut after = builder.finish().ok()?;
    after.set_lion_capture(position.lion_capture_square()).ok()?;
    Some(after)
}

fn evals(p: &Position, owner: Color, l: &Pst, j: &Pst, n: &Nnue) -> [i32; COLUMNS] {
    let hc = handcrafted::evaluate(p);
    let hc = if p.side_to_move() == owner { hc } else { -hc };
    let (_, pl) = pst::evaluate_for_diagnostics(l, p, owner);
    let (jf, jp) = pst::evaluate_for_diagnostics(j, p, owner);
    let nn = nnue::evaluate_for_diagnostics(n, p, owner);
    [hc, pl, jp, jf, nn]
}

/// 自駒の行と相手駒の行の144升平均差の半分（ADR derive-search-piece-values の式）。
fn derived(p: &Pst, state: usize, endpoint: usize) -> f64 {
    let w = p.weights();
    let own: i64 = (0..144).map(|i| i64::from(w[state * 144 + i][endpoint])).sum();
    let enemy: i64 = (0..144).map(|i| i64::from(w[(47 + state) * 144 + i][endpoint])).sum();
    (own - enemy) as f64 / 2304.0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (data, l_path, stride, cap) = (&args[1], &args[2], args[3].parse::<usize>().unwrap(), args[4].parse::<usize>().unwrap());
    let l = read_v2(l_path);
    let j = pst::weights().unwrap();
    let n = nnue::weights().unwrap();

    // 状態ごとの差分列
    let mut diffs: Vec<[Vec<i32>; COLUMNS]> = (0..PIECE_STATE_COUNT).map(|_| Default::default()).collect();
    let mut kinds: Vec<Option<(PieceKind, bool)>> = vec![None; PIECE_STATE_COUNT];
    let mut reader = Reader::new(BufReader::new(File::open(data).unwrap())).unwrap();
    let (mut seen, mut used, mut clamped) = (0usize, 0usize, 0usize);
    'outer: while let Some(game) = reader.read_game().unwrap() {
        for p in game.positions() {
            seen += 1;
            if seen % stride != 0 {
                continue;
            }
            used += 1;
            for owner in Color::ALL {
                let before = evals(p, owner, &l, &j, &n);
                for sq in p.pieces_of(owner) {
                    let piece = p.piece_at(sq).unwrap();
                    let kind = piece.kind().unwrap();
                    if matches!(kind, PieceKind::King | PieceKind::CrownPrince) {
                        continue;
                    }
                    let Some(after) = remove_piece(p, sq) else { continue };
                    let a = evals(&after, owner, &l, &j, &n);
                    if before.iter().chain(a.iter()).any(|v| v.abs() >= LIMIT) {
                        clamped += 1;
                        continue;
                    }
                    let s = piece_state_of(piece);
                    kinds[s].get_or_insert((kind, piece.is_promoted()));
                    for c in 0..COLUMNS {
                        diffs[s][c].push(before[c] - a[c]);
                    }
                }
            }
            if used >= cap {
                break 'outer;
            }
        }
    }
    eprintln!("positions seen {seen}, used {used}, clamped removals skipped {clamped}");

    println!("state\tname\tcount\tv0_table\tstored_pst\tstored_nnue\tderivL_mg\tderivL_eg\tderivJ_mg\tderivJ_eg\tmean_hc\tmean_L\tmean_J\tmean_JFM\tmean_NNUE\tmed_hc\tmed_L\tmed_J\tmed_JFM\tmed_NNUE\tsd_NNUE");
    for s in 0..PIECE_STATE_COUNT {
        let Some((kind, promoted)) = kinds[s] else { continue };
        let cnt = diffs[s][0].len();
        let mut means = [0.0; COLUMNS];
        let mut meds = [0i32; COLUMNS];
        for c in 0..COLUMNS {
            let v = &mut diffs[s][c];
            means[c] = v.iter().map(|&x| x as f64).sum::<f64>() / cnt as f64;
            v.sort_unstable();
            meds[c] = v[cnt / 2];
        }
        let sd = (diffs[s][4].iter().map(|&x| (x as f64 - means[4]).powi(2)).sum::<f64>() / cnt as f64).sqrt();
        let label = format!(
            "{}{}",
            NAMES[kind.index()],
            if s >= 29 { "" } else if promoted { "(成駒)" } else { "(生・成れない)" }
        );
        println!(
            "{s}\t{label}\t{cnt}\t{}\t{}\t{}\t{:.0}\t{:.0}\t{:.0}\t{:.0}\t{:.0}\t{:.0}\t{:.0}\t{:.0}\t{:.0}\t{}\t{}\t{}\t{}\t{}\t{:.0}",
            handcrafted::piece_value(kind),
            j.piece_value_of_state(s),
            n.piece_value_of_state(s),
            derived(&l, s, 0), derived(&l, s, 1), derived(j.as_ref(), s, 0), derived(j.as_ref(), s, 1),
            means[0], means[1], means[2], means[3], means[4],
            meds[0], meds[1], meds[2], meds[3], meds[4], sd,
        );
    }
}
