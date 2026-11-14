//! 自己対局1局の進行とランダム着手の注入。

use super::{INJECTION_WINDOW, opening::Opening};
use minase::datagen::game::{CompletedGame, CompletedRecord};
use minase::datagen::opening::generate_opening_candidate;
use minase::datagen::statistics::Statistics;
use minase::datagen::{
    MATE_BAND_START, best_move_is_tactical, current_position_is_repeated, data_error, invalid_data,
};
use minase::eval::Nnue;
use minase::search::{
    DEFAULT_THREADS, HistoryTables, SearchLimits, SearchResult, SearchSnapshot, TranspositionTable,
    search,
};
use minase::training::records::{Entry, Exclusion, GameRecord, Origin, Outcome, Policy, Value};
use minase_core::rng::{XorShift64, derive_seed};
use minase_core::{Color, Game, Move, MoveGenerator, Position, Rules};
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
    /// 開始局面の選別と評価値による裁定の条件。
    pub(super) policy: GenerationPolicy,
}

/// 開始局面を採用する探索値の絶対値の上限（cp）。
const OPENING_SCORE_LIMIT: u32 = 1_700;
/// 評価値による裁定に数える絶対値の下限（cp）。
const ADJUDICATION_SCORE: u32 = 4_250;
/// 同じ側の優勢が裁定に必要な連続回数。
const ADJUDICATION_PLIES: u8 = 5;

/// CLIでは固定し、再開時にも照合する生成条件。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct GenerationPolicy {
    pub(super) opening_score_limit: u32,
    pub(super) adjudication_score: u32,
    pub(super) adjudication_plies: u8,
}

impl Default for GenerationPolicy {
    fn default() -> Self {
        Self {
            opening_score_limit: OPENING_SCORE_LIMIT,
            adjudication_score: ADJUDICATION_SCORE,
            adjudication_plies: ADJUDICATION_PLIES,
        }
    }
}

impl GenerationPolicy {
    fn accepts_opening(self, score: i32) -> bool {
        score.unsigned_abs() <= self.opening_score_limit
    }
}

/// 手番側の探索値を先手の視点へ直し、同じ側の連続した優勢を数える。
#[derive(Default)]
struct EvaluationAdjudication {
    leader: Option<Color>,
    count: u8,
}

impl EvaluationAdjudication {
    fn update(
        &mut self,
        score: Option<i32>,
        side: Color,
        policy: GenerationPolicy,
    ) -> Option<Color> {
        let Some(score) = score.filter(|score| {
            score.unsigned_abs() >= policy.adjudication_score
                && score.unsigned_abs() < MATE_BAND_START
                && *score != 0
        }) else {
            *self = Self::default();
            return None;
        };
        let leader = if score > 0 { side } else { side.opposite() };
        self.count = if self.leader == Some(leader) {
            self.count + 1
        } else {
            1
        };
        self.leader = Some(leader);
        (self.count >= policy.adjudication_plies).then_some(leader)
    }
}

/// 実際に行った探索だけを、候補の採否と記録の有無によらず集計する。
fn search_position(
    nnue: &Nnue,
    game: &Game,
    nodes: u32,
    table: &mut TranspositionTable,
    histories: &mut HistoryTables,
    stats: &mut Statistics,
) -> io::Result<SearchResult> {
    let snapshot = SearchSnapshot::from_game(game).map_err(data_error)?;
    let limits = SearchLimits::new(None, Some(u64::from(nodes)), None, None).map_err(data_error)?;
    let result =
        search(nnue, &snapshot, &limits, DEFAULT_THREADS, table, histories).map_err(data_error)?;
    stats.searched_positions += 1;
    stats.searched_nodes = stats
        .searched_nodes
        .checked_add(result.nodes)
        .ok_or_else(|| invalid_data("searched node count overflow"))?;
    Ok(result)
}

