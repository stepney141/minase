use crate::error::ContractError;
use std::{collections::BTreeMap, fs::File, io::BufReader, path::PathBuf};

use clap::Args;
use minase::{eval::pst, training::records::Reader};
use minase_core::{
    Color, Game, MoveGenerator, Position, Rules, notation::usi, rules::parse_rule_set,
};
use serde::Serialize;

use super::{
    Result,
    network::{IntegerNetwork, read_baseline},
    sample::{FileIdentity, file_identity},
    statistics::{DeltaStatistics, delta_statistics},
};

#[derive(Debug)]
enum Error {
    DuplicateMove(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateMove(text) => write!(f, "duplicate legal move text: {text}"),
        }
    }
}
impl std::error::Error for Error {}

/// 全合法手の評価差診断の引数。
#[derive(Args)]
pub struct MovesArgs {
    /// 比較するtatara形式のLayerStack。
    #[arg(long)]
    net: PathBuf,
    /// 基準とする、次元32のFMを持つMNPT版3。
    #[arg(long)]
    base: PathBuf,
    /// 全合法手を調べるMNSD局面。合成の由来も受理する。
    #[arg(long)]
    positions: PathBuf,
    /// 新規作成するJSON報告。
    #[arg(long)]
    pub(super) output: PathBuf,
}

/// 着手前Bと着手後B′の整数評価から作る、センチポーン単位の差分。
/// 着手前の手番側をtとし、`before = F_t(B)`、`same = F_t(B′)`、`opposite = F_{t̄}(B′)`とする。
/// `delta = −opposite − before`、`placement = same − before`、`turn = −(opposite + same)`であり、
/// `delta = placement + turn`が成り立つ。報告の差分統計には`delta`を使う。
/// LayerStackのFはPSQT差の半分と後段出力の和をfv_scaleで割った整数評価である。
/// PSQTは視点交換で符号が反転し、手番成分には後段の非反対称性と整数丸めが残る。
#[derive(Clone, Copy, Debug, PartialEq)]
struct Components {
    delta: i32,
    placement: i32,
    turn: i32,
}

impl Components {
    fn new(before: i32, same: i32, opposite: i32) -> Self {
        Self {
            delta: -opposite - before,
            placement: same - before,
            turn: -(opposite + same),
        }
    }

    fn delta(self) -> f64 {
        debug_assert_eq!(self.delta, self.placement + self.turn);
        f64::from(self.delta)
    }
}

type MoveRows<const N: usize> = BTreeMap<String, [Components; N]>;

/// 全合法手を実際に適用し、USI表記をキーとして[`Components`]を記録する。
/// 終局する着手は数だけを記録し、静的評価と差分統計から除外する。
/// 同じUSI表記が2回現れた場合はエラーにする。
/// 基準のPSTとFMを合わせた評価とPSTだけの評価は、2系列として同時に記録する。
fn move_rows<const N: usize>(
    game: &Game,
    rules: Rules,
    evaluate: impl Fn(&Position, Color) -> Result<[i32; N]>,
) -> Result<(MoveRows<N>, usize)> {
    let position = game.position();
    let side = position.side_to_move();
    let before = evaluate(position, side)?;
    let generator = MoveGenerator::new(rules.moves);
    let mut rows = BTreeMap::new();
    let mut terminal_count = 0;
    for mv in game.legal_moves() {
        let text = usi::text(position, mv, &generator)?;
        let mut after = game.clone();
        after.play(mv)?;
        if after.result().is_some() {
            terminal_count += 1;
            continue;
        }
        let same = evaluate(after.position(), side)?;
        let opposite = evaluate(after.position(), side.opposite())?;
        let components = std::array::from_fn(|i| Components::new(before[i], same[i], opposite[i]));
        if rows.insert(text.clone(), components).is_some() {
            return Err(Error::DuplicateMove(text).into());
        }
    }
    Ok((rows, terminal_count))
}

/// 入力順の0始まりの番号`index`で特定する局面ごとの報告。
/// `matched_count`は両方にあるUSI表記の合法手数、`candidate_only_count`と`baseline_only_count`は片側だけの合法手数である。
/// `terminal_count`は終局した着手数であり、差分の集計から除く。
/// `candidate`はNNUEの非終局の合法手の`delta`、`baseline`はPSTとFMを合わせた非終局の合法手の`delta`の[`DeltaStatistics`]である。
/// `candidate_minus_baseline`は共通の手の「NNUEの`delta` − PSTとFMを合わせた`delta`」、
/// `candidate_minus_baseline_pst`は共通の手の「NNUEの`delta` − PSTだけの`delta`」の同じ集計である。
#[derive(Serialize)]
struct PositionReport {
    index: usize,
    terminal_count: usize,
    matched_count: usize,
    candidate_only_count: usize,
    baseline_only_count: usize,
    candidate: DeltaStatistics,
    baseline: DeltaStatistics,
    candidate_minus_baseline: DeltaStatistics,
    candidate_minus_baseline_pst: DeltaStatistics,
}

