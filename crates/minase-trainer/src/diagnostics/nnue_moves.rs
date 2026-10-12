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

/// 全合法手の評価差診断の引数。
#[derive(Args)]
pub struct MovesArgs {
    /// 比較するMNUE版5。
    #[arg(long)]
    net: PathBuf,
    /// 基準とするMNPT版3。
    #[arg(long)]
    base: PathBuf,
    /// 全合法手を調べるMNSD局面。来歴ファイルは不要。
    #[arg(long)]
    positions: PathBuf,
    /// 新規作成するJSON報告。
    #[arg(long)]
    pub(super) output: PathBuf,
}

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

fn move_rows<const N: usize>(
    game: &Game,
    rules: Rules,
    evaluate: impl Fn(&Position, Color) -> [i32; N],
) -> Result<MoveRows<N>> {
    let position = game.position();
    let side = position.side_to_move();
    let before = evaluate(position, side);
    let generator = MoveGenerator::new(rules.moves);
    let mut rows = BTreeMap::new();
    for mv in game.legal_moves() {
        let text = usi::text(position, mv, &generator)?;
        let mut after = game.clone();
        // 終局する手もGameに適用し、着手後局面の静的評価を使う。
        after.play(mv)?;
        let same = evaluate(after.position(), side);
        let opposite = evaluate(after.position(), side.opposite());
        let components = std::array::from_fn(|i| Components::new(before[i], same[i], opposite[i]));
        if rows.insert(text, components).is_some() {
            return Err("duplicate legal move text".into());
        }
    }
    Ok(rows)
}

#[derive(Serialize)]
struct PositionReport {
    index: usize,
    matched_count: usize,
    candidate_only_count: usize,
    baseline_only_count: usize,
    candidate: DeltaStatistics,
    baseline: DeltaStatistics,
    candidate_minus_baseline: DeltaStatistics,
    candidate_minus_baseline_pst: DeltaStatistics,
}

fn position_report(
    index: usize,
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

#[derive(Serialize)]
pub(super) struct Report {
    net: FileIdentity,
    base: FileIdentity,
    positions: FileIdentity,
    position_count: usize,
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

pub(super) fn run(args: &MovesArgs) -> Result<Report> {
    let net = IntegerNetwork::read(&args.net)?;
    let baseline = read_baseline(&args.base)?;
    let mut reader = Reader::new(BufReader::new(File::open(&args.positions)?))?;
    let rules = Rules::from_codes(&parse_rule_set(reader.header().rule_set())?)?;
    if rules != Rules::ENGINE_DEFAULT {
        return Err("diagnostic positions must use engine-default rules".into());
    }
    if reader.header().record_count() == 0 {
        return Err("move probe positions must be nonempty".into());
    }
    let mut reports = Vec::new();
    while let Some(record) = reader.read_record()? {
        let game = Game::from_position(rules, record.to_position()?);
        let candidate = move_rows(
            &game,
            rules,
            |position, side| [net.evaluate(position, side)],
        )?;
        let base = move_rows(&game, rules, |position, side| {
            let (total, pst) = pst::evaluate_for_diagnostics(&baseline, position, side);
            [total, pst]
        })?;
        reports.push(position_report(reports.len(), &candidate, &base));
    }
    let candidate_only = reports.iter().map(|row| row.candidate_only_count).sum();
    let baseline_only = reports.iter().map(|row| row.baseline_only_count).sum();
    Ok(Report {
        net: file_identity(&args.net)?,
        base: file_identity(&args.base)?,
        positions: file_identity(&args.positions)?,
        position_count: reports.len(),
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
        let baseline = BTreeMap::from([("b".into(), [c, c]), ("c".into(), [c, c])]);
        let report = position_report(0, &candidate, &baseline);
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
        assert_eq!(
            unmatched_error(1, 1).as_deref(),
            Some("unmatched moves: candidate_only=1, baseline_only=1")
        );
        assert!(unmatched_error(0, 0).is_none());
    }
}