/// 除外理由は詰みの帯、王手、戦術的な最善手、再出現の順で1つだけ返す。
fn exclusion_reason(
    game: &Game,
    generator: &MoveGenerator,
    score: i32,
    best_move: Move,
) -> io::Result<Exclusion> {
    Ok(if score.unsigned_abs() >= MATE_BAND_START {
        Exclusion::MateBand
    } else if game.position().is_in_check(generator) {
        Exclusion::Check
    } else if best_move_is_tactical(game.position(), generator, best_move).map_err(data_error)? {
        Exclusion::Tactical
    } else if current_position_is_repeated(game) {
        Exclusion::Repetition
    } else {
        Exclusion::None
    })
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
/// 探索値の絶対値が[`MATE_BAND_START`]以上、王手、合法な最善手が捕獲または成り、現局面の再出現の順に除外し、最初に該当した理由だけを数える。
/// 手数上限で終局しなかった対局は書かず、終局した対局には結果と終局理由を付ける。
/// 実戦の開始局面では呼び手が注入回数0を指定し、乱数手を入れず次の局面から価値を記録する。
/// 注入の予定と乱数列は[`plan_injections`]に従う。
/// 理由は`docs/adr/archive/exclude-mate-tactical-and-repeated-positions-from-teacher-data.md`にある。
pub(super) fn play_game_from_opening(
    nnue: &Nnue,
    rules: Rules,
    game_number: u32,
    settings: PlaySettings,
    table: &mut TranspositionTable,
    opening: Option<&Opening>,
) -> io::Result<CompletedGame> {
    let game_seed = derive_seed(settings.base_seed, u64::from(game_number));
    table.clear();
    let mut histories = HistoryTables::new(DEFAULT_THREADS);
    let mut stats = Statistics::default();
    let mut first_search = None;
    let (mut game, opening_moves) = if let Some(opening) = opening {
        (opening.starting_game(rules), Vec::new())
    } else {
        let mut opening_seed = derive_seed(game_seed.get(), 0);
        loop {
            if let Some((candidate, moves)) = generate_opening_candidate(rules, opening_seed)? {
                if candidate.ply_count() >= u32::from(settings.max_ply) {
                    break (candidate, moves);
                }
                table.clear();
                histories.clear();
                let result = search_position(
                    nnue,
                    &candidate,
                    settings.nodes,
                    table,
                    &mut histories,
                    &mut stats,
                )?;
                if settings.policy.accepts_opening(result.score) {
                    first_search = Some(result);
                    break (candidate, moves);
                }
                stats.rejected_openings += 1;
            }
            opening_seed = derive_seed(opening_seed.get(), 0);
        }
    };
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
    let mut candidates = Vec::new();
    stats.planned_injections =
        u64::try_from(injection_plan.plies.len()).expect("the injection count is at most 80");
    let mut adjudication = EvaluationAdjudication::default();
    let mut adjudicated = None;
    let generator = MoveGenerator::new(rules.moves);

    while game.result().is_none() && game.ply_count() + ply_offset < u32::from(settings.max_ply) {
        if injection_plan
            .plies
            .binary_search(&game.ply_count())
            .is_ok()
        {
            first_search = None;
            adjudication.update(None, game.position().side_to_move(), settings.policy);
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
            game.play(selected).map_err(data_error)?;
            stats.performed_injections += 1;
            let offset = usize::try_from(game.ply_count() - 1 - opening_ply)
                .expect("an injection offset below 80 must fit in usize");
            stats.injection_offset_histogram[offset / 10] += 1;
            continue;
        }

        let search_result = match first_search.take() {
            Some(result) => result,
            None => search_position(
                nnue,
                &game,
                settings.nodes,
                table,
                &mut histories,
                &mut stats,
            )?,
        };
        if let Some(winner) = adjudication.update(
            Some(search_result.score),
            game.position().side_to_move(),
            settings.policy,
        ) {
            adjudicated = Some(winner);
            break;
        }

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
            game.play(search_result.best_move).map_err(data_error)?;
            stats.searched_plies += 1;
            continue;
        }
        stats.recordable_positions += 1;

        let score = i16::try_from(search_result.score).map_err(data_error)?;
        let exclusion = exclusion_reason(
            &game,
            &generator,
            search_result.score,
            search_result.best_move,
        )?;
        match exclusion {
            Exclusion::MateBand => stats.excluded_mate_band += 1,
            Exclusion::Check => stats.excluded_check += 1,
            Exclusion::Tactical => stats.excluded_tactical += 1,
            Exclusion::Repetition => stats.excluded_repetition += 1,
            Exclusion::None => candidates.push(CompletedRecord {
                score,
                search_key: *game
                    .repetition_key_history()
                    .last()
                    .expect("every game has an initial search key"),
            }),
        }
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

        game.play(search_result.best_move).map_err(data_error)?;
        stats.searched_plies += 1;
    }

    stats.total_plies = u64::from(game.ply_count() + ply_offset);
    let outcome = if let Some(result) = game.result() {
        stats.record_result(result);
        Some(Outcome::Game(result))
    } else {
        adjudicated.map(|winner| {
            stats.record_evaluation_adjudication(winner);
            Outcome::EvaluationAdjudication { winner }
        })
    };
    let record = match outcome {
        Some(outcome) => {
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
                    outcome,
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
    let record_from = recording_start(opening_ply, &plies);
    InjectionPlan { plies, record_from }
}

/// 記録は最後の予定注入の次、注入がなければ開始局面から始める。
fn recording_start(opening_ply: u32, plies: &[u32]) -> u32 {
    plies.last().map_or(opening_ply, |&last| {
        last.checked_add(1)
            .expect("a game ply count cannot overflow after an injection")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::datagen::opening::generate_opening;

    #[test]
    fn recording_starts_after_last_injection_or_at_opening() {
        assert_eq!(recording_start(12, &[]), 12);
        assert_eq!(recording_start(12, &[12]), 13);
        assert_eq!(recording_start(12, &[14, 50, 91]), 92);
    }

    use crate::selfplay_gen::test_support::test_table;
    use minase::datagen::statistics::INJECTION_HISTOGRAM_BINS;
    use std::collections::BTreeSet;

    /// テスト用の対局設定を返す。
    fn test_settings(base_seed: u64, random_moves: u8, max_ply: u16) -> PlaySettings {
        PlaySettings {
            base_seed,
            nodes: 100,
            random_moves,
            max_ply,
            policy: GenerationPolicy {
                opening_score_limit: MATE_BAND_START - 1,
                adjudication_score: MATE_BAND_START,
                ..GenerationPolicy::default()
            },
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
        let rules = Rules::LISHOGI;
        let nnue = crate::selfplay_gen::test_support::epoch_zero_nnue();
        let mut first_table = test_table();
        let mut second_table = test_table();
        // 固定重みで終局まで進め、未終局の打ち切りによる記録の欠落も検出する。
        let settings = test_settings(7, 4, 1200);
        let first = play_game(&nnue, rules, 1, settings, &mut first_table)
            .expect("the fixed game is valid");
        let second = play_game(&nnue, rules, 1, settings, &mut second_table)
            .expect("the fixed game is valid");

        assert_eq!(first.stats.discarded_games, 0);
        assert!(!first.records.is_empty());
        assert_eq!(first, second);
        let (opening_ply, plan) = opening_and_plan(rules, 7, 1, 4);
        assert!(!plan.plies.is_empty());
        assert!(plan.plies[0] > opening_ply);
        let decoded = crate::selfplay_gen::test_support::round_trip(first.record.as_ref().unwrap());
        let record = decoded.record();
        assert_eq!(record.start_position(), &Position::initial());
        assert_eq!(record.start_ply(), 0);
        assert_eq!(record.origin(), Origin::RandomSelfplay);
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
                + first.stats.excluded_check
                + first.stats.excluded_mate_band
                + first.stats.excluded_tactical
                + first.stats.excluded_repetition
        );
    }

    /// 注入計画の途中で打ち切った対局でも実施回数と探索回数を正しく集計する。
    #[test]
    fn injection_counts_and_offsets_stay_within_configured_bounds() {
        let rules = Rules::LISHOGI;
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
    fn injection_plan_has_unique_offsets() {
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
    #[test]
    fn opening_selection_accepts_1700_and_rejects_1701_for_both_signs() {
        let policy = GenerationPolicy::default();
        for sign in [-1, 1] {
            assert!(policy.accepts_opening(sign * 1700));
            assert!(!policy.accepts_opening(sign * 1701));
            assert!(!policy.accepts_opening(sign * 29000));
        }
    }

    #[test]
    fn adjudication_counts_five_scores_for_the_same_winner_and_resets() {
        let policy = GenerationPolicy::default();
        for winner in Color::ALL {
            let mut state = EvaluationAdjudication::default();
            for ply in 0..5 {
                let side = if ply % 2 == 0 {
                    Color::Black
                } else {
                    Color::White
                };
                let score = if winner == side { 4250 } else { -4250 };
                assert_eq!(
                    state.update(Some(score), side, policy),
                    (ply == 4).then_some(winner)
                );
            }
        }
        // 閾値未満、詰みの帯、探索なしは、4回の優勢の後でも連続を切る。
        for reset in [Some(4249), Some(-4249), Some(29000), Some(-29000), None] {
            let mut state = EvaluationAdjudication::default();
            for _ in 0..4 {
                assert_eq!(state.update(Some(4250), Color::Black, policy), None);
            }
            assert_eq!(state.update(reset, Color::Black, policy), None);
            for count in 1..=5 {
                assert_eq!(
                    state.update(Some(4250), Color::Black, policy),
                    (count == 5).then_some(Color::Black)
                );
            }
        }
        let mut state = EvaluationAdjudication::default();
        for _ in 0..4 {
            assert_eq!(state.update(Some(4250), Color::Black, policy), None);
        }
        for count in 1..=5 {
            assert_eq!(
                state.update(Some(-4250), Color::Black, policy),
                (count == 5).then_some(Color::White)
            );
        }
    }

    #[test]
    fn exclusions_prioritize_mate_check_tactics_and_repetition() {
        use minase_core::{
            Color::{Black, White},
            PieceKind::{King, Pawn, Rook},
            test_util::{position, sq},
        };
        let generator = MoveGenerator::new(Rules::LISHOGI.moves);
        let quiet = Move {
            from: sq(2, 2),
            mid: None,
            to: sq(3, 2),
            promote: false,
        };
        let tactical = Move {
            from: sq(2, 2),
            mid: None,
            to: sq(2, 3),
            promote: false,
        };
        for checked in [false, true] {
            let game = Game::from_position(
                Rules::LISHOGI,
                position(
                    Black,
                    &[
                        (sq(0, 0), Black, King),
                        (sq(11, 11), White, King),
                        (sq(2, 2), Black, Rook),
                        (sq(2, 3), White, Pawn),
                        (sq(if checked { 0 } else { 1 }, 5), White, Rook),
                    ],
                ),
            );
            assert_eq!(
                exclusion_reason(&game, &generator, 100, quiet).unwrap(),
                if checked {
                    Exclusion::Check
                } else {
                    Exclusion::None
                }
            );
            assert_eq!(
                exclusion_reason(&game, &generator, 100, tactical).unwrap(),
                if checked {
                    Exclusion::Check
                } else {
                    Exclusion::Tactical
                }
            );
            assert_eq!(
                exclusion_reason(&game, &generator, 29000, tactical).unwrap(),
                Exclusion::MateBand
            );
        }
        let mut game = Game::from_position(
            Rules::LISHOGI,
            position(
                Black,
                &[
                    (sq(0, 0), Black, King),
                    (sq(11, 11), White, King),
                    (sq(2, 2), Black, Rook),
                    (sq(2, 3), White, Pawn),
                    (sq(9, 9), White, Rook),
                ],
            ),
        );
        for (from, to) in [
            (sq(2, 2), sq(3, 2)),
            (sq(9, 9), sq(8, 9)),
            (sq(3, 2), sq(2, 2)),
            (sq(8, 9), sq(9, 9)),
        ] {
            game.play(Move {
                from,
                mid: None,
                to,
                promote: false,
            })
            .unwrap();
        }
        assert!(current_position_is_repeated(&game));
        assert_eq!(
            exclusion_reason(&game, &generator, 100, quiet).unwrap(),
            Exclusion::Repetition
        );
        assert_eq!(
            exclusion_reason(&game, &generator, 100, tactical).unwrap(),
            Exclusion::Tactical
        );
    }

    fn opening_for_position(position: &Position, ply: u16) -> Vec<Opening> {
        let text = format!(
            "fixture {ply} {} - {}\n",
            minase_core::to_sfen(position),
            u32::from(ply) + 1
        );
        super::super::opening::read_openings(text.as_bytes()).unwrap()
    }

    #[test]
    fn adjudication_saves_both_winners_without_the_fifth_move_or_value() {
        use minase_core::{
            Color::{Black, White},
            PieceKind::{King, Pawn, Rook},
            test_util::{position, reflect_ranks_and_swap_colors, sq},
        };
        let black_ahead = position(
            Black,
            &[
                (sq(0, 0), Black, King),
                (sq(11, 11), White, King),
                (sq(2, 2), Black, Rook),
                (sq(3, 2), Black, Rook),
                (sq(10, 9), White, Pawn),
            ],
        );
        for winner in Color::ALL {
            let position = if winner == Black {
                black_ahead.clone()
            } else {
                reflect_ranks_and_swap_colors(&black_ahead)
            };
            let openings = opening_for_position(&position, 60);
            let settings = PlaySettings {
                policy: GenerationPolicy {
                    opening_score_limit: 0,
                    adjudication_score: 1,
                    adjudication_plies: 5,
                },
                ..test_settings(7, 0, 100)
            };
            let completed = play_game_from_opening(
                &crate::selfplay_gen::test_support::epoch_zero_nnue(),
                Rules::LISHOGI,
                1,
                settings,
                &mut test_table(),
                Some(&openings[0]),
            )
            .unwrap();
            let decoded =
                crate::selfplay_gen::test_support::round_trip(completed.record.as_ref().unwrap());
            assert_eq!(
                decoded.record().outcome(),
                Outcome::EvaluationAdjudication { winner }
            );
            assert_eq!(completed.stats.searched_positions, 5);
            assert_eq!(completed.stats.searched_plies, 4);
            assert_eq!(completed.stats.rejected_openings, 0);
            assert_eq!(completed.stats.evaluation_adjudication_wins, 1);
            assert_eq!(completed.stats.black_wins, u64::from(winner == Black));
            assert_eq!(completed.stats.white_wins, u64::from(winner == White));
            assert_eq!(decoded.record().entries().len(), 5);
            let last = decoded.record().entries().last().unwrap();
            assert_eq!(last.played_move(), None);
            assert_eq!(last.value(), Value::None);
        }
    }

    #[test]
    fn rules_and_ply_limit_precede_evaluation_adjudication() {
        use minase_core::{
            Color::{Black, White},
            GameResult,
            PieceKind::{King, Pawn, Rook},
            WinReason,
            test_util::{position, sq},
        };
        // 次の1手で王を取れる。探索値は詰みの帯なので裁定せず、上限の手で規則による勝ちを保存する。
        let board = position(
            Black,
            &[
                (sq(0, 0), Black, King),
                (sq(5, 8), White, King),
                (sq(5, 5), Black, Rook),
                (sq(0, 1), Black, Rook),
                (sq(11, 9), White, Pawn),
            ],
        );
        let openings = opening_for_position(&board, 60);
        let nnue = crate::selfplay_gen::test_support::epoch_zero_nnue();
        let policy = GenerationPolicy {
            adjudication_score: 1,
            adjudication_plies: 1,
            ..GenerationPolicy::default()
        };
        let settings = PlaySettings {
            policy,
            ..test_settings(7, 0, 61)
        };
        let completed = play_game_from_opening(
            &nnue,
            Rules::LISHOGI,
            1,
            settings,
            &mut test_table(),
            Some(&openings[0]),
        )
        .unwrap();
        assert!(matches!(
            completed.record.unwrap().outcome(),
            Outcome::Game(GameResult::Win {
                winner: Black,
                reason: WinReason::RoyalCapture
            })
        ));
        assert_eq!(completed.stats.evaluation_adjudication_wins, 0);
        assert_eq!(completed.stats.discarded_games, 0);
        assert_eq!(completed.stats.total_plies, 61);
        let completed = play_game_from_opening(
            &nnue,
            Rules::LISHOGI,
            1,
            PlaySettings {
                max_ply: 60,
                ..settings
            },
            &mut test_table(),
            Some(&openings[0]),
        )
        .unwrap();
        assert!(completed.record.is_none());
        assert_eq!(completed.stats.discarded_games, 1);
        assert_eq!(completed.stats.searched_positions, 0);
    }
    #[test]
    fn accepted_opening_is_searched_once_even_when_first_move_is_injected() {
        let rules = Rules::LISHOGI;
        let nnue = crate::selfplay_gen::test_support::epoch_zero_nnue();
        let (number, opening_ply) = (1..=512)
            .find_map(|number| {
                let (ply, plan) = opening_and_plan(rules, 19, number, 80);
                plan.plies.contains(&ply).then_some((number, ply))
            })
            .expect("a fixed seed schedules an injection on the first move");
        for random_moves in [0, 80] {
            let settings = test_settings(19, random_moves, (opening_ply + 1) as u16);
            let completed = play_game(&nnue, rules, number, settings, &mut test_table()).unwrap();
            assert_eq!(completed.stats.rejected_openings, 0);
            assert_eq!(completed.stats.searched_positions, 1);
            assert_eq!(
                completed.stats.performed_injections,
                u64::from(random_moves != 0)
            );
            assert_eq!(completed.stats.searched_plies, u64::from(random_moves == 0));
        }
    }
}