/// USI表記で共通する手と片側だけの手を数え、[`PositionReport`]の規則で差分を集計する。
fn position_report(
    index: usize,
    terminal_count: usize,
    candidate: &MoveRows<1>,
    baseline: &MoveRows<2>,
) -> PositionReport {
    let mut differences = Vec::new();
    let mut pst_differences = Vec::new();
    for (text, row) in candidate {
        if let Some(base) = baseline.get(text) {
            differences.push(row[0].delta() - base[0].delta());
            pst_differences.push(row[0].delta() - base[1].delta());
        }
    }
    PositionReport {
        index,
        terminal_count,
        matched_count: differences.len(),
        candidate_only_count: candidate.len() - differences.len(),
        baseline_only_count: baseline.len() - differences.len(),
        candidate: delta_statistics(
            &candidate
                .values()
                .map(|row| row[0].delta())
                .collect::<Vec<_>>(),
        ),
        baseline: delta_statistics(
            &baseline
                .values()
                .map(|row| row[0].delta())
                .collect::<Vec<_>>(),
        ),
        candidate_minus_baseline: delta_statistics(&differences),
        candidate_minus_baseline_pst: delta_statistics(&pst_differences),
    }
}

/// `diag nnue-moves`の全局面の全合法手についてのJSON報告。
/// `net`、`base`、`positions`は各入力の[`FileIdentity`]、`position_count`は局面数である。
/// `terminal_count`は全局面で終局として記録した着手数の合計である。
/// `evaluation_perspective = "side_to_move_before_move"`、`standard_deviation_ddof = 0`を保存する。
/// `candidate_only_count`と`baseline_only_count`は片側だけの合法手の全局面での合計、
/// `by_position`は入力順の[`PositionReport`]の配列である。
/// 片側だけの合法手がどちらか1件以上あれば、`error`に
/// `"unmatched moves: candidate_only={件数}, baseline_only={件数}"`を持ち、それ以外は欄ごと省略する。
#[derive(Serialize)]
pub(super) struct Report {
    net: FileIdentity,
    base: FileIdentity,
    positions: FileIdentity,
    position_count: usize,
    terminal_count: usize,
    evaluation_perspective: &'static str,
    standard_deviation_ddof: u8,
    candidate_only_count: usize,
    baseline_only_count: usize,
    by_position: Vec<PositionReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
}

fn unmatched_error(candidate_only: usize, baseline_only: usize) -> Option<String> {
    (candidate_only != 0 || baseline_only != 0).then(|| {
        format!("unmatched moves: candidate_only={candidate_only}, baseline_only={baseline_only}")
    })
}

