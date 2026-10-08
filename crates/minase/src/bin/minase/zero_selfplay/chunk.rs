//! 1つの塊の生成。局の集合をスレッドに分けて進め、評価が必要な葉を局をまたいで1回の推論にまとめ、
//! 塊のMNZTとサマリを一時名で書いてから名前を変えて確定する（docs/plans/alphazero.md「強化学習」）。

use super::play::{Ending, FinishedGame, GameSettings, SelfPlayGame};
use crate::selfplay_gen::opening::Opening;
use minase::datagen::{data_error, invalid_data};
use minase::nn::POLICY_SIZE;
use minase::nn::evaluator::{Evaluator, INPUT_SIZE};
use minase::nn::mnzt;
use minase_core::rng::derive_seed;
use minase_core::{Color, DrawReason, GameResult, WinReason};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Instant,
};

/// スレッドごとに推論器を1つ作る関数。ortのセッションはスレッドの中で作る。
pub(super) type EvaluatorFactory<'a> = dyn Fn() -> io::Result<Box<dyn Evaluator>> + Sync + 'a;

/// 終局理由の内訳の名前。サマリには0件の理由も載せる。
const ENDINGS: [&str; 11] = [
    "win_royal_capture",
    "win_mate",
    "win_stalemate",
    "win_repetition",
    "win_piece_exhaustion",
    "win_bare_king",
    "win_resignation",
    "draw_repetition",
    "draw_piece_exhaustion",
    "draw_bare_king",
    "draw_agreement",
];

/// すべての塊に共通する生成の条件。
pub(super) struct Context<'a> {
    /// 対局の設定。
    pub(super) settings: GameSettings,
    /// 実戦開始の局面一覧。
    pub(super) openings: Option<&'a [Opening]>,
    /// 同時に進める対局数。
    pub(super) games_in_flight: NonZeroUsize,
    /// 対局の集合を受け持つスレッドの数。
    pub(super) threads: NonZeroUsize,
    /// 1回の推論にまとめる局面数の上限。
    pub(super) nn_batch_size: NonZeroUsize,
    /// 塊を置くディレクトリ。
    pub(super) out_dir: &'a Path,
    /// MNZTのヘッダの規則セット名。
    pub(super) rule_set: &'a str,
    /// 生成コミット。
    pub(super) generation_commit: &'a str,
    /// 推論に使うネットの検査和。
    pub(super) network_checksum: [u8; 32],
    /// 推論器を作る関数。
    pub(super) evaluator: &'a EvaluatorFactory<'a>,
}

impl Context<'_> {
    /// 塊のMNZTのヘッダを作る。
    pub(super) fn header(&self, chunk_seed: u64) -> io::Result<mnzt::Header> {
        mnzt::Header::new(
            self.rule_set.to_owned(),
            self.generation_commit.to_owned(),
            self.network_checksum,
            mnzt::Teacher::MctsSimulations(u64::from(self.settings.simulations.get())),
            chunk_seed,
        )
        .map_err(data_error)
    }
}

/// 1つの塊の番号、シード、および対局数。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct ChunkPlan {
    /// 0から始まる塊の番号。
    pub(super) index: u32,
    /// 塊のシード。局のシードはここから対局番号で派生させる。
    pub(super) seed: u64,
    /// 塊の対局数。
    pub(super) games: u32,
    /// 塊の最初の対局の、サイクル全体での0から始まる通し番号。実戦開始の局面の選択に使う。
    pub(super) first_game: u64,
}

impl ChunkPlan {
    /// 塊のファイルの経路を返す。`extension`は`mnzt`や`json`のような拡張子である。
    pub(super) fn path(&self, out_dir: &Path, extension: &str) -> PathBuf {
        out_dir.join(format!("chunk-{:05}.{extension}", self.index))
    }
}

