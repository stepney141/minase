//! 対局の結果を目標とした損失を、対局ごとの和として書き出す使い捨ての集計。
//! 訓練の対局は10局面ごと、検証の対局は全局面を、エンジンの整数の評価で求める。
//! 使い方: outcome-gap <出力CSV> --net 名前=パス ... --data MNSD...
use minase::eval::Nnue;
use minase::eval::nnue::evaluate;
use minase_trainer::data::{Dataset, Outcome};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::num::NonZeroUsize;
use std::path::PathBuf;

const THREADS: usize = 12;

fn loss(cp: i32, k: f64, outcome: Outcome) -> f64 {
    let x = f64::from(cp) / k;
    let t = match outcome {
        Outcome::Win => 1.0,
        Outcome::Draw => 0.5,
        Outcome::Loss => 0.0,
    };
    x.max(0.0) - x * t + (-x.abs()).exp().ln_1p()
}

type Row = (u32, f64, u64);

fn run(dataset: &Dataset, net: &Nnue, indices: &[u64]) -> HashMap<u32, (f64, u64)> {
    let k = f64::from(net.cp_per_logit());
    let chunk = indices.len().div_ceil(THREADS).max(1);
    let parts: Vec<Vec<Row>> = std::thread::scope(|scope| {
        let handles: Vec<_> = indices
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    let mut rows = Vec::with_capacity(part.len());
                    for &index in part {
                        let sample = dataset.record(index).unwrap();
                        let position = sample.to_position().unwrap();
                        let cp = evaluate(net, &position);
                        rows.push((sample.game_number(), loss(cp, k, sample.outcome()), 1));
                    }
                    rows
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut games = HashMap::new();
    for (game, value, count) in parts.into_iter().flatten() {
        let entry = games.entry(game).or_insert((0.0, 0));
        entry.0 += value;
        entry.1 += count;
    }
    games
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let output = &args[1];
    let mut nets = Vec::new();
    let mut files = Vec::new();
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--net" => {
                let (name, path) = args[i + 1].split_once('=').unwrap();
                nets.push((name.to_owned(), Nnue::decode(&std::fs::read(path).unwrap()).unwrap()));
                i += 2;
            }
            "--data" => {
                files.extend(args[i + 1..].iter().map(PathBuf::from));
                break;
            }
            _ => panic!("unknown argument"),
        }
    }
    let mut text = String::from("net,file,game,split,count,sum_loss\n");
    for (file_index, file) in files.iter().enumerate() {
        let dataset = Dataset::open(std::slice::from_ref(file), None, NonZeroUsize::new(4).unwrap()).unwrap();
        let training: Vec<u64> = dataset.training_indices().iter().step_by(10).copied().collect();
        let validation: Vec<u64> = dataset.validation_indices().to_vec();
        for (name, net) in &nets {
            for (split, indices) in [("train", &training), ("validation", &validation)] {
                let mut rows: Vec<_> = run(&dataset, net, indices).into_iter().collect();
                rows.sort_unstable_by_key(|row| row.0);
                for (game, (sum, count)) in rows {
                    writeln!(text, "{name},{file_index},{game},{split},{count},{sum}").unwrap();
                }
            }
        }
        eprintln!("{}/{} {}", file_index + 1, files.len(), file.display());
    }
    std::fs::write(output, text).unwrap();
}
