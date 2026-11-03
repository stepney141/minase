//! 自己対局の結果へのロジスティック回帰で、線形の経路の序中盤の列を補正する。
//!
//! 対象は成っていない21駒種に当たる状態で、麒麟成りの獅子と鳳凰成りの奔王を含む。
//! 持ち主から見た段を1〜6段と7段以上の7区分、筋を中央からの距離0〜5の6区分にする。
//! 各区分の手番側の枚数から相手の枚数を引いた差をx、yとし、273個の係数a、bを求める。
//! 王駒を含む総駒数Nに対して`q = min(90, max(0, N−2))`と置き、補正は
//! `δ = (q/90)(Σ a x + Σ b y)`センチポーンとする。
//! 基点の整数評価eに対する予測は`p = σ((e+δ)/K_t + c_u)`である。
//! tは60〜79手から140〜159手までの5区分、uは手番、σはロジスティック関数を表す。
//! Kとcは訓練対局でδを0として最尤推定し、以後固定する。Kが50〜4000の端点か範囲外なら停止する。
//! 補正は`Σ_g mean_{s∈S_g} BCE(z(s),p(s)) + Σ θ²/(2τ²)`を最小化する。
//! BCEは二値交差エントロピーで、結果zは手番側の勝ち1、引き分け0.5、負け0である。
//!
//! MNSDの除外印がなく探索値を持つ60以上160未満の手数を、対局ごとに候補とする。
//! `h = hash64(seed ^ hash64(header_seed, game), 0)`とし、`(hash64(h, ply), ply)`順に
//! 基点を整数評価して`|e| < 3000`の局面を最大5個採る。選択後は手数順に並べ、対局の重みを1にする。
//! `is_validation(header_seed, game)`が真なら判定、それ以外でhが10の倍数ならτの選択、残りは訓練に用いる。
//! すべての集合を入力ファイル順、対局番号順、手数順に集約し、数値計算は1スレッドで行う。
//! τの選択には実数のδを用い、対局ごとの平均BCEを対局について平均する。同損失なら指定順で先のτを選ぶ。
//!
//! 選んだ係数を基底特徴の自駒の行へ`round_ties_even(8(a+b))`、相手駒の行へ
//! 持ち主の段を反転してその負値として足す。対象外の状態、終盤の列、先獅子、非線形部分は変えない。
//! 判定は書き出したバイト列を復号した整数評価e′を用いる。対局ごとの平均BCEをさらに対局について平均し、
//! `D = corrected_loss − base_loss`とする。再標本化は行わず、判定対局ごとの損失をJSONへ残す。

mod error;
mod solver;
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    thread,
};

use minase::{
    eval::{
        features::{FEATURE_COUNT, piece_state},
        nnue,
    },
    training::records::Origin,
};
use minase_core::{Color, Position, Square};
use serde::Serialize;
use serde_json::json;

use crate::{
    data::{Outcome, Sample, hash64, is_validation, stream::read_file},
    diagnostics::network::IntegerNetwork,
    identity::file_identity,
    nn::{atomic_write, hex, workers::workers},
};
pub use error::{Error, Result};
use solver::{Row, bce, newton};

const STATES: [usize; 21] = [
    10, 11, 20, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46,
];
const KINDS: [&str; 21] = [
    "FreeKing",
    "King",
    "Lion",
    "Pawn",
    "GoBetween",
    "Lance",
    "ReverseChariot",
    "SideMover",
    "VerticalMover",
    "Bishop",
    "Rook",
    "DragonHorse",
    "DragonKing",
    "DrunkElephant",
    "FerociousLeopard",
    "BlindTiger",
    "CopperGeneral",
    "SilverGeneral",
    "GoldGeneral",
    "Kirin",
    "Phoenix",
];
const RANK_PARAMETERS: usize = 21 * 7;
const PARAMETERS: usize = RANK_PARAMETERS + 21 * 6;