/// 塊の間で足し合わせる件数。
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub(super) struct Counts {
    /// 対局数。
    pub(super) games: u64,
    /// 学習データに書いた局面数。
    pub(super) positions: u64,
    /// 規則または投了による終局の理由ごとの対局数。
    pub(super) endings: BTreeMap<String, u64>,
    /// 手数の上限に達して学習データから除いた対局数。
    pub(super) max_ply_games: u64,
    /// 投了で終わった対局数。
    pub(super) resigned_games: u64,
    /// 投了せずに最後まで指させた対局数。
    pub(super) playthrough_games: u64,
    /// 誤った投了の割合の分母。最後まで指させた対局のうち、投了の条件がいずれかの側で成立した対局数。
    pub(super) false_resign_candidates: u64,
    /// 誤った投了の割合の分子。分母の対局のうち、条件が成立した側が負けなかった対局数。
    pub(super) false_resignations: u64,
    /// 開始局面の元の手数を含めた終局時の手数の合計。
    pub(super) total_plies: u64,
}

impl Counts {
    /// すべての件数が0の集計を作る。
    pub(super) fn new() -> Self {
        Self {
            games: 0,
            positions: 0,
            endings: ENDINGS.iter().map(|&name| (name.to_owned(), 0)).collect(),
            max_ply_games: 0,
            resigned_games: 0,
            playthrough_games: 0,
            false_resign_candidates: 0,
            false_resignations: 0,
            total_plies: 0,
        }
    }

    /// 終えた1局を集計に加える。
    pub(super) fn add(&mut self, game: &FinishedGame) {
        self.games += 1;
        self.positions += game.records.len() as u64;
        self.total_plies += u64::from(game.plies);
        match game.ending {
            Ending::Finished(result) => {
                *self
                    .endings
                    .entry(ending_name(result).to_owned())
                    .or_insert(0) += 1;
                if matches!(
                    result,
                    GameResult::Win {
                        reason: WinReason::Resignation,
                        ..
                    }
                ) {
                    self.resigned_games += 1;
                }
            }
            Ending::MaxPly => self.max_ply_games += 1,
        }
        if game.playthrough {
            self.playthrough_games += 1;
            if let Some(side) = game.resign_trigger {
                self.false_resign_candidates += 1;
                if !lost(game.ending, side) {
                    self.false_resignations += 1;
                }
            }
        }
    }

    /// 別の集計を加える。
    pub(super) fn merge(&mut self, other: &Self) {
        self.games += other.games;
        self.positions += other.positions;
        for (name, count) in &other.endings {
            *self.endings.entry(name.clone()).or_insert(0) += count;
        }
        self.max_ply_games += other.max_ply_games;
        self.resigned_games += other.resigned_games;
        self.playthrough_games += other.playthrough_games;
        self.false_resign_candidates += other.false_resign_candidates;
        self.false_resignations += other.false_resignations;
        self.total_plies += other.total_plies;
    }

    /// 平均手数を返す。対局がなければ0とする。
    pub(super) fn average_plies(&self) -> f64 {
        ratio(self.total_plies, self.games)
    }
}

/// 塊ごとのサマリ`chunk-<番号>.json`。
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub(super) struct ChunkSummary {
    /// 0から始まる塊の番号。
    pub(super) chunk: u32,
    /// 塊のシード。
    pub(super) seed: u64,
    /// 件数。
    #[serde(flatten)]
    pub(super) counts: Counts,
    /// 平均手数。
    pub(super) average_plies: f64,
    /// 塊の生成の経過時間。
    pub(super) elapsed_seconds: f64,
    /// 毎秒の局面数。
    pub(super) positions_per_second: f64,
}