/// `--positions`の全局面の全合法手について、tatara形式のLayerStackとMNPT版3の着手前後の評価差を比べる。
/// 合成の由来を含むMNSDも受理する。
/// ヘッダの規則セットを解釈して`lishogi`との一致を要求し、不一致と記録数0を拒否する。
/// [`Reader::read_game`]で各対局を復元し、その最初の局面から[`Game`]を作って[`move_rows`]で各手を評価する。
pub(super) fn run(args: &MovesArgs) -> Result<Report> {
    let net = IntegerNetwork::read(&args.net)?;
    let baseline = read_baseline(&args.base)?;
    let mut reader = Reader::new(BufReader::new(File::open(&args.positions)?))?;
    let rules = Rules::from_codes(&parse_rule_set(reader.header().rule_set())?)?;
    if rules != Rules::LISHOGI {
        return Err(ContractError::NondefaultRules.into());
    }
    if reader.header().game_count() == 0 {
        return Err(ContractError::EmptyPositions.into());
    }
    let mut reports = Vec::new();
    while let Some(record) = reader.read_game()? {
        let game = Game::from_position(rules, record.positions()[0].clone());
        let (candidate, terminal_count) = move_rows(&game, rules, |position, side| {
            Ok([net.evaluate(position, side)?])
        })?;
        let (base, _) = move_rows(&game, rules, |position, side| {
            let (total, pst) = pst::evaluate_for_diagnostics(&baseline, position, side);
            Ok([total, pst])
        })?;
        reports.push(position_report(
            reports.len(),
            terminal_count,
            &candidate,
            &base,
        ));
    }
    let candidate_only = reports.iter().map(|row| row.candidate_only_count).sum();
    let baseline_only = reports.iter().map(|row| row.baseline_only_count).sum();
    Ok(Report {
        net: file_identity(&args.net)?,
        base: file_identity(&args.base)?,
        positions: file_identity(&args.positions)?,
        position_count: reports.len(),
        terminal_count: reports.iter().map(|row| row.terminal_count).sum(),
        evaluation_perspective: "side_to_move_before_move",
        standard_deviation_ddof: 0,
        candidate_only_count: candidate_only,
        baseline_only_count: baseline_only,
        by_position: reports,
        error: unmatched_error(candidate_only, baseline_only),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_moves_never_reach_the_static_evaluator() {
        use minase_core::{PieceCode, PieceKind, PositionBuilder, Square};
        use std::cell::Cell;
        let mut builder = PositionBuilder::new(Color::Black);
        for (file, rank, side, kind) in [
            (0, 0, Color::Black, PieceKind::King),
            (11, 11, Color::White, PieceKind::King),
            (11, 8, Color::Black, PieceKind::Rook),
        ] {
            builder
                .put(
                    Square::new(file, rank).unwrap(),
                    PieceCode::new(side, kind).unwrap(),
                )
                .unwrap();
        }
        let game = Game::from_position(Rules::LISHOGI, builder.finish().unwrap());
        let calls = Cell::new(0);
        let (rows, terminal) = move_rows(&game, Rules::LISHOGI, |position, _| {
            for side in Color::ALL {
                assert!(Square::all().any(|s| {
                    position.piece_at(s).is_some_and(|p| {
                        p.color() == Some(side) && p.kind() == Some(PieceKind::King)
                    })
                }));
            }
            calls.set(calls.get() + 1);
            Ok([0])
        })
        .unwrap();
        assert!(terminal > 0);
        assert_eq!(calls.get(), 1 + 2 * rows.len());
        assert_eq!(rows.len() + terminal, game.legal_moves().len());
    }

    #[test]
    fn components_preserve_placement_and_tempo_in_both_perspectives() {
        assert_eq!(
            Components::new(100, 175, -175),
            Components {
                delta: 75,
                placement: 75,
                turn: 0
            }
        );
        assert_eq!(
            Components::new(130, 130, -70),
            Components {
                delta: -60,
                placement: 0,
                turn: -60
            }
        );
        assert_eq!(
            Components::new(-70, -70, 130),
            Components {
                delta: -60,
                placement: 0,
                turn: -60
            }
        );
    }

    #[test]
    fn move_matching_counts_unmatched_rows_and_preserves_empty_statistics() {
        let c = Components::new(0, 0, -6);
        let candidate = BTreeMap::from([("a".into(), [c]), ("b".into(), [c])]);
        let base = Components::new(0, 0, -2);
        let pst = Components::new(0, 0, -9);
        let baseline = BTreeMap::from([("b".into(), [base, pst]), ("c".into(), [base, pst])]);
        let report = position_report(0, 0, &candidate, &baseline);
        assert_eq!(
            (
                report.matched_count,
                report.candidate_only_count,
                report.baseline_only_count
            ),
            (1, 1, 1)
        );
        assert_eq!(report.candidate.count, 2);
        assert_eq!(report.candidate_minus_baseline.count, 1);
        // 共通手の差は6−2=4、PSTだけとの差は6−9=−3。
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["candidate_minus_baseline"]["mean_cp"], 4.0);
        assert_eq!(json["candidate_minus_baseline_pst"]["mean_cp"], -3.0);
        let empty = serde_json::to_value(position_report(1, 0, &BTreeMap::new(), &BTreeMap::new()))
            .unwrap();
        for key in [
            "matched_count",
            "candidate_only_count",
            "baseline_only_count",
        ] {
            assert_eq!(empty[key], 0);
        }
        for key in [
            "candidate",
            "baseline",
            "candidate_minus_baseline",
            "candidate_minus_baseline_pst",
        ] {
            assert_eq!(
                empty[key],
                serde_json::json!({
                    "count": 0, "mean_cp": null, "std_cp": null,
                    "mean_absolute_cp": null, "max_absolute_cp": null,
                })
            );
        }
        assert_eq!(
            unmatched_error(1, 1).as_deref(),
            Some("unmatched moves: candidate_only=1, baseline_only=1")
        );
        assert!(unmatched_error(0, 0).is_none());
    }
}