/// 結果回帰の入力と出力。並列度以外には既定値を持たない。
#[derive(Clone, Debug, clap::Args)]
pub struct Args {
    /// 補正の基点となるMNUEファイル。
    #[arg(long)]
    pub base: PathBuf,
    /// 乱数開始の自己対局を記録したMNSDファイル。入力順を保つ。
    #[arg(long, required = true, num_args = 1..)]
    pub data: Vec<PathBuf>,
    /// 正の有限値で重複のない事前分布の標準偏差、センチポーン。
    #[arg(long, required = true, num_args = 1.., allow_negative_numbers = true)]
    pub tau: Vec<f64>,
    /// 局面の選択と訓練・τの選択の分割に使うシード。
    #[arg(long)]
    pub seed: u64,
    /// 新規または空の出力ディレクトリ。
    #[arg(long)]
    pub output_dir: PathBuf,
    /// ファイル読み込みの並列度。省略時は物理コア数−2、最小1。
    #[arg(long)]
    pub threads: Option<usize>,
}

struct Features {
    counts: Vec<(usize, f64)>,
    q: f64,
}

impl Features {
    fn from_position(position: &Position) -> Self {
        let mut counts = [0_i16; PARAMETERS];
        for square in Square::all() {
            let Some(piece) = position.piece_at(square) else {
                continue;
            };
            let Ok(kind) = STATES.binary_search(&piece_state(piece)) else {
                continue;
            };
            let rank = if piece.color() == Some(Color::Black) {
                square.rank() + 1
            } else {
                12 - square.rank()
            };
            let sign = if piece.color() == Some(position.side_to_move()) {
                1
            } else {
                -1
            };
            counts[kind * 7 + usize::from(rank.min(7) - 1)] += sign;
            counts[RANK_PARAMETERS + kind * 6 + file_class(usize::from(square.file()))] += sign;
        }
        Self {
            counts: counts
                .into_iter()
                .enumerate()
                .filter(|&(_, x)| x != 0)
                .map(|(i, x)| (i, f64::from(x)))
                .collect(),
            q: f64::from(position.occupied().popcount().saturating_sub(2).min(90)),
        }
    }

    fn correction(&self, theta: &[f64]) -> f64 {
        self.q / 90.0 * self.counts.iter().map(|&(i, x)| theta[i] * x).sum::<f64>()
    }
}

fn file_class(file: usize) -> usize {
    if file < 6 { 5 - file } else { file - 6 }
}

struct SelectedPosition {
    sample: Sample,
    evaluation: i32,
    features: Features,
}

impl SelectedPosition {
    fn band(&self) -> usize {
        usize::from((self.sample.ply() - 60) / 20)
    }

    fn side(&self) -> usize {
        usize::from(self.sample.side_to_move() == Color::White)
    }

    fn target(&self) -> f64 {
        match self.sample.outcome() {
            Outcome::Loss => 0.0,
            Outcome::Draw => 0.5,
            Outcome::Win => 1.0,
        }
    }
}

