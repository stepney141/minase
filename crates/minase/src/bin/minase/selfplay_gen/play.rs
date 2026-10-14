//! 自己対局1局の進行とランダム着手の注入。

use super::{
    INJECTION_WINDOW,
    opening::{Opening, starting_game},
};
use minase::datagen::game::{CompletedGame, CompletedRecord};
use minase::datagen::statistics::Statistics;
use minase::datagen::{
    MATE_BAND_START, best_move_is_tactical, current_position_is_repeated, data_error, invalid_data,
};
use minase::eval::Nnue;
use minase::search::{DEFAULT_THREADS, SearchLimits, SearchSnapshot, TranspositionTable, search};
use minase::training::records::{Entry, Exclusion, GameRecord, Origin, Outcome, Policy, Value};
use minase_core::rng::{XorShift64, derive_seed};
use minase_core::{MoveGenerator, Position, Rules};
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

/// 1局のランダム着手予定と記録開始手数。
#[derive(Clone, PartialEq, Eq, Debug)]
struct InjectionPlan {
    /// 注入する対局開始時からの手数。昇順で重複しない。
    plies: Vec<u32>,
    /// 記録対象とする最初の手数。
    record_from: u32,
}

/// 1局をランダム序盤から終局まで進め、対局記録と採用局面の統計を返す。
pub(super) fn play_game(
    nnue: &Nnue,
    rules: Rules,
    game_number: u32,
    settings: PlaySettings,
    table: &mut TranspositionTable,
) -> io::Result<CompletedGame> {
    play_game_from_opening(nnue, rules, game_number, settings, table, None)
}