/// 規則または投了による終局の理由の名前を返す。
fn ending_name(result: GameResult) -> &'static str {
    match result {
        GameResult::Win { reason, .. } => match reason {
            WinReason::RoyalCapture => ENDINGS[0],
            WinReason::Mate => ENDINGS[1],
            WinReason::Stalemate => ENDINGS[2],
            WinReason::Repetition => ENDINGS[3],
            WinReason::PieceExhaustion => ENDINGS[4],
            WinReason::BareKing => ENDINGS[5],
            WinReason::Resignation => ENDINGS[6],
        },
        GameResult::Draw { reason } => match reason {
            DrawReason::Repetition => ENDINGS[7],
            DrawReason::PieceExhaustion => ENDINGS[8],
            DrawReason::BareKing => ENDINGS[9],
            DrawReason::Agreement => ENDINGS[10],
        },
    }
}

/// `side`がその対局に負けたかを返す。手数の上限に達した対局は負けとしない。
fn lost(ending: Ending, side: Color) -> bool {
    matches!(ending, Ending::Finished(GameResult::Win { winner, .. }) if winner != side)
}

/// 分子を分母で割る。分母が0なら0とする。
pub(super) fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

/// 塊を生成し、MNZT、サマリの順に確定させてサマリを返す。
///
/// MNZTは`.mnzt.tmp`へ書いて同期し、サマリを`.json.tmp`から名前を変えて確定させた後、
/// 最後にMNZTの名前を変える。確定したMNZTがあれば、その塊のサマリも確定している。
/// 失敗した場合は一時ファイルを消す。
pub(super) fn generate_chunk(context: &Context<'_>, plan: &ChunkPlan) -> io::Result<ChunkSummary> {
    let started = Instant::now();
    let temporary = plan.path(context.out_dir, "mnzt.tmp");
    let scratch = plan.path(context.out_dir, "mnzt.policy.tmp");
    let summary_temporary = plan.path(context.out_dir, "json.tmp");
    let written = write_chunk(context, plan, &temporary, &scratch);
    // 方策の列の一時置き場は成否にかかわらず消す。
    let removed = fs::remove_file(&scratch);
    let result = written.and_then(|counts| {
        removed?;
        let elapsed_seconds = started.elapsed().as_secs_f64();
        let summary = ChunkSummary {
            chunk: plan.index,
            seed: plan.seed,
            average_plies: counts.average_plies(),
            elapsed_seconds,
            positions_per_second: if elapsed_seconds > 0.0 {
                counts.positions as f64 / elapsed_seconds
            } else {
                0.0
            },
            counts,
        };
        let mut file = BufWriter::new(File::create(&summary_temporary)?);
        serde_json::to_writer_pretty(&mut file, &summary).map_err(data_error)?;
        file.write_all(b"\n")?;
        file.into_inner()
            .map_err(io::IntoInnerError::into_error)?
            .sync_all()?;
        fs::rename(&summary_temporary, plan.path(context.out_dir, "json"))?;
        fs::rename(&temporary, plan.path(context.out_dir, "mnzt"))?;
        Ok(summary)
    });
    if result.is_err() {
        for path in [&temporary, &summary_temporary] {
            let _ = fs::remove_file(path);
        }
    }
    result
}

/// 塊の対局を進め、対局番号の順にMNZTの一時ファイルへ書いて同期し、件数を返す。
fn write_chunk(
    context: &Context<'_>,
    plan: &ChunkPlan,
    temporary: &Path,
    scratch: &Path,
) -> io::Result<Counts> {
    let output = File::create(temporary)?;
    let scratch = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(scratch)?;
    let mut writer =
        mnzt::Writer::new(output, scratch, context.header(plan.seed)?).map_err(data_error)?;
    let mut counts = Counts::new();
    play_chunk(context, plan, |game| {
        for record in &game.records {
            writer.write_record(record).map_err(data_error)?;
        }
        counts.add(game);
        Ok(())
    })?;
    writer.finish().map_err(data_error)?.sync_all()?;
    Ok(counts)
}

