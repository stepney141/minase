//! 自己対局1局の進行とランダム着手の注入。

use super::{
    INJECTION_WINDOW,
    opening::{Opening, starting_game},
};
use minase::datagen::game::{CompletedGame, CompletedRecord};
use minase::datagen::statistics::Statistics;
use minase::datagen::{MATE_BAND_START, current_position_is_repeated, invalid_data};
use minase::eval::Pst;
use minase::rng::{XorShift64, derive_seed};
use minase::search::{DEFAULT_THREADS, SearchLimits, SearchSnapshot, TranspositionTable, search};
use minase::training::records::{Outcome, Record, best_move_is_tactical};
use minase::{MoveGenerator, Position, Rules};
use std::{
    io,
    num::{NonZeroU64, NonZeroUsize},
};

/// 1局の生成に共通する探索と注入の設定。
#[derive(Clone, Copy)]
pub(super) struct PlaySettings {
    /// 対局シードの派生元。
    pub(super) base_seed: u64,
    /// 1探索のノード上限。
    pub(super) nodes: u32,
    /// 1局に注入するランダム着手の上限回数。
    pub(super) random_moves: u8,
    /// 1局の手数上限。
    pub(super) max_ply: u16,
}

/// 終局後に結果を付ける記録候補。
#[derive(Clone, PartialEq, Eq, Debug)]
struct Candidate {
    /// 記録時点の局面。
    position: Position,
    /// 手番側視点の探索値。
    score: i16,
    /// 記録時点の手数。
    ply: u16,
    /// 記録時点の探索キー。
    search_key: u64,
}

/// 1局のランダム着手予定と記録開始手数。
#[derive(Clone, PartialEq, Eq, Debug)]
struct InjectionPlan {
    /// 注入する対局開始時からの手数。昇順で重複しない。
    plies: Vec<u32>,
    /// 記録対象とする最初の手数。
    record_from: u32,
}

/// 1局をランダム序盤から終局まで進め、採用レコードを返す。
pub(super) fn play_game(
    pst: &Pst,
    rules: Rules,
    game_number: u32,
    settings: PlaySettings,
    table: &mut TranspositionTable,
) -> io::Result<CompletedGame> {
    play_game_from_opening(pst, rules, game_number, settings, table, None)
}

/// 実戦の開始局面では乱数手を入れず、次の局面から記録する。
pub(super) fn play_game_from_opening(
    pst: &Pst,
    rules: Rules,
    game_number: u32,
    settings: PlaySettings,
    table: &mut TranspositionTable,
    opening: Option<&Opening>,
) -> io::Result<CompletedGame> {
    let game_seed = derive_seed(settings.base_seed, u64::from(game_number));
    let mut game = starting_game(rules, game_seed, opening)?;
    let ply_offset = opening.map_or(0, |opening| opening.ply);
    let opening_ply = game.ply_count();
    let (mut injection_plan, mut injection_rng) =
        plan_injections(game_seed, opening_ply, settings.random_moves);
    if opening.is_some() {
        injection_plan.record_from = 1;
    }
    table.clear();
    let limits = SearchLimits::new(None, Some(u64::from(settings.nodes)), None, None)
        .expect("the CLI parser accepts only non-zero node limits");
    let mut candidates = Vec::new();
    let mut stats = Statistics {
        planned_injections: u64::try_from(injection_plan.plies.len())
            .expect("the injection count is at most 80"),
        ..Statistics::default()
    };
    let generator = MoveGenerator::new(rules.moves);

    while game.result().is_none() && game.ply_count() + ply_offset < u32::from(settings.max_ply) {
        if injection_plan
            .plies
            .binary_search(&game.ply_count())
            .is_ok()
        {
            let legal_moves = game.legal_moves();
            let move_count = NonZeroUsize::new(legal_moves.len()).ok_or_else(|| {
                invalid_data(format!(
                    "game {game_number} is ongoing but has no legal moves"
                ))
            })?;
            let selected = legal_moves[injection_rng.index(move_count)];
            game.play(selected).map_err(|error| {
                invalid_data(format!(
                    "game {game_number} rejected random injection move: {error}"
                ))
            })?;
            stats.performed_injections += 1;
            let offset = usize::try_from(game.ply_count() - 1 - opening_ply)
                .expect("an injection offset below 80 must fit in usize");
            stats.injection_offset_histogram[offset / 10] += 1;
            continue;
        }

        let snapshot = SearchSnapshot::from_game(&game).map_err(|_| {
            invalid_data(format!(
                "game {game_number} is ongoing but has no legal moves"
            ))
        })?;
        let search_result = search(pst, &snapshot, &limits, DEFAULT_THREADS, table)
            .map_err(|error| invalid_data(error.to_string()))?;
        stats.searched_positions += 1;
        stats.searched_nodes = stats
            .searched_nodes
            .checked_add(search_result.nodes)
            .ok_or_else(|| invalid_data("searched node count overflow"))?;

        if game.ply_count() < injection_plan.record_from {
            game.play(search_result.best_move).map_err(|error| {
                invalid_data(format!("game {game_number} rejected best move: {error}"))
            })?;
            stats.searched_plies += 1;
            continue;
        }
        stats.recordable_positions += 1;

        if search_result.score.unsigned_abs() >= MATE_BAND_START {
            stats.excluded_mate_band += 1;
        } else if best_move_is_tactical(game.position(), &generator, search_result.best_move)
            .map_err(|error| {
                invalid_data(format!(
                    "game {game_number} search returned an illegal best move: {error}"
                ))
            })?
        {
            stats.excluded_tactical += 1;
        } else if current_position_is_repeated(&game) {
            stats.excluded_repetition += 1;
        } else {
            let score = i16::try_from(search_result.score).map_err(|_| {
                invalid_data(format!(
                    "game {game_number} score {} does not fit in i16",
                    search_result.score
                ))
            })?;
            let ply = u16::try_from(game.ply_count() + ply_offset).map_err(|_| {
                invalid_data(format!(
                    "game {game_number} ply {} does not fit in u16",
                    game.ply_count()
                ))
            })?;
            candidates.push(Candidate {
                position: game.position().clone(),
                score,
                ply,
                search_key: *game
                    .search_key_history()
                    .last()
                    .expect("every game has an initial search key"),
            });
        }

        game.play(search_result.best_move).map_err(|error| {
            invalid_data(format!("game {game_number} rejected best move: {error}"))
        })?;
        stats.searched_plies += 1;
    }

    stats.total_plies = u64::from(game.ply_count() + ply_offset);
    let records = match game.result() {
        Some(result) => {
            stats.record_result(result);
            let records = candidates
                .into_iter()
                .map(|candidate| {
                    let outcome =
                        Outcome::from_game_result(result, candidate.position.side_to_move());
                    CompletedRecord {
                        record: Record::from_position(
                            &candidate.position,
                            candidate.score,
                            outcome,
                            game_number,
                            candidate.ply,
                        ),
                        search_key: candidate.search_key,
                    }
                })
                .collect::<Vec<_>>();
            stats.recorded_positions =
                u64::try_from(records.len()).map_err(|error| invalid_data(error.to_string()))?;
            records
        }
        None => {
            stats.discarded_games = 1;
            Vec::new()
        }
    };

    Ok(CompletedGame {
        game_number,
        records,
        stats,
    })
}