struct SelectedGame {
    file: usize,
    game: u32,
    positions: Vec<SelectedPosition>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Split {
    Training,
    TauSelection,
    Decision,
}

fn game_hash(seed: u64, header_seed: u64, game: u32) -> u64 {
    hash64(seed ^ hash64(header_seed, game), 0)
}

fn split(seed: u64, header_seed: u64, game: u32) -> Split {
    if is_validation(header_seed, game) {
        Split::Decision
    } else if game_hash(seed, header_seed, game).is_multiple_of(10) {
        Split::TauSelection
    } else {
        Split::Training
    }
}

fn select_game(
    mut samples: Vec<Sample>,
    hash: u64,
    mut evaluate: impl FnMut(&Sample, &Position) -> i32,
) -> Result<Vec<SelectedPosition>> {
    samples.retain(|s| (60..160).contains(&s.ply()));
    samples.sort_unstable_by_key(|s| (hash64(hash, u32::from(s.ply())), s.ply()));
    let mut selected = Vec::new();
    for sample in samples {
        let position = sample.to_position()?;
        let evaluation = evaluate(&sample, &position);
        if evaluation <= -3000 || evaluation >= 3000 {
            continue;
        }
        selected.push(SelectedPosition {
            sample,
            evaluation,
            features: Features::from_position(&position),
        });
        if selected.len() == 5 {
            break;
        }
    }
    selected.sort_unstable_by_key(|s| s.sample.ply());
    Ok(selected)
}

struct LoadedFile {
    header_seed: u64,
    games: Vec<SelectedGame>,
    identity: serde_json::Value,
}

fn load_file(path: &Path, file: usize, seed: u64, base: &[u8]) -> Result<LoadedFile> {
    let mut candidates: BTreeMap<u32, Vec<Sample>> = BTreeMap::new();
    let summary = read_file(path, None, &mut |item| {
        if (60..160).contains(&item.sample.ply()) {
            candidates
                .entry(item.sample.game_number())
                .or_default()
                .push(item.sample);
        }
        Ok(())
    })?;
    if summary.origin != Some(Origin::RandomSelfplay) {
        return Err(Error::NonRandomSelfplay {
            path: path.to_owned(),
            origin: summary.origin,
        });
    }
    let network = IntegerNetwork::decode(base)?;
    let header_seed = summary.header.seed();
    let mut games = Vec::new();
    for (game, samples) in candidates {
        let positions = select_game(
            samples,
            game_hash(seed, header_seed, game),
            |_, position| network.evaluate(position, position.side_to_move()),
        )?;
        if !positions.is_empty() {
            games.push(SelectedGame {
                file,
                game,
                positions,
            });
        }
    }
    let selected_positions: usize = games.iter().map(|g| g.positions.len()).sum();
    let mut identity = serde_json::to_value(file_identity(path)?)?;
    identity["games"] = json!(summary.header.game_count());
    identity["selected_games"] = json!(games.len());
    identity["selected_positions"] = json!(selected_positions);
    Ok(LoadedFile {
        header_seed,
        games,
        identity,
    })
}

#[derive(Default)]
struct Dataset {
    training: Vec<SelectedGame>,
    tau_selection: Vec<SelectedGame>,
    decision: Vec<SelectedGame>,
    files: Vec<serde_json::Value>,
}

fn load(args: &Args, base: &[u8], threads: usize) -> Result<Dataset> {
    let mut paths = Vec::new();
    let mut unique = HashSet::new();
    for path in &args.data {
        let path = path.canonicalize()?;
        if !unique.insert(path.clone()) {
            return Err(Error::DuplicatePath { path });
        }
        paths.push(path);
    }
    let mut seeds = HashSet::new();
    let mut dataset = Dataset::default();
    for (chunk_index, paths) in paths.chunks(threads).enumerate() {
        let loaded = thread::scope(|scope| {
            let handles: Vec<_> = paths
                .iter()
                .enumerate()
                .map(|(index, path)| {
                    let file = chunk_index * threads + index;
                    thread::Builder::new()
                        .spawn_scoped(scope, move || load_file(path, file, args.seed, base))
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .map_err(Error::Io)?
                        .join()
                        .map_err(|_| Error::WorkerPanicked)?
                })
                .collect::<Vec<_>>()
        });
        // 失敗しても起動した全スレッドを回収し、結果とエラーを入力順に扱う。
        for file in loaded {
            let file = file?;
            if !seeds.insert(file.header_seed) {
                return Err(Error::DuplicateSeed {
                    seed: file.header_seed,
                });
            }
            for game in file.games {
                match split(args.seed, file.header_seed, game.game) {
                    Split::Training => &mut dataset.training,
                    Split::TauSelection => &mut dataset.tau_selection,
                    Split::Decision => &mut dataset.decision,
                }
                .push(game);
            }
            dataset.files.push(file.identity);
        }
    }
    for (name, games) in [
        ("training", &dataset.training),
        ("tau_selection", &dataset.tau_selection),
        ("decision", &dataset.decision),
    ] {
        if games.is_empty() {
            return Err(Error::EmptySplit { split: name });
        }
    }
    Ok(dataset)
}

#[derive(Serialize)]
struct SideIntercept {
    black: f64,
    white: f64,
}

#[derive(Serialize)]
struct Calibration {
    cp_per_logit: [f64; 5],
    side_intercept: SideIntercept,
}

impl Calibration {
    fn logit(&self, position: &SelectedPosition, evaluation: f64) -> f64 {
        evaluation / self.cp_per_logit[position.band()]
            + match position.sample.side_to_move() {
                Color::Black => self.side_intercept.black,
                Color::White => self.side_intercept.white,
            }
    }
}

fn calibrate(games: &[SelectedGame]) -> Result<Calibration> {
    let rows: Vec<_> = games
        .iter()
        .flat_map(|g| {
            g.positions.iter().map(|p| Row {
                terms: vec![
                    (p.band(), f64::from(p.evaluation) / 1000.0),
                    (5 + p.side(), 1.0),
                ],
                offset: 0.0,
                target: p.target(),
                weight: 1.0 / g.positions.len() as f64,
            })
        })
        .collect();
    let theta = newton(&rows, 7, 0.0, "calibration")?;
    let mut cp_per_logit = [0.0; 5];
    for (band, k) in cp_per_logit.iter_mut().enumerate() {
        let beta = theta[band] / 1000.0;
        *k = 1.0 / beta;
        // data/teacher.rsの探索範囲と同じ値。設計書に従い端点も拒否する。
        if beta <= 0.0 || !k.is_finite() || *k <= 50.0 || *k >= 4000.0 {
            return Err(Error::CalibrationOutOfRange {
                band,
                beta,
                cp_per_logit: *k,
            });
        }
    }
    Ok(Calibration {
        cp_per_logit,
        side_intercept: SideIntercept {
            black: theta[5],
            white: theta[6],
        },
    })
}

fn fit(games: &[SelectedGame], calibration: &Calibration, tau: f64) -> Result<Vec<f64>> {
    let precision = (1.0 / tau) / tau;
    if !precision.is_finite() || precision <= 0.0 {
        return Err(Error::UnrepresentablePrecision { tau });
    }
    let rows: Vec<_> = games
        .iter()
        .flat_map(|g| {
            g.positions.iter().map(|p| {
                let scale = p.features.q / (90.0 * calibration.cp_per_logit[p.band()]);
                Row {
                    terms: p
                        .features
                        .counts
                        .iter()
                        .map(|&(i, x)| (i, x * scale))
                        .collect(),
                    offset: calibration.logit(p, f64::from(p.evaluation)),
                    target: p.target(),
                    weight: 1.0 / g.positions.len() as f64,
                }
            })
        })
        .collect();
    newton(&rows, PARAMETERS, precision, "correction")
}

fn mean_loss(games: &[SelectedGame], calibration: &Calibration, theta: &[f64]) -> f64 {
    games
        .iter()
        .map(|g| {
            g.positions
                .iter()
                .map(|p| {
                    bce(
                        calibration
                            .logit(p, f64::from(p.evaluation) + p.features.correction(theta)),
                        p.target(),
                    )
                })
                .sum::<f64>()
                / g.positions.len() as f64
        })
        .sum::<f64>()
        / games.len() as f64
}

fn corrections(theta: &[f64]) -> Result<Vec<i16>> {
    let mut result = vec![0; FEATURE_COUNT];
    for (kind, &state) in STATES.iter().enumerate() {
        for relative_color in 0..2 {
            for rank in 0..12 {
                let owner_rank = if relative_color == 0 {
                    rank + 1
                } else {
                    12 - rank
                };
                for file in 0..12 {
                    let feature = (relative_color * 47 + state) * 144 + rank * 12 + file;
                    let sum = theta[kind * 7 + owner_rank.min(7) - 1]
                        + theta[RANK_PARAMETERS + kind * 6 + file_class(file)];
                    let value = (8.0 * sum).round_ties_even()
                        * if relative_color == 0 { 1.0 } else { -1.0 };
                    if !value.is_finite()
                        || !(f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&value)
                    {
                        return Err(Error::CorrectionOutOfRange { feature, value });
                    }
                    result[feature] = value as i16;
                }
            }
        }
    }
    Ok(result)
}

#[derive(Serialize)]
struct GameLoss {
    file: usize,
    game: u32,
    positions: usize,
    base_loss: f64,
    corrected_loss: f64,
}

#[derive(Serialize)]
struct Decision {
    base_loss: f64,
    corrected_loss: f64,
    d: f64,
    games: Vec<GameLoss>,
}

fn decide(
    games: &[SelectedGame],
    calibration: &Calibration,
    network: &IntegerNetwork,
) -> Result<Decision> {
    let mut losses = Vec::new();
    for game in games {
        let mut base_loss = 0.0;
        let mut corrected_loss = 0.0;
        for p in &game.positions {
            let position = p.sample.to_position()?;
            let corrected = network.evaluate(&position, position.side_to_move());
            base_loss += bce(calibration.logit(p, f64::from(p.evaluation)), p.target());
            corrected_loss += bce(calibration.logit(p, f64::from(corrected)), p.target());
        }
        let positions = game.positions.len();
        losses.push(GameLoss {
            file: game.file,
            game: game.game,
            positions,
            base_loss: base_loss / positions as f64,
            corrected_loss: corrected_loss / positions as f64,
        });
    }
    let base_loss = losses.iter().map(|g| g.base_loss).sum::<f64>() / losses.len() as f64;
    let corrected_loss = losses.iter().map(|g| g.corrected_loss).sum::<f64>() / losses.len() as f64;
    if !base_loss.is_finite() || !corrected_loss.is_finite() {
        return Err(Error::NonFinite { stage: "decision" });
    }
    Ok(Decision {
        base_loss,
        corrected_loss,
        d: corrected_loss - base_loss,
        games: losses,
    })
}

fn validate_args(args: &Args) -> Result<()> {
    if args.data.is_empty() {
        return Err(Error::MissingData);
    }
    if args.tau.is_empty() {
        return Err(Error::MissingTaus);
    }
    for (index, &value) in args.tau.iter().enumerate() {
        if !value.is_finite() || value <= 0.0 {
            return Err(Error::InvalidTau { index, value });
        }
        if args.tau[..index].contains(&value) {
            return Err(Error::DuplicateTau { value });
        }
    }
    if args.output_dir.exists() && fs::read_dir(&args.output_dir)?.next().is_some() {
        return Err(Error::NonEmptyOutput {
            path: args.output_dir.clone(),
        });
    }
    Ok(())
}

/// 基点と自己対局を検査し、補正した`nnue.bin`と判定を含む`summary.json`を書き出す。
/// 入力と計算の失敗は原因別に返す。並列読み込み後の集約順と数値演算順は固定する。
pub fn run(args: Args) -> Result<()> {
    validate_args(&args)?;
    let threads = workers(args.threads).map_err(Error::Workers)?;
    let base = fs::read(&args.base)?;
    IntegerNetwork::decode(&base)?;
    let base_identity = serde_json::to_value(file_identity(&args.base)?)?;
    let dataset = load(&args, &base, threads)?;
    let calibration = calibrate(&dataset.training)?;
    let mut tau_losses = Vec::new();
    let mut best: Option<(f64, f64, Vec<f64>)> = None;
    for &tau in &args.tau {
        let theta = fit(&dataset.training, &calibration, tau)?;
        let loss = mean_loss(&dataset.tau_selection, &calibration, &theta);
        if !loss.is_finite() {
            return Err(Error::NonFinite {
                stage: "tau selection",
            });
        }
        tau_losses.push(json!({"tau": tau, "loss": loss}));
        if best
            .as_ref()
            .is_none_or(|(_, best_loss, _)| loss < *best_loss)
        {
            best = Some((tau, loss, theta));
        }
    }
    let (selected_tau, _, theta) = best.ok_or(Error::MissingTaus)?;
    let written = nnue::add_to_linear_midgame(&base, &corrections(&theta)?)?;
    let network = IntegerNetwork::decode(&written)?;
    let decision = decide(&dataset.decision, &calibration, &network)?;
    let counts = |positions: bool| {
        let count = |games: &[SelectedGame]| {
            if positions {
                games.iter().map(|g| g.positions.len()).sum()
            } else {
                games.len()
            }
        };
        json!({"training": count(&dataset.training), "tau_selection": count(&dataset.tau_selection), "decision": count(&dataset.decision)})
    };
    fs::create_dir_all(&args.output_dir)?;
    let output = args.output_dir.canonicalize()?.join("nnue.bin");
    let output_path = serde_json::to_value(&output)?;
    let summary = json!({
        "base": base_identity, "data": dataset.files, "seed": args.seed, "taus": args.tau,
        "games": counts(false), "positions": counts(true), "calibration": calibration,
        "tau_losses": tau_losses, "tau_selection_base_loss": mean_loss(&dataset.tau_selection, &calibration, &[0.0; PARAMETERS]),
        "selected_tau": selected_tau, "kinds": KINDS,
        "rank_coefficients": theta[..RANK_PARAMETERS].as_chunks::<7>().0,
        "file_coefficients": theta[RANK_PARAMETERS..].as_chunks::<6>().0,
        "decision": decision, "output": {"path": output_path, "checksum": hex(&written[144..176])},
    });
    let summary_bytes = serde_json::to_vec_pretty(&summary)?;
    atomic_write(&output, &written)?;
    atomic_write(&args.output_dir.join("summary.json"), &summary_bytes)?;
    println!(
        "tau={selected_tau}, D={}, parameters={PARAMETERS}",
        decision.d
    );
    Ok(())
}
