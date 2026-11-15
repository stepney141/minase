//! 選別済みのlishogi棋譜を再生し、探索なしの検証用MNSDへ書き出す。
//!
//! 入力はUTF-8の1行1局のJSONで、`input::parse_line`の契約に従う。
//! 棋譜idは空白を含まない空でない文字列で、ファイル内で重複しない。
//! `initialSfen`欄はnullでも認めず、`status`は`resign`、`royalsLost`、`bareKing`、
//! `winner`は`sente`または`gote`に限る。違反は行番号つきのエラーで処理を止める。
//! レーティングなどの選別は`scripts/select_validation_games.py`が行う。
//!
//! 各対局を[`Rules::LISHOGI`]の初期局面から再生する。解析または適用に失敗した手は
//! `illegal_move`、残る手がある時点の終局は`finished_early`として除外する。
//! 全手を再生した後、王駒の捕獲と裸玉は記録の勝者と理由が一致すること、投了は未終局であることを
//! 求め、違えば`result_mismatch`とする。投了の勝者は手番によらず記録どおりに採る。
//!
//! 受理した対局を入力順の1起算の対局番号で保存する。教師の種類は[`TeacherKind::NoSearch`]、
//! 検査和と探索規模は0、由来は[`Origin::HumanGame`]とし、初期局面から全着手を保存する。
//! 各着手の項目は価値、訪問回数、除外理由を持たず、方策は[`Policy::PlayedMove`]である。
//! 最後に、着手も方策もない項目を1つ置くので、項目数は手数+1になる。
//!
//! JSON報告は`input_games`に入力の行数、`accepted`に受理件数、`accepted_ids`に対局番号順のidを置く。
//! `excluded`は上記3理由をすべて持ち、各理由の`count`と入力順の`ids`は0件でも省略しない。
//! 出力と報告は新規作成に限り、失敗時にはこの実行で作ったファイルを削除する。

use super::{ImportError, filter::winner, input, output};
use minase::{
    datagen::{self, data_error},
    training::records::{
        Entry, Exclusion, GameRecord, Header, Origin, Outcome, Policy, TeacherKind, Value, Writer,
    },
};
use minase_core::{Game, GameResult, Rules, WinReason, notation::usi};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::{self, BufRead, BufReader},
    num::NonZeroU64,
    path::PathBuf,
};

#[derive(clap::Args)]
pub(super) struct Arguments {
    /// 選別済みの1行1局のゲームJSON。
    #[arg(long)]
    input: PathBuf,
    /// 新規作成する探索なしのMNSD。
    #[arg(long)]
    output: PathBuf,
    /// 受理と除外の件数および棋譜idを保存するJSON。
    #[arg(long)]
    report: PathBuf,
    /// 他の記録ファイルと重ならない、0以外の識別用シード。
    #[arg(long)]
    seed: NonZeroU64,
    /// 変更のある作業ツリーでの生成を許可する。
    #[arg(long)]
    allow_dirty: bool,
}

#[derive(Debug)]
enum ContractError {
    DuplicateId { line: usize, id: String },
    InitialSfen { line: usize },
    Status { line: usize },
    Winner { line: usize },
    OutputExists { path: PathBuf },
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateId { line, id } => {
                write!(f, "NDJSON line {line}: duplicate game id {id}")
            }
            Self::InitialSfen { line } => {
                write!(f, "NDJSON line {line}: initialSfen must be absent")
            }
            Self::Status { line } => write!(
                f,
                "NDJSON line {line}: status must be resign, royalsLost or bareKing"
            ),
            Self::Winner { line } => write!(f, "NDJSON line {line}: winner must be sente or gote"),
            Self::OutputExists { path } => write!(f, "output already exists: {}", path.display()),
        }
    }
}
impl std::error::Error for ContractError {}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReplayExclusion {
    IllegalMove,
    FinishedEarly,
    ResultMismatch,
}

impl ReplayExclusion {
    const ALL: [Self; 3] = [Self::IllegalMove, Self::FinishedEarly, Self::ResultMismatch];

    fn name(self) -> &'static str {
        match self {
            Self::IllegalMove => "illegal_move",
            Self::FinishedEarly => "finished_early",
            Self::ResultMismatch => "result_mismatch",
        }
    }
}

#[derive(Default, Serialize)]
struct Excluded {
    count: usize,
    ids: Vec<String>,
}

#[derive(Serialize)]
struct Report {
    input_games: usize,
    accepted: usize,
    accepted_ids: Vec<String>,
    excluded: BTreeMap<ReplayExclusion, Excluded>,
}