/// 塊の対局をスレッドに分けて進め、終えた対局を対局番号の順に`on_game`へ渡す。
fn play_chunk(
    context: &Context<'_>,
    plan: &ChunkPlan,
    mut on_game: impl FnMut(&FinishedGame) -> io::Result<()>,
) -> io::Result<()> {
    let threads = context.threads.get();
    let in_flight = context.games_in_flight.get();
    let next_game = AtomicU64::new(1);
    let abort = AtomicBool::new(false);
    let (sender, receiver) = mpsc::channel::<io::Result<FinishedGame>>();
    thread::scope(|scope| {
        for thread_index in 0..threads {
            let slots = in_flight / threads + usize::from(thread_index < in_flight % threads);
            let sender = sender.clone();
            let (next_game, abort) = (&next_game, &abort);
            scope.spawn(move || {
                if let Err(error) = run_worker(context, plan, slots, next_game, abort, &sender) {
                    abort.store(true, Ordering::Relaxed);
                    let _ = sender.send(Err(error));
                }
            });
        }
        drop(sender);

        let mut pending = BTreeMap::new();
        let mut next_to_write = 1_u32;
        let mut failure = None;
        for message in receiver {
            match message {
                Ok(game) if failure.is_none() => {
                    pending.insert(game.number, game);
                    while let Some(game) = pending.remove(&next_to_write) {
                        if let Err(error) = on_game(&game) {
                            abort.store(true, Ordering::Relaxed);
                            failure = Some(error);
                            break;
                        }
                        next_to_write += 1;
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    abort.store(true, Ordering::Relaxed);
                    failure.get_or_insert(error);
                }
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        if next_to_write != plan.games + 1 {
            return Err(invalid_data(format!(
                "chunk {} finished {} of {} games",
                plan.index,
                next_to_write - 1,
                plan.games
            )));
        }
        Ok(())
    })
}

/// 1つのスレッドが`slot_count`局を同時に進め、評価が必要な葉をまとめて推論にかける。
///
/// 空いた枠には次の対局番号の対局を入れる。各局の進行は局のシードだけで決まるので、
/// どのスレッドがどの対局を受け持っても、対局の内容は変わらない。
fn run_worker(
    context: &Context<'_>,
    plan: &ChunkPlan,
    slot_count: usize,
    next_game: &AtomicU64,
    abort: &AtomicBool,
    sender: &mpsc::Sender<io::Result<FinishedGame>>,
) -> io::Result<()> {
    let mut evaluator = (context.evaluator)()?;
    let mut slots: Vec<Option<SelfPlayGame>> = (0..slot_count).map(|_| None).collect();
    let mut exhausted = false;
    let mut owners = Vec::with_capacity(slot_count);
    let (mut inputs, mut policy, mut values) = (Vec::new(), Vec::new(), Vec::new());
    let settings = &context.settings;
    while !abort.load(Ordering::Relaxed) {
        owners.clear();
        for (index, slot) in slots.iter_mut().enumerate() {
            loop {
                if slot.is_none() && !exhausted {
                    let number = next_game.fetch_add(1, Ordering::Relaxed);
                    match u32::try_from(number) {
                        Ok(number) if number <= plan.games => {
                            *slot = Some(start_game(context, plan, number)?);
                        }
                        _ => exhausted = true,
                    }
                }
                let Some(game) = slot.as_mut() else {
                    break;
                };
                if game.advance(settings)? {
                    owners.push(index);
                    break;
                }
                if let Some(game) = slot.take()
                    && sender.send(Ok(game.finish()?)).is_err()
                {
                    return Ok(());
                }
            }
        }
        if owners.is_empty() {
            return Ok(());
        }
        inputs.clear();
        for game in owners.iter().filter_map(|&index| slots[index].as_ref()) {
            let input = game.pending_input();
            if input.len() != INPUT_SIZE {
                return Err(invalid_data(format!(
                    "a search returned {} input values instead of {INPUT_SIZE}",
                    input.len()
                )));
            }
            inputs.extend_from_slice(input);
        }
        let batch = owners.len();
        policy.resize(batch * POLICY_SIZE, 0.0);
        values.resize(batch, 0.0);
        let step = context.nn_batch_size.get();
        for start in (0..batch).step_by(step) {
            let end = (start + step).min(batch);
            evaluator
                .evaluate(
                    &inputs[start * INPUT_SIZE..end * INPUT_SIZE],
                    end - start,
                    &mut policy[start * POLICY_SIZE..end * POLICY_SIZE],
                    &mut values[start..end],
                )
                .map_err(|error| invalid_data(error.to_string()))?;
        }
        for (row, &index) in owners.iter().enumerate() {
            if let Some(game) = slots[index].as_mut() {
                game.supply(
                    &policy[row * POLICY_SIZE..(row + 1) * POLICY_SIZE],
                    values[row],
                )?;
            }
        }
    }
    Ok(())
}

/// 塊の`number`番目の対局を、局のシードと開始局面から始める。
fn start_game(context: &Context<'_>, plan: &ChunkPlan, number: u32) -> io::Result<SelfPlayGame> {
    let opening = context.openings.map(|openings| {
        let index = (plan.first_game + u64::from(number) - 1) % openings.len() as u64;
        &openings[index as usize]
    });
    SelfPlayGame::new(
        number,
        derive_seed(plan.seed, u64::from(number)),
        opening,
        &context.settings,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::{Color, DrawReason};

    /// 試験用の終えた対局を作る。
    fn finished(ending: Ending, playthrough: bool, trigger: Option<Color>) -> FinishedGame {
        FinishedGame {
            number: 1,
            records: Vec::new(),
            ending,
            plies: 10,
            playthrough,
            resign_trigger: trigger,
        }
    }

    /// alphazero.md「強化学習」: 誤った投了の割合は、最後まで指させた対局のうち投了の条件が成立した
    /// 対局を分母、条件が成立した側が負けなかった対局（勝ち、引き分け、手数上限への到達）を分子とする。
    #[test]
    fn false_resignations_count_games_the_triggering_side_did_not_lose() {
        let black_wins = Ending::Finished(GameResult::Win {
            winner: Color::Black,
            reason: WinReason::RoyalCapture,
        });
        let draw = Ending::Finished(GameResult::Draw {
            reason: DrawReason::Repetition,
        });
        let resigned = Ending::Finished(GameResult::Win {
            winner: Color::White,
            reason: WinReason::Resignation,
        });
        let mut counts = Counts::new();
        for game in [
            // 分母と分子の両方に入る：条件が成立した先手が勝った、引き分けた、手数上限に達した。
            finished(black_wins, true, Some(Color::Black)),
            finished(draw, true, Some(Color::Black)),
            finished(Ending::MaxPly, true, Some(Color::White)),
            // 分母だけに入る：条件が成立した後手が負けた。
            finished(black_wins, true, Some(Color::White)),
            // どちらにも入らない：条件が成立しなかった、または投了を許した対局。
            finished(black_wins, true, None),
            finished(resigned, false, None),
        ] {
            counts.add(&game);
        }
        assert_eq!(counts.games, 6);
        assert_eq!(counts.playthrough_games, 5);
        assert_eq!(counts.false_resign_candidates, 4);
        assert_eq!(counts.false_resignations, 3);
        assert_eq!(counts.resigned_games, 1);
        assert_eq!(counts.max_ply_games, 1);
        assert_eq!(counts.endings["win_royal_capture"], 3);
        assert_eq!(counts.endings["draw_repetition"], 1);
        assert_eq!(counts.endings["win_resignation"], 1);
        assert_eq!(counts.average_plies(), 10.0);

        let mut merged = Counts::new();
        merged.merge(&counts);
        merged.merge(&counts);
        assert_eq!(merged.false_resign_candidates, 8);
        assert_eq!(merged.endings["win_royal_capture"], 6);
    }
}