/// 対局シードからランダム着手の予定と記録開始手数を決める。
fn plan_injections(
    game_seed: NonZeroU64,
    opening_ply: u32,
    maximum: u8,
) -> (InjectionPlan, XorShift64) {
    let mut rng = XorShift64::new(derive_seed(game_seed.get(), 1));
    let plan = plan_injections_with_rng(&mut rng, opening_ply, maximum);
    (plan, rng)
}

/// 指定乱数列を進め、ランダム着手の予定と記録開始手数を決める。
fn plan_injections_with_rng(rng: &mut XorShift64, opening_ply: u32, maximum: u8) -> InjectionPlan {
    let planned = rng.index(
        NonZeroUsize::new(usize::from(maximum) + 1)
            .expect("the injection count range always contains zero"),
    );
    let mut offsets = std::array::from_fn::<_, INJECTION_WINDOW, _>(|index| index);
    for index in 0..planned {
        let remaining = NonZeroUsize::new(INJECTION_WINDOW - index)
            .expect("partial Fisher-Yates stops before the window is empty");
        let selected = index + rng.index(remaining);
        offsets.swap(index, selected);
    }
    let mut plies = offsets[..planned]
        .iter()
        .map(|&offset| {
            opening_ply
                .checked_add(u32::try_from(offset).expect("an offset below 80 fits in u32"))
                .expect("a game ply count cannot overflow within 80 plies")
        })
        .collect::<Vec<_>>();
    plies.sort_unstable();
    let record_from = plies.last().map_or(opening_ply, |&last| {
        last.checked_add(1)
            .expect("a game ply count cannot overflow after an injection")
    });
    InjectionPlan { plies, record_from }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::{Arguments, Operation},
        engine_default_rules,
        opening::generate_opening,
        test_support::test_table,
    };
    use clap::Parser;
    use minase::datagen::statistics::INJECTION_HISTOGRAM_BINS;
    use std::collections::BTreeSet;

    /// テスト用の対局設定を返す。
    const fn test_settings(base_seed: u64, random_moves: u8, max_ply: u16) -> PlaySettings {
        PlaySettings {
            base_seed,
            nodes: 100,
            random_moves,
            max_ply,
        }
    }

    /// 指定条件の対局について序盤と注入計画を再現する。
    fn opening_and_plan(
        rules: Rules,
        base_seed: u64,
        game_number: u32,
        maximum: u8,
    ) -> (u32, InjectionPlan) {
        let game_seed = derive_seed(base_seed, u64::from(game_number));
        let game = generate_opening(rules, game_seed).expect("the fixed opening is valid");
        let opening_ply = game.ply_count();
        let (plan, _) = plan_injections(game_seed, opening_ply, maximum);
        (opening_ply, plan)
    }

    /// 同じ対局条件はレコードと全統計を再現する。
    #[test]
    fn play_game_is_deterministic_for_same_seed_and_arguments() {
        let rules = engine_default_rules().expect("engine-default rules are valid");
        let pst = minase::eval::weights().expect("embedded weights are valid");
        let mut first_table = test_table();
        let mut second_table = test_table();
        let settings = test_settings(7, 4, 600);
        let first = play_game(pst.as_ref(), rules, 1, settings, &mut first_table)
            .expect("the fixed game is valid");
        let second = play_game(pst.as_ref(), rules, 1, settings, &mut second_table)
            .expect("the fixed game is valid");

        assert!(!first.records.is_empty());
        assert_eq!(first, second);
        let (_, plan) = opening_and_plan(rules, 7, 1, 4);
        assert!(
            first
                .records
                .iter()
                .all(|completed| u32::from(completed.record.ply()) >= plan.record_from)
        );
        assert_eq!(
            first.stats.recordable_positions,
            first
                .stats
                .total_plies
                .saturating_sub(u64::from(plan.record_from))
        );
        assert_eq!(
            first.stats.recordable_positions,
            first.stats.recorded_positions
                + first.stats.excluded_mate_band
                + first.stats.excluded_tactical
                + first.stats.excluded_repetition
        );
    }

    /// 注入計画の途中で打ち切った対局でも実施回数と探索回数を正しく集計する。
    #[test]
    fn injection_counts_and_offsets_stay_within_configured_bounds() {
        let rules = engine_default_rules().expect("engine-default rules are valid");
        let pst = minase::eval::weights().expect("embedded weights are valid");
        let maximum = 80;

        let game_number = 1;
        let (opening_ply, plan) = opening_and_plan(rules, 19, game_number, maximum);
        let mut table = test_table();
        let completed = play_game(
            pst.as_ref(),
            rules,
            game_number,
            test_settings(19, maximum, 32),
            &mut table,
        )
        .expect("the fixed game is valid");

        assert!(plan.plies.len() <= usize::from(maximum));
        assert_eq!(
            completed.stats.planned_injections,
            u64::try_from(plan.plies.len()).unwrap()
        );
        assert!(completed.stats.performed_injections > 0);
        assert!(completed.stats.performed_injections < completed.stats.planned_injections);
        assert_eq!(completed.stats.total_plies, 32);
        assert!(u64::from(plan.record_from) > completed.stats.total_plies);
        assert!(completed.records.is_empty());
        assert_eq!(
            completed
                .stats
                .injection_offset_histogram
                .iter()
                .sum::<u64>(),
            completed.stats.performed_injections
        );

        let mut expected_histogram = [0_u64; INJECTION_HISTOGRAM_BINS];
        for &ply in plan
            .plies
            .iter()
            .filter(|&&ply| u64::from(ply) < completed.stats.total_plies)
        {
            let offset = usize::try_from(ply - opening_ply).unwrap();
            assert!(offset < INJECTION_WINDOW);
            expected_histogram[offset / 10] += 1;
        }
        assert_eq!(
            completed.stats.injection_offset_histogram,
            expected_histogram
        );
        assert_eq!(
            completed.stats.searched_plies + completed.stats.performed_injections,
            completed.stats.total_plies - u64::from(opening_ply)
        );
        assert_eq!(
            completed.stats.searched_positions,
            completed.stats.searched_plies
        );
    }

    /// 注入計画は0以上80未満の異なるオフセットと予定由来の境界を返す。
    #[test]
    fn injection_plan_has_unique_offsets_and_planned_boundary() {
        let opening_ply = 12;
        for number in 1..=32 {
            let game_seed = derive_seed(31, number);
            let (plan, _) = plan_injections(game_seed, opening_ply, 80);
            let unique = plan.plies.iter().copied().collect::<BTreeSet<_>>();

            assert_eq!(unique.len(), plan.plies.len());
            assert!(plan.plies.len() <= 80);
            assert!(
                plan.plies
                    .iter()
                    .all(|&ply| (opening_ply..opening_ply + 80).contains(&ply))
            );
            assert_eq!(
                plan.record_from,
                plan.plies.last().map_or(opening_ply, |&last| last + 1)
            );
        }
    }

    /// 上限0では注入を予定せず序盤終了局面から記録する。
    #[test]
    fn zero_random_moves_produces_empty_plan_at_opening_boundary() {
        let opening_ply = 12;
        let (plan, _) = plan_injections(derive_seed(41, 12), opening_ply, 0);
        assert!(plan.plies.is_empty());
        assert_eq!(plan.record_from, opening_ply);
    }

    /// 段階7の試行生成で選んだ4,000手を、省略時の手数上限とする。
    #[test]
    fn omitted_max_ply_uses_measured_generation_cap() {
        let arguments = Arguments::try_parse_from([
            "selfplay_gen",
            "generate",
            "--output",
            "unused.bin",
            "--games",
            "1",
            "--seed",
            "1",
            "--random-moves",
            "0",
        ])
        .expect("the required generation arguments are valid");
        let Operation::Generate(arguments) = arguments.command else {
            panic!("generate must select generation arguments");
        };

        assert_eq!(arguments.max_ply, 4_000);
    }
}