/// 入力の契約を検査し、記録すべき勝敗を返す。
fn recorded_result(input: &input::InputGame, line: usize) -> io::Result<GameResult> {
    if input.initial_sfen.is_some() {
        return Err(data_error(ContractError::InitialSfen { line }));
    }
    let reason = match input.status.as_deref() {
        Some("resign") => WinReason::Resignation,
        Some("royalsLost") => WinReason::RoyalCapture,
        Some("bareKing") => WinReason::BareKing,
        _ => return Err(data_error(ContractError::Status { line })),
    };
    let winner = winner(input).ok_or_else(|| data_error(ContractError::Winner { line }))?;
    Ok(GameResult::Win { winner, reason })
}

/// lishogi規則で全手と終局を照合し、指した手の列を返す。
fn replay(
    input: &input::InputGame,
    expected: GameResult,
) -> Result<Vec<minase_core::Move>, ReplayExclusion> {
    let mut game = Game::new(Rules::LISHOGI);
    let mut moves = input.moves.split_whitespace().peekable();
    let mut played = Vec::new();
    while let Some(text) = moves.next() {
        let mv = usi::parse(game.position(), text).map_err(|_| ReplayExclusion::IllegalMove)?;
        game.play(mv).map_err(|_| ReplayExclusion::IllegalMove)?;
        played.push(mv);
        if game.result().is_some() && moves.peek().is_some() {
            return Err(ReplayExclusion::FinishedEarly);
        }
    }
    let matching = if matches!(
        expected,
        GameResult::Win {
            reason: WinReason::Resignation,
            ..
        }
    ) {
        game.result().is_none()
    } else {
        game.result() == Some(expected)
    };
    if !matching {
        return Err(ReplayExclusion::ResultMismatch);
    }
    Ok(played)
}

pub(super) fn run(arguments: Arguments) -> io::Result<()> {
    for path in [&arguments.output, &arguments.report] {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                return Err(data_error(ContractError::OutputExists {
                    path: path.clone(),
                }));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    if !arguments.allow_dirty && !datagen::git::git_output(&["status", "--porcelain"])?.is_empty() {
        return Err(data_error(ImportError::DirtyTree));
    }
    let header = Header::new(
        Rules::LISHOGI.to_string(),
        datagen::git::git_output(&["rev-parse", "HEAD"])?,
        [0; 32],
        TeacherKind::NoSearch,
        0,
        arguments.seed.get(),
    )
    .map_err(data_error)?;
    let source = BufReader::new(File::open(&arguments.input)?);
    let mut report = Report {
        input_games: 0,
        accepted: 0,
        accepted_ids: Vec::new(),
        excluded: ReplayExclusion::ALL
            .into_iter()
            .map(|reason| (reason, Excluded::default()))
            .collect(),
    };
    let mut seen = HashSet::new();
    let mut created = Vec::new();
    let result = (|| {
        let file = output::create(&arguments.output)?;
        created.push(&arguments.output);
        let mut writer = Writer::new(file, header).map_err(data_error)?;
        for (index, line) in source.lines().enumerate() {
            let line_number = index + 1;
            let input = input::parse_line(line, line_number)?;
            report.input_games += 1;
            if !seen.insert(input.id.clone()) {
                return Err(data_error(ContractError::DuplicateId {
                    line: line_number,
                    id: input.id,
                }));
            }
            let expected = recorded_result(&input, line_number)?;
            let moves = match replay(&input, expected) {
                Ok(moves) => moves,
                Err(reason) => {
                    let entry = report.excluded.entry(reason).or_default();
                    entry.count += 1;
                    entry.ids.push(input.id);
                    continue;
                }
            };
            let mut entries = moves
                .into_iter()
                .map(|mv| {
                    Entry::new(
                        Some(mv),
                        Value::None,
                        Exclusion::None,
                        Policy::PlayedMove,
                        None,
                    )
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(data_error)?;
            entries.push(
                Entry::new(None, Value::None, Exclusion::None, Policy::None, None)
                    .map_err(data_error)?,
            );
            let number = u32::try_from(report.accepted + 1).map_err(data_error)?;
            let record = GameRecord::new(
                number,
                Origin::HumanGame,
                Outcome::Game(expected),
                0,
                Game::new(Rules::LISHOGI).position().clone(),
                entries,
            )
            .map_err(data_error)?;
            writer.write_game(&record).map_err(data_error)?;
            report.accepted += 1;
            report.accepted_ids.push(input.id);
        }
        writer.finish().map_err(data_error)?.sync_all()?;
        output::write_json(&arguments.report, &report)?;
        created.push(&arguments.report);
        Ok(())
    })();
    if result.is_err() {
        for path in created {
            fs::remove_file(path)?;
        }
    }
    result?;
    println!("accepted: {}", report.accepted);
    for (reason, excluded) in report.excluded {
        println!("{}: {}", reason.name(), excluded.count);
    }
    Ok(())
}