/// 1局を終局まで進め、全局面の着手と教師情報を返す。
/// 価値の記録は序盤ランダム化の後から始め、注入を予定した場合は最後の予定注入の次の手から始める。
/// 探索値の絶対値が[`MATE_BAND_START`]以上、合法な最善手が捕獲または成り、現局面の再出現の順に除外し、最初に該当した理由だけを数える。
/// 手数上限で終局しなかった対局は書かず、終局した対局には結果と終局理由を付ける。
/// 実戦の開始局面では呼び手が注入回数0を指定し、乱数手を入れず次の局面から価値を記録する。
/// 注入の予定と乱数列は[`plan_injections`]に従う。
/// 理由は`docs/adr/exclude-mate-tactical-and-repeated-positions-from-teacher-data.md`にある。
pub(super) fn play_game_from_opening(
    nnue: &Nnue,
    rules: Rules,
    game_number: u32,
    settings: PlaySettings,
    table: &mut TranspositionTable,
    opening: Option<&Opening>,
) -> io::Result<CompletedGame> {
    let game_seed = derive_seed(settings.base_seed, u64::from(game_number));
    let (mut game, opening_moves) = starting_game(rules, game_seed, opening)?;
    let start_position = if opening.is_some() {
        game.position().clone()
    } else {
        Position::initial()
    };
    let mut entries = opening_moves
        .into_iter()
        .map(|mv| {
            Entry::new(Some(mv), Value::None, Exclusion::None, Policy::None, None)
                .map_err(data_error)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let ply_offset = opening.map_or(0, |opening| opening.origin.ply);
    let start_ply = u16::try_from(ply_offset).map_err(data_error)?;
    let opening_ply = game.ply_count();
    let (mut injection_plan, mut injection_rng) =
        plan_injections(game_seed, opening_ply, settings.random_moves);
    if opening.is_some() {
        injection_plan.record_from = 1;
    }
    table.clear();
    let mut histories = minase::search::HistoryTables::new(DEFAULT_THREADS);
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
            entries.push(
                Entry::new(
                    Some(selected),
                    Value::None,
                    Exclusion::None,
                    Policy::None,
                    None,
                )
                .map_err(data_error)?,
            );
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
        let search_result = search(
            nnue,
            &snapshot,
            &limits,
            DEFAULT_THREADS,
            table,
            &mut histories,
        )
        .map_err(|error| invalid_data(error.to_string()))?;
        stats.searched_positions += 1;
        stats.searched_nodes = stats
            .searched_nodes
            .checked_add(search_result.nodes)
            .ok_or_else(|| invalid_data("searched node count overflow"))?;

        if game.ply_count() < injection_plan.record_from {
            entries.push(
                Entry::new(
                    Some(search_result.best_move),
                    Value::None,
                    Exclusion::None,
                    Policy::PlayedMove,
                    None,
                )
                .map_err(data_error)?,
            );
            game.play(search_result.best_move).map_err(|error| {
                invalid_data(format!("game {game_number} rejected best move: {error}"))
            })?;
            stats.searched_plies += 1;
            continue;
        }
        stats.recordable_positions += 1;

        let score = i16::try_from(search_result.score).map_err(data_error)?;
        let exclusion = if search_result.score.unsigned_abs() >= MATE_BAND_START {
            stats.excluded_mate_band += 1;
            Exclusion::MateBand
        } else if best_move_is_tactical(game.position(), &generator, search_result.best_move)
            .map_err(|error| {
                invalid_data(format!(
                    "game {game_number} search returned an illegal best move: {error}"
                ))
            })?
        {
            stats.excluded_tactical += 1;
            Exclusion::Tactical
        } else if current_position_is_repeated(&game) {
            stats.excluded_repetition += 1;
            Exclusion::Repetition
        } else {
            candidates.push(CompletedRecord {
                score,
                search_key: *game
                    .search_key_history()
                    .last()
                    .expect("every game has an initial search key"),
            });
            Exclusion::None
        };
        entries.push(
            Entry::new(
                Some(search_result.best_move),
                Value::AlphaBeta(score),
                exclusion,
                Policy::PlayedMove,
                None,
            )
            .map_err(data_error)?,
        );

        game.play(search_result.best_move).map_err(|error| {
            invalid_data(format!("game {game_number} rejected best move: {error}"))
        })?;
        stats.searched_plies += 1;
    }

    stats.total_plies = u64::from(game.ply_count() + ply_offset);
    let record = match game.result() {
        Some(result) => {
            stats.record_result(result);
            entries.push(
                Entry::new(None, Value::None, Exclusion::None, Policy::None, None)
                    .map_err(data_error)?,
            );
            stats.recorded_positions = candidates.len() as u64;
            Some(
                GameRecord::new(
                    game_number,
                    if opening.is_some() {
                        Origin::HumanOpeningSelfplay
                    } else {
                        Origin::RandomSelfplay
                    },
                    Outcome::Game(result),
                    start_ply,
                    start_position,
                    entries,
                )
                .map_err(data_error)?,
            )
        }
        None => {
            stats.discarded_games = 1;
            candidates.clear();
            None
        }
    };

    Ok(CompletedGame {
        game_number,
        record,
        records: candidates,
        stats,
    })
}

/// 対局シードから派生させた専用の乱数列で、注入予定と注入手の選択を行う。
/// ワーカーの乱数状態を使わないため、結果は並列度に依存しない。
/// 注入の手番では教師探索をせず、この乱数列で合法手から一様に選んで指す。
/// 予定の範囲と記録開始位置は[`plan_injections_with_rng`]に従う。
fn plan_injections(
    game_seed: NonZeroU64,
    opening_ply: u32,
    maximum: u8,
) -> (InjectionPlan, XorShift64) {
    let mut rng = XorShift64::new(derive_seed(game_seed.get(), 1));
    let plan = plan_injections_with_rng(&mut rng, opening_ply, maximum);
    (plan, rng)
}

/// 注入回数を0〜上限から一様に選び、序盤終了手数pから`[p, p+80)`の範囲で注入手数を重複なく選ぶ。
/// 記録は最後に予定した注入の次の手から始め、注入回数0ならpから始める。
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
    use crate::selfplay_gen::{
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
        let (game, _) = generate_opening(rules, game_seed).expect("the fixed opening is valid");
        let opening_ply = game.ply_count();
        let (plan, _) = plan_injections(game_seed, opening_ply, maximum);
        (opening_ply, plan)
    }

    /// 同じ対局条件はレコードと全統計を再現する。
    #[test]
    fn play_game_is_deterministic_for_same_seed_and_arguments() {
        let rules = engine_default_rules().expect("engine-default rules are valid");
        let nnue = minase::eval::weights().expect("embedded weights are valid");
        let mut first_table = test_table();
        let mut second_table = test_table();
        // 100ノードの対局が終局して記録を残すように、手数上限は埋め込んだ重みで確かめた値にする。
        let settings = test_settings(7, 4, 1200);
        let first = play_game(nnue.as_ref(), rules, 1, settings, &mut first_table)
            .expect("the fixed game is valid");
        let second = play_game(nnue.as_ref(), rules, 1, settings, &mut second_table)
            .expect("the fixed game is valid");

        assert!(!first.records.is_empty());
        assert_eq!(first, second);
        let (opening_ply, plan) = opening_and_plan(rules, 7, 1, 4);
        assert!(!plan.plies.is_empty());
        assert!(plan.plies[0] > opening_ply);
        let decoded = crate::selfplay_gen::test_support::round_trip(first.record.as_ref().unwrap());
        let record = decoded.record();
        assert_eq!(record.start_position(), &Position::initial());
        assert_eq!(record.start_ply(), 0);
        assert_eq!(record.entries().len() as u64, first.stats.total_plies + 1);
        let mut eligible = 0;
        for (ply, entry) in record.entries().iter().enumerate() {
            let ply = ply as u32;
            if u64::from(ply) == first.stats.total_plies {
                assert_eq!(entry.played_move(), None);
                assert_eq!(entry.value(), Value::None);
                assert_eq!(entry.policy(), &Policy::None);
                assert_eq!(entry.exclusion(), Exclusion::None);
                continue;
            }
            assert!(entry.played_move().is_some());
            if ply < opening_ply || plan.plies.contains(&ply) {
                assert_eq!(entry.value(), Value::None);
                assert_eq!(entry.policy(), &Policy::None);
                assert_eq!(entry.exclusion(), Exclusion::None);
            } else {
                assert_eq!(entry.policy(), &Policy::PlayedMove);
                if ply < plan.record_from {
                    assert_eq!(entry.value(), Value::None);
                    assert_eq!(entry.exclusion(), Exclusion::None);
                } else {
                    assert!(matches!(entry.value(), Value::AlphaBeta(_)));
                    if entry.exclusion() == Exclusion::None {
                        eligible += 1;
                    }
                }
            }
        }
        assert_eq!(eligible, first.stats.recorded_positions);
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
        let nnue = minase::eval::weights().expect("embedded weights are valid");
        let maximum = 80;

        let game_number = 1;
        let (opening_ply, plan) = opening_and_plan(rules, 19, game_number, maximum);
        let mut table = test_table();
        let completed = play_game(
            nnue.as_ref(),
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
        assert!(completed.record.is_none());
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

    /// `docs/adr/cap-teacher-games-at-4000-plies.md`で選んだ4,000手を既定の上限とする。
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
