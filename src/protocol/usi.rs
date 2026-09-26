//! Universal Shogi Interfaceのアダプター。

use std::collections::VecDeque;
use std::io::{self, BufRead, Write};
use std::num::NonZeroUsize;
#[cfg(feature = "tuning")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use crate::core::game::{DrawReason, Game, GameResult, GameStatus, WinReason};
use crate::core::mv::Move;
use crate::core::piece::Color;
use crate::core::position::Position;
use crate::core::rules::parse_rule_set;
use crate::notation::sfen::{SetupPosition, parse_extended_sfen, to_sfen};
use crate::notation::usi;
use crate::search::{
    self, ClockLimits, SearchEvent, SearchHandle, SearchLimits, SearchSnapshot, StopReason,
    TranspositionTable,
};

use super::Protocol;
use super::engine::{
    Engine, EngineCommand, EngineLifecycle, EngineReply, RejectReason, canonical_rules_text,
};

/// USIから設定できる置換表容量の上限(MiB)。
const MAX_HASH_SIZE_MB: usize = 65_536;

/// 採用結果の評価値に適用する既定の投了閾値(cp)。
const DEFAULT_RESIGN_VALUE: i32 = 20_000;

/// プロセス内で一度でもgoを受信したら調整係数を固定する。
#[cfg(feature = "tuning")]
static TUNING_LOCKED: AtomicBool = AtomicBool::new(false);

/// lishogi系拡張を含むUSIプロトコル。
pub struct UsiProtocol {
    /// `option`宣言のdefault値に使う起動時規則の正準表記。
    startup_rules_text: String,
    /// 探索間で引き継ぐ置換表。探索スレッドへ貸し出している間は`None`。
    transposition_table: Option<TranspositionTable>,
    /// 最後の`position`コマンドが受理され、局面が同期済みかどうか。
    position_synchronized: bool,
    /// 最後に受理した`position`の開始局面と着手のトークン列。
    accepted_position: Option<AcceptedPosition>,
    /// 次に開始する探索へ割り当てる識別子。
    next_search_id: u64,
    /// 次の探索に使うワーカー数。
    threads: NonZeroUsize,
    /// 完了深さが1以上の採用結果に適用する投了閾値(cp)。
    resign_value: i32,
}

/// 最後に受理した`position`のトークン列。
struct AcceptedPosition {
    /// `startpos`または`sfen`から`moves`直前までのトークン列。
    setup_tokens: Vec<String>,
    /// `moves`以降の着手トークン列。
    move_tokens: Vec<String>,
}

/// 受理した`position`に応じたトークン履歴の更新方法。
enum PositionHistoryUpdate<'a> {
    /// 全再生後にトークン列全体を置き換える。
    Replace {
        /// 開始局面のトークン列。
        setup_tokens: &'a [&'a str],
        /// 着手のトークン列。
        move_tokens: &'a [&'a str],
    },
    /// 差分適用後に追加分だけを連結する。
    Extend(&'a [&'a str]),
    /// 末尾のトークンだけを交換する。
    Last(&'a str),
}

/// 実行中の探索に対応する局面と設定。
struct SearchContext {
    /// 探索イベントの照合に使う探索識別子。
    id: u64,
    /// 探索開始時の局面。`bestmove`と`info`の指し手表記に使う。
    position: Position,
    /// 探索開始時の採用規則。
    rules: crate::Rules,
    /// `go infinite`による探索かどうか。
    infinite: bool,
    /// 的中または停止まで結果を保留する先読みか。
    ponder: bool,
    /// 最後に`info`として出力した完了深さ。
    last_info_depth: Option<u32>,
}

/// 探索の進行状態。
enum ActiveSearch {
    /// 探索スレッドが実行中。
    Running {
        /// 探索の局面と設定。
        context: SearchContext,
        /// 探索スレッドへのハンドル。
        handle: SearchHandle,
    },
    /// 無限探索または先読みが完了し、停止または的中まで結果を保留する状態。
    AwaitingStop {
        /// 探索の局面と設定。
        context: SearchContext,
        /// `stop`受信時に返す最善手。
        best_move: Move,
        /// 完了したPVから検査済みの予想手表記。
        ponder_move: Option<String>,
        /// 採用結果の根の手番側から見た評価値。
        score: i32,
        /// 採用結果の完了した深さ。
        depth: u32,
        /// 探索を停止した条件。
        stop_reason: StopReason,
    },
}

/// 待機状態での1行の処理結果。
enum LineAction {
    /// 待機状態を継続する。
    Continue,
    /// 探索を開始する。
    Start(Box<ActiveSearch>),
    /// セッションを終了する。
    Quit,
}

impl UsiProtocol {
    /// エンジンの起動時active規則をoption宣言の正準default値として保持する。
    ///
    /// セッション開始前に構築することで、宣言値と状態機械の起動時規則を
    /// 異なる値から作れないようにする。
    pub fn new(engine: &Engine) -> Self {
        Self {
            startup_rules_text: canonical_rules_text(engine.active_rule_codes()),
            transposition_table: None,
            position_synchronized: false,
            accepted_position: None,
            next_search_id: 1,
            threads: search::DEFAULT_THREADS,
            resign_value: DEFAULT_RESIGN_VALUE,
        }
    }

    /// 探索していない待機状態で1コマンドを処理する。
    fn handle_idle_line(
        &mut self,
        engine: &mut Engine,
        line: &str,
        output: &mut dyn Write,
    ) -> io::Result<LineAction> {
        let tokens: Vec<_> = line.split_whitespace().collect();
        let Some(command) = tokens.first().copied() else {
            return Ok(LineAction::Continue);
        };

        #[cfg(feature = "tuning")]
        if command == "go" {
            TUNING_LOCKED.store(true, Ordering::Relaxed);
        }

        match command {
            "usi" => self.write_handshake(output)?,
            "isready" => {
                // 置換表の確保と初期化を最初の`go`の前に済ませ、対局開始直後の手の
                // 思考時間に数えられないようにする（time-management-efficiency.md）。
                // 確保に失敗した場合は`go`で改めて試み、そこで報告する。
                if self.transposition_table.is_none()
                    && let Ok(table) = TranspositionTable::new(search::DEFAULT_TT_SIZE_MB)
                {
                    self.transposition_table = Some(table);
                }
                writeln!(output, "readyok")?;
            }
            "setoption" => self.handle_setoption(engine, &tokens[1..], output)?,
            "usinewgame" => {
                self.apply_silent(engine, EngineCommand::NewGame, output)?;
            }
            "position" => self.handle_position(engine, &tokens[1..], output)?,
            "gameover" => {
                self.apply_silent(engine, EngineCommand::EndGame, output)?;
            }
            "moves" => self.handle_moves(engine, output)?,
            "state" => self.handle_state(engine, output)?,
            "ponderhit" => write_error(output, "ponderhit requires an active ponder search")?,
            "go" if tokens[1..].contains(&"mate") => {
                writeln!(output, "checkmate notimplemented")?;
            }
            "go" => {
                let Some(search) = self.start_go(engine, &tokens[1..], output)? else {
                    output.flush()?;
                    return Ok(LineAction::Continue);
                };
                output.flush()?;
                return Ok(LineAction::Start(Box::new(search)));
            }
            "quit" => {
                let _ = engine.handle(EngineCommand::Quit);
                return Ok(LineAction::Quit);
            }
            _ => {}
        }
        output.flush()?;
        Ok(LineAction::Continue)
    }

    /// `go`の引数を検証し、非同期探索を開始する。
    ///
    /// 前提条件を満たさない場合はエラーを出力して`None`を返す。
    fn start_go(
        &mut self,
        engine: &Engine,
        tokens: &[&str],
        output: &mut dyn Write,
    ) -> io::Result<Option<ActiveSearch>> {
        if engine.lifecycle() != EngineLifecycle::InGame {
            write_error(output, "go requires an active game")?;
            return Ok(None);
        }
        if !self.position_synchronized {
            write_error(output, "go requires a synchronized position")?;
            return Ok(None);
        }
        let game = engine.game();
        let config = match parse_go_config(tokens, game.position().side_to_move(), engine.ply()) {
            Ok(config) => config,
            Err(error) => {
                write_error(output, &error)?;
                return Ok(None);
            }
        };
        let position = game.position().clone();
        let rules = engine.active_rules();
        let snapshot = match SearchSnapshot::from_game(game) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                write_error(output, &error.to_string())?;
                return Ok(None);
            }
        };
        let search_id = self.next_search_id;
        self.next_search_id = self.next_search_id.wrapping_add(1);
        let pst = match crate::eval::weights() {
            Ok(pst) => pst,
            Err(error) => {
                write_error(output, &error.to_string())?;
                return Ok(None);
            }
        };
        let transposition_table = match self.transposition_table.take() {
            Some(table) => table,
            None => match TranspositionTable::new(search::DEFAULT_TT_SIZE_MB) {
                Ok(table) => table,
                Err(error) => {
                    write_error(output, &error.to_string())?;
                    return Ok(None);
                }
            },
        };
        let infinite = config.is_infinite();
        let handle = search::start_search(
            pst,
            snapshot,
            config,
            search_id,
            self.threads,
            transposition_table,
            tokens.contains(&"ponder"),
        );
        Ok(Some(ActiveSearch::Running {
            context: SearchContext {
                id: search_id,
                position,
                rules,
                infinite,
                ponder: tokens.contains(&"ponder"),
                last_info_depth: None,
            },
            handle,
        }))
    }

    /// reader threadが送るUSI入力と探索イベントを並行して処理する。
    ///
    /// 各入力要素は改行を除いた1コマンドとする。送信側がdropされた
    /// 場合は、有限探索の完了を待ってからセッションを終了する。
    pub fn run_channel(
        &mut self,
        engine: &mut Engine,
        input: &Receiver<io::Result<String>>,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let mut active = None;
        let mut pending = VecDeque::new();
        let mut input_open = true;

        loop {
            if active.is_none() {
                let line = if let Some(line) = pending.pop_front() {
                    line
                } else if input_open {
                    match input.recv() {
                        Ok(Ok(line)) => line,
                        Ok(Err(error)) => return Err(error),
                        Err(_) => break,
                    }
                } else {
                    break;
                };
                match self.handle_idle_line(engine, line.trim_end(), output)? {
                    LineAction::Continue => {}
                    LineAction::Start(search) => active = Some(*search),
                    LineAction::Quit => break,
                }
                continue;
            }

            // 探索中は入力を優先して処理し、なければ探索イベントを刈り取る。
            if input_open {
                match input.try_recv() {
                    Ok(Ok(line)) => {
                        self.handle_searching_line(
                            engine,
                            &mut active,
                            &mut pending,
                            line.trim_end(),
                            output,
                        )?;
                        continue;
                    }
                    Ok(Err(error)) => {
                        self.discard_search(&mut active)?;
                        return Err(error);
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => input_open = false,
                }
            }

            self.poll_search(engine, &mut active, output)?;
            if active.is_none() {
                continue;
            }

            // 入力もイベントもない間は、短い待ちで両者を交互に見張る。
            if input_open {
                match input.recv_timeout(Duration::from_millis(10)) {
                    Ok(Ok(line)) => self.handle_searching_line(
                        engine,
                        &mut active,
                        &mut pending,
                        line.trim_end(),
                        output,
                    )?,
                    Ok(Err(error)) => {
                        self.discard_search(&mut active)?;
                        return Err(error);
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => input_open = false,
                }
            } else if active
                .as_ref()
                .is_some_and(ActiveSearch::withholds_bestmove)
            {
                // 入力が閉じると停止も的中も届かないため、結果を保留する探索は破棄する。
                self.discard_search(&mut active)?;
            } else {
                self.wait_search_event(engine, &mut active, output)?;
            }
        }

        if active.is_some() {
            self.discard_search(&mut active)?;
        }
        Ok(())
    }

    /// 探索中に届いた1コマンドを処理する。
    ///
    /// `stop`と`gameover`・`quit`は探索を終わらせ、探索と無関係な
    /// コマンドは探索終了後に処理するため`pending`へ積む。
    fn handle_searching_line(
        &mut self,
        engine: &Engine,
        active: &mut Option<ActiveSearch>,
        pending: &mut VecDeque<String>,
        line: &str,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let command = line.split_whitespace().next();
        match command {
            Some("stop") => self.stop_search(engine, active, output)?,
            Some("gameover" | "quit") => {
                pending.push_back(line.to_owned());
                self.discard_search(active)?;
            }
            Some("go") => write_error(output, "go is already running")?,
            Some("ponderhit") => self.ponderhit(engine, active, output)?,
            _ => pending.push_back(line.to_owned()),
        }
        output.flush()
    }

    /// 溜まっている探索イベントをブロックせずにすべて処理する。
    fn poll_search(
        &mut self,
        engine: &Engine,
        active: &mut Option<ActiveSearch>,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        loop {
            let event = match active.as_ref() {
                Some(ActiveSearch::Running { handle, .. }) => match handle.events().try_recv() {
                    Ok(event) => event,
                    Err(TryRecvError::Empty) => return Ok(()),
                    Err(TryRecvError::Disconnected) => {
                        return self.handle_search_disconnect(active);
                    }
                },
                Some(ActiveSearch::AwaitingStop { .. }) | None => return Ok(()),
            };
            self.handle_search_event(engine, active, event, output)?;
        }
    }

    /// 探索イベントを短時間だけ待って処理する。入力が閉じた後の待機に使う。
    fn wait_search_event(
        &mut self,
        engine: &Engine,
        active: &mut Option<ActiveSearch>,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        match active.as_ref() {
            Some(ActiveSearch::Running { handle, .. }) => {
                match handle.events().recv_timeout(Duration::from_millis(50)) {
                    Ok(event) => self.handle_search_event(engine, active, event, output)?,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        self.handle_search_disconnect(active)?;
                    }
                }
            }
            Some(ActiveSearch::AwaitingStop { .. }) | None => {}
        }
        Ok(())
    }

    /// 完了イベントなしで探索チャネルが切断された異常を処理する。
    fn handle_search_disconnect(&mut self, active: &mut Option<ActiveSearch>) -> io::Result<()> {
        let Some(ActiveSearch::Running { handle, .. }) = active.take() else {
            return Ok(());
        };
        self.transposition_table = Some(join_search(handle)?);
        Err(io::Error::other("search ended without a finished event"))
    }

    /// 探索イベントを`info`行または`bestmove`行へ変換する。
    ///
    /// `go infinite`の完了は、USIの規定どおり`stop`を受けるまで
    /// `bestmove`を保留する。
    fn handle_search_event(
        &mut self,
        engine: &Engine,
        active: &mut Option<ActiveSearch>,
        event: SearchEvent,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let event_search_id = event.search_id();
        let Some(ActiveSearch::Running { context, .. }) = active.as_mut() else {
            return Ok(());
        };
        if event_search_id != context.id {
            return Ok(());
        }

        match event {
            SearchEvent::Progress {
                depth,
                score,
                nodes,
                elapsed,
                pv,
                ..
            } => {
                write_info(output, context, depth, score, nodes, elapsed, &pv)?;
                context.last_info_depth = Some(depth);
                Ok(())
            }
            SearchEvent::Finished {
                best_move,
                score,
                depth,
                nodes,
                elapsed,
                pv,
                stop_reason,
                ..
            } => {
                let Some(ActiveSearch::Running {
                    mut context,
                    handle,
                }) = active.take()
                else {
                    unreachable!();
                };
                self.transposition_table = Some(join_search(handle)?);
                write_final_info_if_deeper(
                    output,
                    &mut context,
                    depth,
                    score,
                    nodes,
                    elapsed,
                    &pv,
                )?;
                let ponder_move = validated_ponder_move(engine.game(), best_move, &pv);
                if context.infinite || context.ponder {
                    *active = Some(ActiveSearch::AwaitingStop {
                        context,
                        best_move,
                        ponder_move,
                        score,
                        depth,
                        stop_reason,
                    });
                    Ok(())
                } else {
                    write_bestmove(
                        output,
                        &context.position,
                        best_move,
                        ponder_move.as_deref(),
                        stop_reason,
                        self.should_resign(score, depth),
                    )
                }
            }
        }
    }

    /// 先読みの結果保留を解除する。探索は作り直さない。
    fn ponderhit(
        &mut self,
        engine: &Engine,
        active: &mut Option<ActiveSearch>,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        match active.as_mut() {
            Some(ActiveSearch::Running { context, handle }) if context.ponder => {
                handle.ponderhit();
                context.ponder = false;
                Ok(())
            }
            Some(ActiveSearch::AwaitingStop { context, .. }) if context.ponder => {
                self.finish_search(engine, active, output, false)
            }
            _ => write_error(output, "ponderhit requires an active ponder search"),
        }
    }

    /// `stop`に応じて探索を打ち切り、`bestmove`を返す。
    fn stop_search(
        &mut self,
        engine: &Engine,
        active: &mut Option<ActiveSearch>,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        self.finish_search(engine, active, output, true)
    }

    /// 探索の完了を待ち切って`bestmove`を出力する。
    ///
    /// 完了までの進捗イベントも順に`info`行として出力する。
    fn finish_search(
        &mut self,
        engine: &Engine,
        active: &mut Option<ActiveSearch>,
        output: &mut dyn Write,
        request_stop: bool,
    ) -> io::Result<()> {
        let Some(search) = active.take() else {
            return Ok(());
        };
        match search {
            ActiveSearch::AwaitingStop {
                context,
                best_move,
                ponder_move,
                score,
                depth,
                stop_reason,
            } => write_bestmove(
                output,
                &context.position,
                best_move,
                ponder_move.as_deref(),
                stop_reason,
                self.should_resign(score, depth),
            ),
            ActiveSearch::Running {
                mut context,
                handle,
            } => {
                if request_stop {
                    handle.request_stop();
                }
                let (best_move, ponder_move, score, depth, stop_reason) = loop {
                    let event = handle
                        .events()
                        .recv()
                        .map_err(|_| io::Error::other("search ended without a finished event"))?;
                    if event.search_id() != context.id {
                        continue;
                    }
                    match event {
                        SearchEvent::Progress {
                            depth,
                            score,
                            nodes,
                            elapsed,
                            pv,
                            ..
                        } => {
                            write_info(output, &context, depth, score, nodes, elapsed, &pv)?;
                            context.last_info_depth = Some(depth);
                        }
                        SearchEvent::Finished {
                            best_move,
                            score,
                            depth,
                            nodes,
                            elapsed,
                            pv,
                            stop_reason,
                            ..
                        } => {
                            write_final_info_if_deeper(
                                output,
                                &mut context,
                                depth,
                                score,
                                nodes,
                                elapsed,
                                &pv,
                            )?;
                            break (
                                best_move,
                                validated_ponder_move(engine.game(), best_move, &pv),
                                score,
                                depth,
                                stop_reason,
                            );
                        }
                    }
                };
                self.transposition_table = Some(join_search(handle)?);
                write_bestmove(
                    output,
                    &context.position,
                    best_move,
                    ponder_move.as_deref(),
                    stop_reason,
                    self.should_resign(score, depth),
                )
            }
        }
    }

    /// 完了した探索の採用結果が投了閾値以下かを返す。
    fn should_resign(&self, score: i32, depth: u32) -> bool {
        depth >= 1 && score <= -self.resign_value
    }

    /// `bestmove`を出力せずに探索を破棄する。`gameover`・`quit`と入力断で使う。
    fn discard_search(&mut self, active: &mut Option<ActiveSearch>) -> io::Result<()> {
        let Some(search) = active.take() else {
            return Ok(());
        };
        if let ActiveSearch::Running { handle, .. } = search {
            handle.request_stop();
            self.transposition_table = Some(join_search(handle)?);
        }
        Ok(())
    }

    /// `usi`への応答としてエンジン名とoption宣言を出力する。
    fn write_handshake(&self, output: &mut dyn Write) -> io::Result<()> {
        writeln!(output, "id name minase {}", env!("CARGO_PKG_VERSION"))?;
        writeln!(output, "id author stepney141")?;
        writeln!(
            output,
            "option name RuleSet type string default {}",
            self.startup_rules_text
        )?;
        writeln!(
            output,
            "option name USI_Variant type string default chushogi"
        )?;
        writeln!(
            output,
            "option name USI_Hash type spin default {} min 1 max {}",
            search::DEFAULT_TT_SIZE_MB,
            MAX_HASH_SIZE_MB
        )?;
        writeln!(
            output,
            "option name Threads type spin default {} min 1 max 256",
            search::DEFAULT_THREADS
        )?;
        writeln!(
            output,
            "option name ResignValue type spin default {DEFAULT_RESIGN_VALUE} min 1 max 99999"
        )?;
        #[cfg(feature = "tuning")]
        for &(name, default, min, max) in search::alphabeta::params::PARAMETERS {
            writeln!(
                output,
                "option name Tune_{name} type spin default {default} min {min} max {max}"
            )?;
        }
        writeln!(output, "usiok")
    }

    /// `setoption`を処理する。RuleSet・USI_Variant・USI_Hash・Threads・ResignValueを受理し、
    /// 未知のoption名はUSIの慣例に従って黙って無視する。
    fn handle_setoption(
        &mut self,
        engine: &mut Engine,
        tokens: &[&str],
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let name = token_after(tokens, "name");
        let value = token_after(tokens, "value");
        let Some(name) = name else {
            return Ok(());
        };

        #[cfg(feature = "tuning")]
        if let Some(parameter) = name.strip_prefix("Tune_") {
            if TUNING_LOCKED.load(Ordering::Relaxed) {
                return write_error(output, "tuning parameters cannot change after go");
            }
            let Some(value) = value else {
                return write_error(output, &format!("{name} requires a value"));
            };
            let Ok(value) = value.parse::<i32>() else {
                return write_error(output, &format!("{name} must be an integer"));
            };
            return match search::alphabeta::params::set(parameter, value) {
                Ok(()) => Ok(()),
                Err(error) => write_error(output, &error.to_string()),
            };
        }

        if name.eq_ignore_ascii_case("RuleSet") {
            let Some(value) = value else {
                return write_error(output, "RuleSet requires a value");
            };
            let codes = match parse_rule_set(value) {
                Ok(codes) => codes,
                Err(error) => return write_error(output, &error.to_string()),
            };
            self.apply_silent(engine, EngineCommand::SetRules(codes), output)
        } else if name.eq_ignore_ascii_case("USI_Variant") {
            match value {
                Some(value) if value.eq_ignore_ascii_case("chushogi") => Ok(()),
                Some(value) => {
                    write_error(output, &format!("unsupported USI_Variant value '{value}'"))
                }
                None => write_error(output, "USI_Variant requires a value"),
            }
        } else if name.eq_ignore_ascii_case("USI_Hash") {
            let Some(value) = value else {
                return write_error(output, "USI_Hash requires a value");
            };
            let Some(size_mb) = value
                .parse::<usize>()
                .ok()
                .filter(|&size| (1..=MAX_HASH_SIZE_MB).contains(&size))
            else {
                return write_error(
                    output,
                    &format!("USI_Hash must be an integer from 1 to {MAX_HASH_SIZE_MB}"),
                );
            };
            let result = match &mut self.transposition_table {
                Some(transposition_table) => transposition_table.resize(size_mb),
                None => TranspositionTable::new(size_mb).map(|table| {
                    self.transposition_table = Some(table);
                }),
            };
            if let Err(error) = result {
                return write_error(output, &error.to_string());
            }
            Ok(())
        } else if name.eq_ignore_ascii_case("Threads") {
            let Some(value) = value else {
                return write_error(output, "Threads requires a value");
            };
            let Some(threads) = parse_threads(value) else {
                return write_error(output, "Threads must be an integer from 1 to 256");
            };
            self.threads = threads;
            Ok(())
        } else if name.eq_ignore_ascii_case("ResignValue") {
            let Some(value) = value else {
                return write_error(output, "missing ResignValue value");
            };
            let Some(value) = value
                .bytes()
                .all(|byte| byte.is_ascii_digit())
                .then(|| value.parse::<i32>().ok())
                .flatten()
                .filter(|value| (1..=99_999).contains(value))
            else {
                return write_error(output, "invalid ResignValue value");
            };
            self.resign_value = value;
            Ok(())
        } else {
            Ok(())
        }
    }

    /// `position`を処理し、受理できた場合だけ局面を同期済みにする。
    fn handle_position(
        &mut self,
        engine: &mut Engine,
        tokens: &[&str],
        output: &mut dyn Write,
    ) -> io::Result<()> {
        self.position_synchronized = false;
        let Some(kind) = tokens.first().copied() else {
            self.accepted_position = None;
            return write_error(output, "position requires startpos or sfen");
        };
        let moves_index = tokens.iter().position(|token| *token == "moves");
        let move_tokens = moves_index.map(|index| &tokens[index + 1..]).unwrap_or(&[]);
        let position_tokens = &tokens[..moves_index.unwrap_or(tokens.len())];

        if engine.lifecycle() == EngineLifecycle::InGame
            && let Some(accepted) = &self.accepted_position
            && !move_tokens.is_empty()
            && move_tokens.len() == accepted.move_tokens.len()
            && accepted
                .setup_tokens
                .iter()
                .map(String::as_str)
                .eq(position_tokens.iter().copied())
            && accepted.move_tokens[..move_tokens.len() - 1]
                .iter()
                .map(String::as_str)
                .eq(move_tokens[..move_tokens.len() - 1].iter().copied())
            && accepted.move_tokens.last().map(String::as_str) != move_tokens.last().copied()
            && let Some(previous) = engine.before_last_move()
        {
            let text = move_tokens[move_tokens.len() - 1];
            let mv = match usi::parse(previous.position(), text) {
                Ok(mv) => mv,
                Err(error) => return write_error(output, &error.to_string()),
            };
            return self.apply_position(
                engine,
                EngineCommand::ReplaceLastMove(mv),
                Some(text),
                PositionHistoryUpdate::Last(text),
                output,
            );
        }

        let extension_start = position_extension_start(
            self.accepted_position.as_ref(),
            engine.lifecycle(),
            position_tokens,
            move_tokens,
        );
        if let Some(extension_start) = extension_start {
            let parsed = match parse_moves_from_game(engine.game(), &move_tokens[extension_start..])
            {
                Ok(parsed) => parsed,
                Err(error) => {
                    self.accepted_position = None;
                    return write_error(output, &error);
                }
            };
            return self.apply_position(
                engine,
                EngineCommand::ExtendPosition {
                    moves: parsed.moves,
                },
                parsed.first_rejected_text.as_deref(),
                PositionHistoryUpdate::Extend(&move_tokens[extension_start..]),
                output,
            );
        }

        let rules = engine.position_rules();
        let setup = match kind {
            "startpos" => {
                if position_tokens.len() != 1 {
                    self.accepted_position = None;
                    return write_error(output, "position startpos has unexpected fields");
                }
                SetupPosition::new(Position::initial(), None, 1)
                    .expect("the standard initial setup is valid")
            }
            "sfen" => match parse_sfen_fields(&position_tokens[1..], rules.moves) {
                Ok(setup) => setup,
                Err(error) => {
                    self.accepted_position = None;
                    return write_error(output, &error.to_string());
                }
            },
            _ => {
                self.accepted_position = None;
                return write_error(output, "position requires startpos or sfen");
            }
        };

        let parsed = match parse_moves(&setup, rules, move_tokens) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.accepted_position = None;
                return write_error(output, &error);
            }
        };
        self.apply_position(
            engine,
            EngineCommand::SetPosition {
                setup,
                moves: parsed.moves,
            },
            parsed.first_rejected_text.as_deref(),
            PositionHistoryUpdate::Replace {
                setup_tokens: position_tokens,
                move_tokens,
            },
            output,
        )
    }

    /// `position`の状態機械コマンドを適用し、同期状態と履歴を更新する。
    fn apply_position(
        &mut self,
        engine: &mut Engine,
        command: EngineCommand,
        first_rejected_text: Option<&str>,
        history_update: PositionHistoryUpdate<'_>,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        match engine.handle(command) {
            EngineReply::Accepted { .. } => {
                self.position_synchronized = true;
                match history_update {
                    PositionHistoryUpdate::Replace {
                        setup_tokens,
                        move_tokens,
                    } => {
                        self.accepted_position = Some(AcceptedPosition {
                            setup_tokens: setup_tokens
                                .iter()
                                .map(|token| (*token).to_owned())
                                .collect(),
                            move_tokens: move_tokens
                                .iter()
                                .map(|token| (*token).to_owned())
                                .collect(),
                        });
                    }
                    PositionHistoryUpdate::Last(text) => {
                        *self
                            .accepted_position
                            .as_mut()
                            .expect("replacement requires a position")
                            .move_tokens
                            .last_mut()
                            .expect("replacement requires a move") = text.to_owned();
                    }
                    PositionHistoryUpdate::Extend(move_tokens) => self
                        .accepted_position
                        .as_mut()
                        .expect("the extension path requires an accepted position")
                        .move_tokens
                        .extend(move_tokens.iter().map(|token| (*token).to_owned())),
                }
                Ok(())
            }
            EngineReply::Rejected(reason) => {
                self.accepted_position = None;
                write_error(
                    output,
                    &position_reject_reason_text(&reason, first_rejected_text),
                )
            }
        }
    }

    /// 独自拡張`moves`への応答として現局面の全合法手を出力する。
    fn handle_moves(&self, engine: &Engine, output: &mut dyn Write) -> io::Result<()> {
        if engine.lifecycle() != EngineLifecycle::InGame {
            return write_error(output, "moves requires an active game");
        }

        let game = engine.game();
        write!(output, "moves")?;
        for mv in game.legal_moves() {
            write!(output, " {}", usi::text_generated(game.position(), mv))?;
        }
        writeln!(output)
    }

    /// 独自拡張`state`への応答として規則・盤面・対局状態を出力する。
    fn handle_state(&self, engine: &Engine, output: &mut dyn Write) -> io::Result<()> {
        if engine.lifecycle() == EngineLifecycle::AwaitingStart {
            return write_error(output, "state requires an active or finished game");
        }

        let status = match state_status_text(engine.status()) {
            Ok(status) => status,
            Err(error) => return write_error(output, error),
        };
        writeln!(
            output,
            "state rules {} board {} status {status}",
            canonical_rules_text(engine.active_rule_codes()),
            to_sfen(engine.game().position()),
        )
    }

    /// 受理時に何も出力しないコマンドを状態機械へ渡す。
    fn apply_silent(
        &mut self,
        engine: &mut Engine,
        command: EngineCommand,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        // 新規対局と規則変更で置換表を空にし、前対局・別規則の評価を持ち越さない。
        let clears_transposition_table = matches!(
            &command,
            EngineCommand::NewGame | EngineCommand::SetRules(_)
        );
        let clears_position_history = matches!(
            &command,
            EngineCommand::NewGame | EngineCommand::SetRules(_) | EngineCommand::EndGame
        );
        let invalidates_position =
            matches!(&command, EngineCommand::NewGame | EngineCommand::EndGame);
        match engine.handle(command) {
            EngineReply::Accepted { .. } => {
                if invalidates_position {
                    self.position_synchronized = false;
                }
                if clears_position_history {
                    self.accepted_position = None;
                }
                if clears_transposition_table
                    && let Some(transposition_table) = &mut self.transposition_table
                {
                    transposition_table.clear();
                }
                Ok(())
            }
            EngineReply::Rejected(reason) => write_error(output, &reason.to_string()),
        }
    }
}

/// 前回受理した`position`との差分を適用できる場合に追加開始位置を返す。
fn position_extension_start(
    accepted: Option<&AcceptedPosition>,
    lifecycle: EngineLifecycle,
    setup_tokens: &[&str],
    move_tokens: &[&str],
) -> Option<usize> {
    let accepted = accepted?;
    let same_setup = accepted
        .setup_tokens
        .iter()
        .map(String::as_str)
        .eq(setup_tokens.iter().copied());
    let extends_moves = move_tokens.len() >= accepted.move_tokens.len()
        && accepted
            .move_tokens
            .iter()
            .map(String::as_str)
            .eq(move_tokens[..accepted.move_tokens.len()].iter().copied());
    (lifecycle == EngineLifecycle::InGame && same_setup && extends_moves)
        .then_some(accepted.move_tokens.len())
}

/// `go`の引数列を探索制限へ変換する。
///
/// `depth`・`nodes`・`movetime`・時計引数(`btime`等)・`infinite`を受理し、
/// 時計引数からは手番側の残り時間だけを取り出す。
fn parse_go_config(tokens: &[&str], side_to_move: Color, ply: u32) -> Result<SearchLimits, String> {
    if tokens.is_empty() {
        return Err("go requires depth or nodes".to_owned());
    }

    let mut depth = None;
    let mut nodes = None;
    let mut movetime_ms = None;
    let mut btime = None;
    let mut wtime = None;
    let mut binc = None;
    let mut winc = None;
    let mut byoyomi = None;
    let mut infinite = false;
    let mut ponder = false;
    let mut index = 0;
    while index < tokens.len() {
        let name = tokens[index];
        let value = tokens.get(index + 1).copied();
        match name {
            "depth" => {
                if depth.is_some() {
                    return Err("go depth must be specified once".to_owned());
                }
                let parsed = value
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|&value| value > 0)
                    .ok_or_else(|| "go depth must be a positive integer".to_owned())?;
                if parsed > search::MAX_PLY {
                    return Err(format!("go depth must not exceed {}", search::MAX_PLY));
                }
                depth = Some(parsed);
            }
            "nodes" => {
                if nodes.is_some() {
                    return Err("go nodes must be specified once".to_owned());
                }
                nodes = Some(
                    value
                        .and_then(|value| value.parse::<u64>().ok())
                        .filter(|&value| value > 0)
                        .ok_or_else(|| "go nodes must be a positive integer".to_owned())?,
                );
            }
            "movetime" => {
                movetime_ms = Some(parse_go_milliseconds(name, value, movetime_ms)?);
            }
            "btime" => btime = Some(parse_go_milliseconds(name, value, btime)?),
            "wtime" => wtime = Some(parse_go_milliseconds(name, value, wtime)?),
            "binc" => binc = Some(parse_go_milliseconds(name, value, binc)?),
            "winc" => winc = Some(parse_go_milliseconds(name, value, winc)?),
            "byoyomi" => {
                byoyomi = Some(parse_go_milliseconds(name, value, byoyomi)?);
            }
            "ponder" => {
                if ponder {
                    return Err("go ponder must be specified once".to_owned());
                }
                ponder = true;
                index += 1;
                continue;
            }
            "infinite" => {
                if infinite {
                    return Err("go infinite must be specified once".to_owned());
                }
                infinite = true;
                index += 1;
                continue;
            }
            unsupported => return Err(format!("unsupported go argument '{unsupported}'")),
        }
        index += 2;
    }

    let clock_specified =
        btime.is_some() || wtime.is_some() || binc.is_some() || winc.is_some() || byoyomi.is_some();
    let clock = clock_specified
        .then(|| {
            let (remaining_ms, increment_ms) = match side_to_move {
                Color::Black => (btime.unwrap_or(0), binc.unwrap_or(0)),
                Color::White => (wtime.unwrap_or(0), winc.unwrap_or(0)),
            };
            ClockLimits::new(remaining_ms, increment_ms, byoyomi.unwrap_or(0), ply)
        })
        .transpose()
        .map_err(|error| error.to_string())?;

    if infinite {
        if ponder || depth.is_some() || nodes.is_some() || movetime_ms.is_some() || clock.is_some()
        {
            return Err("go infinite cannot be combined with finite limits".to_owned());
        }
        Ok(SearchLimits::infinite())
    } else {
        SearchLimits::new(depth, nodes, movetime_ms, clock).map_err(|error| error.to_string())
    }
}

/// `go`の時間引数1個をミリ秒として解析する。重複指定は拒否する。
fn parse_go_milliseconds(
    name: &str,
    value: Option<&str>,
    previous: Option<u64>,
) -> Result<u64, String> {
    if previous.is_some() {
        return Err(format!("go {name} must be specified once"));
    }
    value
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| format!("go {name} must be a non-negative integer"))
}

/// `position sfen`の欄列を4欄または5欄の拡張SFENとして解析する。
fn parse_sfen_fields(
    fields: &[&str],
    rules: crate::MoveRules,
) -> Result<SetupPosition, crate::notation::sfen::SfenError> {
    parse_extended_sfen(&fields.join(" "), rules)
}

impl Protocol for UsiProtocol {
    fn run(
        &mut self,
        engine: &mut Engine,
        input: &mut dyn BufRead,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let mut active = None;
        let mut pending = VecDeque::new();
        let mut line = String::new();
        loop {
            let command = if active.is_none()
                && let Some(command) = pending.pop_front()
            {
                command
            } else {
                line.clear();
                if input.read_line(&mut line)? == 0 {
                    break;
                }
                line.trim_end().to_owned()
            };

            if active.is_some() {
                self.handle_searching_line(engine, &mut active, &mut pending, &command, output)?;
                self.poll_search(engine, &mut active, output)?;
                if active
                    .as_ref()
                    .is_some_and(|search| !search.withholds_bestmove())
                {
                    self.finish_search(engine, &mut active, output, false)?;
                }
                continue;
            }

            match self.handle_idle_line(engine, &command, output)? {
                LineAction::Continue => {}
                LineAction::Start(search) => {
                    active = Some(*search);
                    if !active
                        .as_ref()
                        .is_some_and(ActiveSearch::withholds_bestmove)
                    {
                        self.finish_search(engine, &mut active, output, false)?;
                    }
                }
                LineAction::Quit => return Ok(()),
            }
        }
        self.discard_search(&mut active)
    }
}

impl ActiveSearch {
    /// 停止または的中まで結果を保留するかを返す。
    fn withholds_bestmove(&self) -> bool {
        match self {
            Self::Running { context, .. } | Self::AwaitingStop { context, .. } => {
                context.infinite || context.ponder
            }
        }
    }
}

/// 探索スレッドの終了を待ち、貸し出していた置換表を回収する。
fn join_search(handle: SearchHandle) -> io::Result<TranspositionTable> {
    handle
        .join()
        .map_err(|_| io::Error::other("search thread panicked"))
}

/// 最終結果の2手目を対局履歴と終局規則で検査する。
fn validated_ponder_move(game: &Game, best_move: Move, pv: &[Move]) -> Option<String> {
    let &prediction = pv.get(1)?;
    let mut game = game.clone();
    game.play(best_move).ok()?;
    let position = game.position().clone();
    (game.play(prediction).ok()? == GameStatus::Ongoing)
        .then(|| usi::text_generated(&position, prediction))
}

/// 停止理由と`bestmove`行を出力する。
fn write_bestmove(
    output: &mut dyn Write,
    position: &Position,
    mv: Move,
    ponder_move: Option<&str>,
    stop_reason: StopReason,
    resign: bool,
) -> io::Result<()> {
    writeln!(output, "info string stop {}", stop_reason_text(stop_reason))?;
    if resign {
        writeln!(output, "bestmove resign")?;
    } else {
        write!(output, "bestmove {}", usi::text_generated(position, mv))?;
        if let Some(prediction) = ponder_move {
            write!(output, " ponder {prediction}")?;
        }
        writeln!(output)?;
    }
    output.flush()
}

/// 探索停止理由をUSI出力用の固定語へ変換する。
const fn stop_reason_text(reason: StopReason) -> &'static str {
    match reason {
        StopReason::DepthCompleted => "depth",
        StopReason::NodeLimit => "nodes",
        StopReason::SoftLimit => "soft",
        StopReason::HardLimit => "hard",
        StopReason::ExternalStop => "external",
    }
}

/// 探索進捗の`info`行を出力する。読み筋は局面を進めながら表記する。
fn write_info(
    output: &mut dyn Write,
    context: &SearchContext,
    depth: u32,
    score: i32,
    nodes: u64,
    elapsed: Duration,
    pv: &[Move],
) -> io::Result<()> {
    let score = score_text(score);
    write!(
        output,
        "info depth {depth} score {score} nodes {nodes} nps {} time {} pv",
        nodes_per_second(nodes, elapsed),
        elapsed.as_millis()
    )?;
    let mut position = context.position.clone();
    for &mv in pv {
        write!(output, " {}", usi::text_generated(&position, mv))?;
        let _ = position.make_move_unchecked(mv, context.rules.moves);
    }
    writeln!(output)?;
    output.flush()
}

/// 採用深さが最後の進捗出力を超える場合だけ、採用結果を`info`として出す。
#[allow(clippy::too_many_arguments)]
fn write_final_info_if_deeper(
    output: &mut dyn Write,
    context: &mut SearchContext,
    depth: u32,
    score: i32,
    nodes: u64,
    elapsed: Duration,
    pv: &[Move],
) -> io::Result<()> {
    if depth <= context.last_info_depth.unwrap_or(0) {
        return Ok(());
    }
    write_info(output, context, depth, score, nodes, elapsed, pv)?;
    context.last_info_depth = Some(depth);
    Ok(())
}

/// 評価値を`info score`用の文字列へ変換する。
/// 詰み帯は終局までの手数から2を引いて0以上にし、0にも勝敗の符号を付ける。
fn score_text(score: i32) -> String {
    if score >= search::MATE_THRESHOLD {
        let moves = (search::MATE - score - 2).max(0);
        if moves == 0 {
            "mate +0".to_owned()
        } else {
            format!("mate {moves}")
        }
    } else if score <= -search::MATE_THRESHOLD {
        let moves = (search::MATE + score - 2).max(0);
        format!("mate -{moves}")
    } else {
        format!("cp {score}")
    }
}

/// 1秒あたりの探索ノード数を計算する。経過0では0を返す。
fn nodes_per_second(nodes: u64, elapsed: Duration) -> u64 {
    let nanoseconds = elapsed.as_nanos();
    if nanoseconds == 0 {
        return 0;
    }
    (u128::from(nodes) * 1_000_000_000 / nanoseconds).min(u128::from(u64::MAX)) as u64
}

/// 指定キーの直後のトークンを返す。
fn token_after<'a>(tokens: &'a [&str], key: &str) -> Option<&'a str> {
    tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case(key))
        .and_then(|index| tokens.get(index + 1).copied())
}

/// 1以上256以下のワーカー数を解析する。
fn parse_threads(value: &str) -> Option<NonZeroUsize> {
    value
        .parse::<usize>()
        .ok()
        .and_then(NonZeroUsize::new)
        .filter(|threads| threads.get() <= 256)
}

/// `moves`以降の指し手列を解析し、局面を進めながら検証する。
///
/// 不合法な指し手が現れても解析済みの列は返し、拒否理由の報告用に
/// その表記を保持する。合否の最終判定は状態機械側の再適用が行う。
fn parse_moves(
    setup: &SetupPosition,
    rules: crate::Rules,
    move_tokens: &[&str],
) -> Result<ParsedMoves, String> {
    let mut position = setup.position().clone();
    position
        .set_lion_capture(setup.lion_capture())
        .map_err(|error| error.to_string())?;
    let game = Game::from_position(rules, position);
    parse_moves_from_game(&game, move_tokens)
}

/// 現在の対局を起点に着手列を解析し、局面を進めながら検証する。
fn parse_moves_from_game(game: &Game, move_tokens: &[&str]) -> Result<ParsedMoves, String> {
    let mut game = game.clone();
    let mut moves = Vec::with_capacity(move_tokens.len());

    for &text in move_tokens {
        let mv = usi::parse(game.position(), text).map_err(|error| error.to_string())?;
        moves.push(mv);
        if game.play(mv).is_err() {
            return Ok(ParsedMoves {
                moves,
                first_rejected_text: Some(text.to_owned()),
            });
        }
    }
    Ok(ParsedMoves {
        moves,
        first_rejected_text: None,
    })
}

/// `position`の指し手列の解析結果。
struct ParsedMoves {
    /// 解析できた指し手列。
    moves: Vec<Move>,
    /// 最初に拒否された指し手の入力表記。
    first_rejected_text: Option<String>,
}

/// `position`拒否の理由文を、拒否された指し手の表記を添えて作る。
fn position_reject_reason_text(reason: &RejectReason, rejected_text: Option<&str>) -> String {
    if let (RejectReason::IllegalMove { cause, .. }, Some(text)) = (reason, rejected_text) {
        format!("illegal move '{text}': {cause}")
    } else {
        reason.to_string()
    }
}

/// 対局状態を`state`行のstatus欄表記へ変換する。
///
/// 投了と合意引き分けはプロトコル外の操作で成立するため表現しない。
fn state_status_text(status: GameStatus) -> Result<String, &'static str> {
    match status {
        GameStatus::Ongoing => Ok("ongoing".to_owned()),
        GameStatus::Finished(GameResult::Win { winner, reason }) => {
            let winner = match winner {
                Color::Black => "black",
                Color::White => "white",
            };
            let reason = match reason {
                WinReason::RoyalCapture => "royal-capture",
                WinReason::Repetition => "repetition",
                WinReason::PieceExhaustion => "piece-exhaustion",
                WinReason::BareKing => "bare-king",
                WinReason::Stalemate => "stalemate",
                WinReason::Mate => "mate",
                WinReason::Resignation => return Err("state cannot represent resignation"),
            };
            Ok(format!("win {winner} {reason}"))
        }
        GameStatus::Finished(GameResult::Draw { reason }) => {
            let reason = match reason {
                DrawReason::Repetition => "repetition",
                DrawReason::PieceExhaustion => "piece-exhaustion",
                DrawReason::BareKing => "bare-king",
                DrawReason::Agreement => return Err("state cannot represent agreement"),
            };
            Ok(format!("draw {reason}"))
        }
    }
}

/// エラーを`info string`行として出力する。
fn write_error(output: &mut dyn Write, message: &str) -> io::Result<()> {
    writeln!(output, "info string error: {message}")
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::Rules;
    use crate::core::rules::RuleCode;

    /// RULES.md第5条の初期配置に対応する2欄SFEN（盤面部と手番部、D6-USI-10）。
    const INITIAL_BOARD: &str = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b";

    /// 王駒捕獲1手前の局面。7g7dで先手が後手の最後の王駒を取る（D6-USI-28の代表局面）。
    const ROYAL_SFEN: &str = "12/12/12/5k6/12/12/5R6/12/12/12/12/K11 b - 1";

    /// 7g7d適用後のFinished局面に対応するstate行（BG「stateコマンド」の単一行完全一致契約）。
    const ROYAL_FINISHED_STATE: &str = "state rules L0,P0,R1,E2 board 12/12/12/5R6/12/12/12/12/12/12/12/K11 w status win black royal-capture";

    /// movesの対局開始前・終局後エラー行（BG「movesコマンド」、台本完全一致）。
    const MOVES_ERROR: &str = "info string error: moves requires an active game";

    /// stateのAwaitingStartエラー行（BG「stateコマンド」、台本完全一致）。
    const STATE_ERROR: &str = "info string error: state requires an active or finished game";

    // search.md「探索内の終局と規則処理」の表示例と詰み帯の境界。
    #[test]
    fn score_text_displays_mate_distance_with_signed_zero() {
        for (score, expected) in [
            (search::MATE - 1, "mate +0"),
            (search::MATE - 3, "mate 1"),
            (search::MATE - 5, "mate 3"),
            (-search::MATE, "mate -0"),
            (-search::MATE + 2, "mate -0"),
            (-search::MATE + 4, "mate -2"),
            (search::MATE_THRESHOLD, "mate 254"),
            (-search::MATE_THRESHOLD, "mate -254"),
        ] {
            assert_eq!(score_text(score), expected, "score={score}");
        }
    }

    // 表示変換は詰み帯だけに適用し、通常評価値と詰み帯直前のcpはそのまま出す。
    #[test]
    fn score_text_preserves_cp_values_outside_mate_band() {
        for (score, expected) in [
            (0, "cp 0"),
            (100, "cp 100"),
            (-100, "cp -100"),
            (search::MATE_THRESHOLD - 1, "cp 29743"),
            (-search::MATE_THRESHOLD + 1, "cp -29743"),
        ] {
            assert_eq!(score_text(score), expected, "score={score}");
        }
    }

    /// 通常版では調整用オプションを宣言せず、未知オプションとして無視する。
    #[cfg(not(feature = "tuning"))]
    #[test]
    fn normal_build_does_not_expose_tuning_options() {
        let output = lishogi_session("usi\nsetoption name Tune_DeltaMargin value nope\n");
        assert!(
            !output
                .lines()
                .any(|line| line.starts_with("option name Tune_"))
        );
        assert!(error_lines(&output).is_empty());
    }

    fn make_engine(codes: &[RuleCode]) -> Engine {
        let mut complete = codes.to_vec();
        if !complete
            .iter()
            .any(|code| matches!(code, RuleCode::L0 | RuleCode::L1))
        {
            complete.push(RuleCode::L0);
        }
        if !complete
            .iter()
            .any(|code| matches!(code, RuleCode::P0 | RuleCode::P1 | RuleCode::P2))
        {
            complete.push(RuleCode::P0);
        }
        if !complete
            .iter()
            .any(|code| matches!(code, RuleCode::E0 | RuleCode::E2 | RuleCode::E3))
        {
            complete.push(RuleCode::E0);
        }
        Engine::new(complete).unwrap()
    }

    fn run(protocol: &mut UsiProtocol, engine: &mut Engine, input: &str) -> String {
        let (sender, receiver) = std::sync::mpsc::channel();
        for line in input.lines() {
            sender.send(Ok(line.to_owned())).unwrap();
        }
        drop(sender);
        let mut output = Vec::new();
        protocol
            .run_channel(engine, &receiver, &mut output)
            .unwrap();
        String::from_utf8(output).unwrap()
    }

    fn session(codes: &[RuleCode], input: &str) -> String {
        let mut engine = make_engine(codes);
        let mut protocol = UsiProtocol::new(&engine);
        run(&mut protocol, &mut engine, input)
    }

    fn lishogi_session(input: &str) -> String {
        let mut engine = Engine::new(parse_rule_set("lishogi").unwrap()).unwrap();
        let mut protocol = UsiProtocol::new(&engine);
        run(&mut protocol, &mut engine, input)
    }

    fn bestmoves(output: &str) -> Vec<&str> {
        output
            .lines()
            .filter(|line| line.starts_with("bestmove "))
            .collect()
    }

    fn error_lines(output: &str) -> Vec<&str> {
        output
            .lines()
            .filter(|line| line.starts_with("info string error: "))
            .collect()
    }

    fn state_lines(output: &str) -> Vec<&str> {
        output
            .lines()
            .filter(|line| line.starts_with("state "))
            .collect()
    }

    fn state_rules(line: &str) -> &str {
        line.split_whitespace().nth(2).unwrap()
    }

    fn moves_sets(output: &str) -> Vec<HashSet<String>> {
        output
            .lines()
            .filter(|line| line.starts_with("moves"))
            .map(|line| line.split_whitespace().skip(1).map(str::to_owned).collect())
            .collect()
    }

    #[test]
    fn handshake_declares_ruleset_before_variant_and_ends_with_usiok() {
        // PL「プロトコル固有の制御コマンド」・「規則オプション」（D6-USI-01）。
        // USI_Hashの宣言位置はSU-05により契約にせず、存在だけを確認する。
        let output = session(&[RuleCode::E2, RuleCode::R1, RuleCode::L1], "usi\nquit\n");
        let lines: Vec<_> = output.lines().collect();

        assert!(lines[0].starts_with("id name minase "));
        assert_eq!(lines[1], "id author stepney141");
        assert_eq!(*lines.last().unwrap(), "usiok");
        let ruleset = lines
            .iter()
            .position(|line| *line == "option name RuleSet type string default L1,P0,R1,E2")
            .expect("RuleSet declaration must exist with the canonical default");
        let variant = lines
            .iter()
            .position(|line| line.starts_with("option name USI_Variant type string"))
            .expect("USI_Variant must be declared as a string option");
        assert!(ruleset < variant);
        // EC実施状況フェーズ1: USI_Hashを宣言する（細目はSU-04で契約外）。
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("option name USI_Hash"))
        );
        // LS「プロトコル設定」: Threadsの宣言はdefaultを探索層の既定値から表示する（D6-USI-35）。
        assert!(lines.contains(&"option name Threads type spin default 1 min 1 max 256"));
        // ponder.md設計判断「USI_Ponder」: 時間管理が値に依存しないため宣言しない（D6-USI-18）。
        assert!(!output.contains("USI_Ponder"));
    }

    #[test]
    fn threads_accepts_boundaries_and_rejects_invalid_values() {
        // LS「プロトコル設定」: Threadsは1..=256だけを受理し、不正値には固定エラーを返す
        // （D6-USI-36）。正当値は無応答である。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);
        // 探索開始へ渡す値を受信後に観測し、設定の無視や丸めを検出する。
        for threads in [256, 1] {
            assert_eq!(
                run(
                    &mut protocol,
                    &mut engine,
                    &format!("setoption name Threads value {threads}\n")
                ),
                ""
            );
            assert_eq!(protocol.threads.get(), threads);
        }
        let output = run(
            &mut protocol,
            &mut engine,
            concat!(
                "setoption name Threads\n",
                "setoption name Threads value nope\n",
                "setoption name Threads value 0\n",
                "setoption name Threads value 257\n",
            ),
        );

        assert_eq!(
            output,
            concat!(
                "info string error: Threads requires a value\n",
                "info string error: Threads must be an integer from 1 to 256\n",
                "info string error: Threads must be an integer from 1 to 256\n",
                "info string error: Threads must be an integer from 1 to 256\n",
            )
        );
        assert_eq!(protocol.threads.get(), 1);
    }

    /// 王将の逃げ先が奔王、竪行、飛車に覆われ、反車でも捕獲を防げない局面。
    /// search/tests.rsのrepetition_fixtureをSFENへ写した（RS、D6-USI-51）。
    const RESIGN_MATE_SFEN: &str = "q10v/k11/12/12/12/6A5/12/12/12/12/12/r10K b - 1";

    /// 先手の王将と歩兵に対して後手が玉将と飛車を持つ通常の負評価局面。
    const RESIGN_MATERIAL_SFEN: &str = "k11/12/12/12/12/r11/12/12/12/12/11P/11K b - 1";

    /// SFENで局面を同期し、各テストの探索用に小さい置換表を確保する。
    fn resignation_engine(sfen: &str) -> (Engine, UsiProtocol) {
        let mut engine = Engine::new(parse_rule_set("lishogi").unwrap()).unwrap();
        let mut protocol = UsiProtocol::new(&engine);
        let output = run(
            &mut protocol,
            &mut engine,
            &format!("setoption name USI_Hash value 1\nposition sfen {sfen}\n"),
        );
        assert!(output.is_empty(), "{output}");
        (engine, protocol)
    }

    /// 非投了の応答が、同じ局面のmovesに含まれる着手を1つ返すことを確認する。
    fn assert_legal_bestmove(output: &str, legal: &HashSet<String>) {
        let best = bestmoves(output);
        assert_eq!(best.len(), 1, "{output}");
        assert!(
            legal.contains(best[0].split_whitespace().nth(1).unwrap()),
            "{output}"
        );
    }

    // RS「USI層の契約」（D6-USI-47）。宣言の文字列は設計書の固定値。
    #[test]
    fn resignation_option_declares_default_and_bounds_once() {
        let output = lishogi_session("usi\n");
        assert_eq!(
            output
                .lines()
                .filter(|line| line.starts_with("option name ResignValue "))
                .collect::<Vec<_>>(),
            ["option name ResignValue type spin default 20000 min 1 max 99999"]
        );
        assert_eq!(output.lines().last(), Some("usiok"));
    }

    // RS「USI層の契約」（D6-USI-48）。設定の保持は次の探索の応答で観測する。
    #[test]
    fn resignation_option_accepts_bounds_and_rejects_invalid_values_without_changing_it() {
        let (mut engine, mut protocol) = resignation_engine(RESIGN_MATERIAL_SFEN);
        let legal = moves_sets(&run(&mut protocol, &mut engine, "moves\n")).remove(0);
        for value in ["1", "99999", "00001"] {
            assert_eq!(
                run(
                    &mut protocol,
                    &mut engine,
                    &format!("setoption name ResignValue value {value}\n")
                ),
                ""
            );
            let output = run(&mut protocol, &mut engine, "go depth 1\n");
            if value == "99999" {
                assert_legal_bestmove(&output, &legal);
            } else {
                assert_eq!(bestmoves(&output), ["bestmove resign"]);
            }
        }
        for value in [
            "",
            "value",
            "value nope",
            "value 0",
            "value 100000",
            "value -1",
            "value +1",
            "value 1.5",
            "value 1e3",
            "value １２",
            "value 999999999999999999999999999",
        ] {
            let output = run(
                &mut protocol,
                &mut engine,
                &format!("setoption name ResignValue {value}\n"),
            );
            assert_eq!(
                output,
                if matches!(value, "" | "value") {
                    "info string error: missing ResignValue value\n"
                } else {
                    "info string error: invalid ResignValue value\n"
                }
            );
            let output = run(&mut protocol, &mut engine, "go depth 1\n");
            assert_eq!(bestmoves(&output), ["bestmove resign"], "{value}: {output}");
        }
    }

    // RS設計判断「判定の入力」（D6-USI-50）。深さ0の静的評価では投了しない。
    #[test]
    fn resignation_depth_zero_returns_a_legal_move_without_search_info() {
        let output = lishogi_session(&format!(
            "setoption name USI_Hash value 1\nsetoption name ResignValue value 1\nposition sfen {RESIGN_MATERIAL_SFEN}\nmoves\ngo nodes 1\n"
        ));
        assert!(error_lines(&output).is_empty(), "{output}");
        assert!(
            !output.lines().any(|line| line.starts_with("info depth ")),
            "{output}"
        );
        assert!(output.contains("info string stop nodes\n"), "{output}");
        assert_legal_bestmove(&output, &moves_sets(&output)[0]);
    }

    // RS設計判断「既定値」「出力の形式」（D6-USI-51、D6-USI-58）。
    #[test]
    fn resignation_default_emits_mate_info_then_stop_then_resign_once() {
        for threads in [1, 2] {
            let output = lishogi_session(&format!(
                "setoption name USI_Hash value 1\nsetoption name Threads value {threads}\nposition sfen {RESIGN_MATE_SFEN}\ngo depth 2\n"
            ));
            assert!(error_lines(&output).is_empty(), "{output}");
            let lines: Vec<_> = output.lines().collect();
            assert_eq!(
                &lines[lines.len() - 2..],
                ["info string stop depth", "bestmove resign"]
            );
            assert!(
                lines[..lines.len() - 2]
                    .iter()
                    .all(|line| line.starts_with("info depth ") && line.contains(" score mate -")),
                "{output}"
            );
            assert_eq!(bestmoves(&output), ["bestmove resign"]);
            // 同じ完了深さを投了のために再出力しない。
            let depths: Vec<_> = lines[..lines.len() - 2]
                .iter()
                .map(|line| {
                    line.split_whitespace()
                        .nth(2)
                        .unwrap()
                        .parse::<u32>()
                        .unwrap()
                })
                .collect();
            assert!(!depths.is_empty());
            assert!(depths.windows(2).all(|pair| pair[0] < pair[1]), "{output}");
        }
    }

    // RS設計判断「閾値の与え方」（D6-USI-52）。評価値の定数を写さず、
    // 無効化した探索の公開出力を基準に等号と隣接値の関係を検証する。
    #[test]
    fn resignation_material_threshold_includes_equality() {
        let search = |threshold| {
            lishogi_session(&format!(
                "setoption name USI_Hash value 1\nsetoption name ResignValue value {threshold}\nposition sfen {RESIGN_MATERIAL_SFEN}\nmoves\ngo depth 1\n"
            ))
        };
        let baseline = search(99999);
        assert!(error_lines(&baseline).is_empty(), "{baseline}");
        let score: i32 = baseline
            .lines()
            .find_map(|line| {
                line.split_once(" score cp ")
                    .map(|(_, rest)| rest.split_whitespace().next().unwrap().parse().unwrap())
            })
            .unwrap();
        assert!((-29744..0).contains(&score), "{baseline}");
        for threshold in [1, -score] {
            assert_eq!(bestmoves(&search(threshold)), ["bestmove resign"]);
        }
        let output = search(-score + 1);
        assert_legal_bestmove(&output, &moves_sets(&output)[0]);
    }

    // RS設計判断「閾値の与え方」（D6-USI-51、D6-USI-53）。
    #[test]
    fn resignation_mate_thresholds_and_winning_scores_use_signed_comparison() {
        for threshold in [29744, 29998, 29999, 30000, 99999] {
            let output = lishogi_session(&format!(
                "setoption name USI_Hash value 1\nsetoption name ResignValue value {threshold}\nposition sfen {RESIGN_MATE_SFEN}\nmoves\ngo depth 2\n"
            ));
            assert!(error_lines(&output).is_empty(), "{output}");
            assert!(output.contains(" score mate -0 "), "{output}");
            if threshold <= 29998 {
                assert_eq!(bestmoves(&output), ["bestmove resign"]);
            } else {
                assert_legal_bestmove(&output, &moves_sets(&output)[0]);
            }
        }
        let output = session(
            &[RuleCode::R1, RuleCode::E2],
            &format!(
                "setoption name USI_Hash value 1\nsetoption name ResignValue value 1\nposition sfen {ROYAL_SFEN}\nmoves\ngo depth 1\n"
            ),
        );
        assert!(output.contains(" score mate +0 "), "{output}");
        assert_legal_bestmove(&output, &moves_sets(&output)[0]);
    }

    // RS設計判断「判定の入力」「出力の形式」（D6-USI-52、D6-USI-53、D6-USI-58）。
    // 探索イベント境界へ規範値を入力し、進捗と採用値が異なる場合を決定的に検証する。
    #[test]
    fn resignation_uses_adopted_score_at_mate_boundary_without_repeating_info() {
        for (score, threshold, resign) in [
            (-29744, 29744, true),
            (-29744, 29745, false),
            (-100, 100, true),
            (0, 1, false),
            (100, 1, false),
        ] {
            for release in [None, Some("stop"), Some("ponderhit")] {
                let (mut engine, mut protocol) = resignation_engine(RESIGN_MATERIAL_SFEN);
                run(
                    &mut protocol,
                    &mut engine,
                    &format!("setoption name ResignValue value {threshold}\n"),
                );
                let legal = moves_sets(&run(&mut protocol, &mut engine, "moves\n")).remove(0);
                let mut output = Vec::new();
                let limits = if release.is_some() {
                    vec!["ponder", "depth", "2"]
                } else {
                    vec!["depth", "2"]
                };
                let mut active = protocol.start_go(&engine, &limits, &mut output).unwrap();
                let before_release = loop {
                    let Some(ActiveSearch::Running { handle, .. }) = active.as_ref() else {
                        panic!("search must be running")
                    };
                    let mut event = handle
                        .events()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                    let finished = match &mut event {
                        SearchEvent::Progress {
                            score: progress_score,
                            ..
                        } => {
                            // 採用値とは逆の判定になる進捗報告にする。
                            *progress_score = if resign { 100 } else { -29999 };
                            false
                        }
                        SearchEvent::Finished {
                            score: adopted_score,
                            ..
                        } => {
                            *adopted_score = score;
                            true
                        }
                    };
                    let before = output.len();
                    protocol
                        .handle_search_event(&engine, &mut active, event, &mut output)
                        .unwrap();
                    if finished {
                        break before;
                    }
                };
                if let Some(command) = release {
                    assert!(matches!(active, Some(ActiveSearch::AwaitingStop { .. })));
                    assert_eq!(
                        output.len(),
                        before_release,
                        "held result must not repeat info"
                    );
                    protocol
                        .handle_searching_line(
                            &engine,
                            &mut active,
                            &mut VecDeque::new(),
                            command,
                            &mut output,
                        )
                        .unwrap();
                }
                assert!(active.is_none());
                let released = std::str::from_utf8(&output[before_release..]).unwrap();
                assert!(
                    released.starts_with("info string stop depth\n"),
                    "{released}"
                );
                assert_eq!(released.lines().count(), 2, "{released}");
                if resign {
                    assert_eq!(bestmoves(released), ["bestmove resign"]);
                } else {
                    assert_legal_bestmove(released, &legal);
                }
            }
        }
    }

    // RS設計判断「判定を適用するbestmove」（D6-USI-55）。
    // 無限探索の完了イベントを処理してからstopを与え、保留中の解放を通す。
    #[test]
    fn resignation_infinite_completed_result_waits_for_stop() {
        let (mut engine, mut protocol) = resignation_engine(RESIGN_MATERIAL_SFEN);
        run(
            &mut protocol,
            &mut engine,
            "setoption name ResignValue value 1\n",
        );
        let mut output = Vec::new();
        let mut active = protocol
            .start_go(&engine, &["infinite"], &mut output)
            .unwrap();
        resignation_wait_for_iteration(&mut protocol, &engine, &mut active, &mut output);
        let Some(ActiveSearch::Running { handle, .. }) = active.as_ref() else {
            panic!("search must be running")
        };
        handle.request_stop();
        while matches!(active, Some(ActiveSearch::Running { .. })) {
            protocol
                .wait_search_event(&engine, &mut active, &mut output)
                .unwrap();
        }
        assert!(matches!(active, Some(ActiveSearch::AwaitingStop { .. })));
        assert!(bestmoves(std::str::from_utf8(&output).unwrap()).is_empty());
        let before = output.len();
        protocol
            .handle_searching_line(
                &engine,
                &mut active,
                &mut VecDeque::new(),
                "stop",
                &mut output,
            )
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&output[before..]).unwrap(),
            "info string stop external\nbestmove resign\n"
        );
    }

    // RS「判定を適用するbestmove」（D6-USI-51、D6-USI-56、D6-USI-58）。
    // Protocol::runの完了待ちもチャネル経路と同じ応答を返す。
    #[test]
    fn resignation_sequential_normal_and_ponderhit_return_resign() {
        for commands in ["go depth 2\n", "go ponder depth 2\nponderhit\n"] {
            let (mut engine, mut protocol) = resignation_engine(RESIGN_MATE_SFEN);
            let mut output = Vec::new();
            protocol
                .run(
                    &mut engine,
                    &mut std::io::Cursor::new(commands),
                    &mut output,
                )
                .unwrap();
            let output = String::from_utf8(output).unwrap();
            assert!(output.contains(" score mate -"), "{output}");
            assert!(
                output.ends_with("info string stop depth\nbestmove resign\n"),
                "{output}"
            );
            assert_eq!(bestmoves(&output), ["bestmove resign"]);
        }
    }

    // RS「局面の状態」、BG「stateコマンド」（D6-USI-59）。
    #[test]
    fn resignation_preserves_position_moves_and_gameover_lifecycle() {
        let (mut engine, mut protocol) = resignation_engine(RESIGN_MATE_SFEN);
        let mut output = run(&mut protocol, &mut engine, "state\nmoves\ngo depth 2\n");
        output.push_str(&run(
            &mut protocol,
            &mut engine,
            "state\nmoves\ngo depth 2\n",
        ));
        output.push_str(&run(&mut protocol, &mut engine, &format!(
            "position sfen {RESIGN_MATERIAL_SFEN}\nstate\ngameover lose\nstate\nposition sfen {RESIGN_MATE_SFEN}\nstate\ngo depth 2\n"
        )));
        assert_eq!(error_lines(&output), [STATE_ERROR]);
        assert_eq!(bestmoves(&output), ["bestmove resign"; 3]);
        let states = state_lines(&output);
        assert_eq!(states.len(), 4);
        assert_eq!(states[0], states[1]);
        assert_eq!(states[0], states[3]);
        assert!(states.iter().all(|line| line.ends_with("status ongoing")));
        assert!(states[2].contains(RESIGN_MATERIAL_SFEN.strip_suffix(" - 1").unwrap()));
        let moves = moves_sets(&output);
        assert_eq!(moves.len(), 2);
        assert_eq!(moves[0], moves[1]);
    }

    /// 実行中の探索から深さ1の進捗だけを処理し、停止経路へ渡す。
    fn resignation_wait_for_iteration(
        protocol: &mut UsiProtocol,
        engine: &Engine,
        active: &mut Option<ActiveSearch>,
        output: &mut Vec<u8>,
    ) {
        let Some(ActiveSearch::Running { handle, .. }) = active.as_ref() else {
            panic!("search must be running")
        };
        let event = handle
            .events()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(event, SearchEvent::Progress { depth: 1, score, .. } if score < 0));
        protocol
            .handle_search_event(engine, active, event, output)
            .unwrap();
    }

    // RS「判定を適用するbestmove」、PO「停止理由」（D6-USI-54）。
    // 完了イベントをまだ消費していないRunningからstopを処理する。
    #[test]
    fn resignation_running_stop_returns_resign_for_normal_infinite_and_ponder_searches() {
        for limits in [
            vec!["depth", "256"],
            vec!["infinite"],
            vec!["ponder", "depth", "256"],
        ] {
            let (mut engine, mut protocol) = resignation_engine(RESIGN_MATERIAL_SFEN);
            run(
                &mut protocol,
                &mut engine,
                "setoption name ResignValue value 1\n",
            );
            let mut output = Vec::new();
            let mut active = protocol.start_go(&engine, &limits, &mut output).unwrap();
            resignation_wait_for_iteration(&mut protocol, &engine, &mut active, &mut output);
            protocol
                .handle_searching_line(
                    &engine,
                    &mut active,
                    &mut VecDeque::new(),
                    "stop",
                    &mut output,
                )
                .unwrap();
            assert!(active.is_none());
            let output = String::from_utf8(output).unwrap();
            assert!(
                output.ends_with("info string stop external\nbestmove resign\n"),
                "{output}"
            );
            assert_eq!(bestmoves(&output), ["bestmove resign"]);
        }
    }

    // RS「判定を適用するbestmove」、PO「的中」（D6-USI-56、D6-USI-58）。
    #[test]
    fn resignation_running_ponderhit_continues_until_the_adopted_result() {
        let (mut engine, mut protocol) = resignation_engine(RESIGN_MATERIAL_SFEN);
        run(
            &mut protocol,
            &mut engine,
            "setoption name ResignValue value 1\n",
        );
        let mut output = Vec::new();
        let mut active = protocol
            .start_go(
                &engine,
                &["ponder", "depth", "256", "nodes", "50000"],
                &mut output,
            )
            .unwrap();
        resignation_wait_for_iteration(&mut protocol, &engine, &mut active, &mut output);
        protocol
            .handle_searching_line(
                &engine,
                &mut active,
                &mut VecDeque::new(),
                "ponderhit",
                &mut output,
            )
            .unwrap();
        assert!(matches!(active, Some(ActiveSearch::Running { .. })));
        assert!(bestmoves(std::str::from_utf8(&output).unwrap()).is_empty());
        while active.is_some() {
            protocol
                .wait_search_event(&engine, &mut active, &mut output)
                .unwrap();
        }
        let output = String::from_utf8(output).unwrap();
        assert!(
            output.ends_with("info string stop nodes\nbestmove resign\n"),
            "{output}"
        );
        assert_eq!(bestmoves(&output), ["bestmove resign"]);
    }

    // RS「判定を適用するbestmove」「判定の入力」（D6-USI-50、D6-USI-55、D6-USI-57）。
    // Finishedを先に処理して保留状態を確実に作り、実行中の経路と区別する。
    #[test]
    fn resignation_held_stop_and_ponderhit_preserve_score_and_completed_depth() {
        for command in ["stop", "ponderhit"] {
            for (limit, reason, resign) in [
                (vec!["ponder", "depth", "2"], "depth", true),
                (vec!["ponder", "nodes", "1"], "nodes", false),
            ] {
                let (mut engine, mut protocol) = resignation_engine(RESIGN_MATE_SFEN);
                run(
                    &mut protocol,
                    &mut engine,
                    "setoption name ResignValue value 1\n",
                );
                let legal = moves_sets(&run(&mut protocol, &mut engine, "moves\n")).remove(0);
                let mut output = Vec::new();
                let mut active = protocol.start_go(&engine, &limit, &mut output).unwrap();
                while matches!(active, Some(ActiveSearch::Running { .. })) {
                    protocol
                        .wait_search_event(&engine, &mut active, &mut output)
                        .unwrap();
                }
                assert!(matches!(active, Some(ActiveSearch::AwaitingStop { .. })));
                assert!(bestmoves(std::str::from_utf8(&output).unwrap()).is_empty());
                let before = output.len();
                protocol
                    .handle_searching_line(
                        &engine,
                        &mut active,
                        &mut VecDeque::new(),
                        command,
                        &mut output,
                    )
                    .unwrap();
                assert!(active.is_none());
                let released = std::str::from_utf8(&output[before..]).unwrap();
                assert!(released.starts_with(&format!("info string stop {reason}\n")));
                assert_eq!(released.lines().count(), 2);
                if resign {
                    assert_eq!(bestmoves(released), ["bestmove resign"]);
                } else {
                    assert_legal_bestmove(released, &legal);
                }
            }
        }
    }

    // RS「USI層の契約」（D6-USI-49）。待機列は現在のbestmoveより後に適用する。
    #[test]
    fn resignation_option_changes_wait_until_running_or_held_result_is_released() {
        for held in [false, true] {
            let (mut engine, mut protocol) = resignation_engine(RESIGN_MATERIAL_SFEN);
            run(
                &mut protocol,
                &mut engine,
                "setoption name ResignValue value 99999\n",
            );
            let legal = moves_sets(&run(&mut protocol, &mut engine, "moves\n")).remove(0);
            let mut output = Vec::new();
            let mut active = protocol
                .start_go(
                    &engine,
                    &["ponder", "depth", if held { "2" } else { "256" }],
                    &mut output,
                )
                .unwrap();
            if held {
                while matches!(active, Some(ActiveSearch::Running { .. })) {
                    protocol
                        .wait_search_event(&engine, &mut active, &mut output)
                        .unwrap();
                }
                assert!(matches!(active, Some(ActiveSearch::AwaitingStop { .. })));
            } else {
                resignation_wait_for_iteration(&mut protocol, &engine, &mut active, &mut output);
            }
            let mut pending = VecDeque::new();
            protocol
                .handle_searching_line(
                    &engine,
                    &mut active,
                    &mut pending,
                    "setoption name ResignValue value 1",
                    &mut output,
                )
                .unwrap();
            protocol
                .handle_searching_line(&engine, &mut active, &mut pending, "stop", &mut output)
                .unwrap();
            assert_legal_bestmove(std::str::from_utf8(&output).unwrap(), &legal);
            assert_eq!(pending.len(), 1);
            while let Some(command) = pending.pop_front() {
                protocol
                    .handle_idle_line(&mut engine, &command, &mut output)
                    .unwrap();
            }
            let next = run(&mut protocol, &mut engine, "go depth 1\n");
            assert_eq!(bestmoves(&next), ["bestmove resign"]);
        }
    }

    // RS「判定を適用するbestmove」、PO「先読み中のその他の入力」（D6-USI-60）。
    #[test]
    fn resignation_discarded_running_and_held_results_emit_no_bestmove() {
        for held in [false, true] {
            for command in ["gameover lose", "quit", "eof"] {
                let (mut engine, mut protocol) = resignation_engine(RESIGN_MATERIAL_SFEN);
                run(
                    &mut protocol,
                    &mut engine,
                    "setoption name ResignValue value 1\n",
                );
                let mut output = Vec::new();
                let mut active = protocol
                    .start_go(
                        &engine,
                        &["ponder", "depth", if held { "2" } else { "256" }],
                        &mut output,
                    )
                    .unwrap();
                if held {
                    while matches!(active, Some(ActiveSearch::Running { .. })) {
                        protocol
                            .wait_search_event(&engine, &mut active, &mut output)
                            .unwrap();
                    }
                    assert!(matches!(active, Some(ActiveSearch::AwaitingStop { .. })));
                } else {
                    resignation_wait_for_iteration(
                        &mut protocol,
                        &engine,
                        &mut active,
                        &mut output,
                    );
                }
                if command == "eof" {
                    protocol.discard_search(&mut active).unwrap();
                } else {
                    protocol
                        .handle_searching_line(
                            &engine,
                            &mut active,
                            &mut VecDeque::new(),
                            command,
                            &mut output,
                        )
                        .unwrap();
                }
                assert!(active.is_none());
                let output = String::from_utf8(output).unwrap();
                assert!(bestmoves(&output).is_empty(), "{output}");
                assert!(!output.contains("info string stop "), "{output}");
            }
        }
    }

    #[test]
    fn isready_allocates_the_default_transposition_table_before_the_first_go() {
        // 対局開始直後の手が置換表の確保を待たないよう、isreadyで既定容量の
        // 置換表を作る（time-management-efficiency.md「診断の指標」1）。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);
        assert!(protocol.transposition_table.is_none());
        let output = run(&mut protocol, &mut engine, "isready\n");
        assert_eq!(output, "readyok\n");
        assert!(protocol.transposition_table.is_some());
    }

    #[test]
    fn isready_returns_readyok_without_changing_state() {
        // PL「プロトコル固有の制御コマンド」（isreadyにはreadyok、同期実装では即時）（D6-USI-02）。
        assert_eq!(session(&[RuleCode::R1], "isready\nquit\n"), "readyok\n");

        let output = session(
            &[RuleCode::R1],
            "position startpos\nstate\nisready\nstate\n",
        );
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines[1], "readyok");
        // isreadyは状態を変更しない（前後のstateが不変）。
        assert_eq!(lines[0], lines[2]);
    }

    #[test]
    fn ruleset_default_is_the_canonical_form_of_the_startup_rules() {
        // PL「規則オプション」: 宣言defaultは起動時規則の正準表記（大文字・L,P,R,E順・番号昇順）（D6-USI-03）。
        let output = session(
            &[RuleCode::E1, RuleCode::R1, RuleCode::L2, RuleCode::L1],
            "usi\n",
        );
        assert!(output.contains("option name RuleSet type string default L1,L2,P0,R1,E0,E1\n"));

        // プリセット起動では展開後コード列だけが現れ、プリセット名は現れない（PL: 入力糖衣）。
        let output = lishogi_session("usi\n");
        assert!(output.contains("option name RuleSet type string default L1,L2,P0,P3,R1,E1,E3\n"));
        assert!(!output.contains("lishogi"));
    }

    #[test]
    fn ruleset_values_are_case_insensitive_and_duplicates_are_rejected() {
        // PL「規則オプション」: コンマ区切り・大小非区別・重複拒否（D6-USI-04）。
        let output = session(
            &[RuleCode::R1],
            concat!(
                "setoption name RuleSet value l1,p0,r2,e0\n",
                "setoption name RuleSet value L1,l1,P0,R1,E0\n",
                "usinewgame\nposition startpos\nstate\n",
            ),
        );
        let lines: Vec<_> = output.lines().collect();

        assert_eq!(lines.len(), 2);
        // 重複（大小違いも同一コード）は拒否され、pendingは変わらない。
        assert!(lines[0].starts_with("info string error: "));
        assert_eq!(state_rules(lines[1]), "L1,P0,R2,E0");
    }

    #[test]
    fn presets_expand_to_code_lists_and_reject_any_combination() {
        // PL「規則オプション」・RULES.md第33条第5・6項（D6-USI-05）。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);
        let output = run(
            &mut protocol,
            &mut engine,
            "setoption name RuleSet value LISHOGI\nusinewgame\nposition startpos\nstate\n",
        );
        assert_eq!(state_rules(state_lines(&output)[0]), "L1,L2,P0,P3,R1,E1,E3");
        assert!(!output.contains("lishogi"));

        let output = run(
            &mut protocol,
            &mut engine,
            "gameover win\nsetoption name RuleSet value engine-default\nposition startpos\nstate\n",
        );
        assert_eq!(state_rules(state_lines(&output)[0]), "L0,P0,R1,E0");

        // 併記とstandardは拒否し、直前の正当なpendingを保つ（PL 2026-08-11追記を含む）。
        let output = session(
            &[RuleCode::R1],
            concat!(
                "setoption name RuleSet value lishogi\n",
                "setoption name RuleSet value lishogi,P1\n",
                "setoption name RuleSet value lishogi,engine-default\n",
                "setoption name RuleSet value standard\n",
                "usinewgame\nposition startpos\nstate\n",
            ),
        );
        assert_eq!(error_lines(&output).len(), 3);
        assert_eq!(state_rules(state_lines(&output)[0]), "L1,L2,P0,P3,R1,E1,E3");
    }

    #[test]
    fn invalid_ruleset_values_are_rejected_on_receipt_and_pending_survives() {
        // PL「規則オプション」・PLコマンドenum（SetRulesは受信時検証）・R33第5項（D6-USI-06、D6-ENG-02）。
        let output = session(
            &[RuleCode::R1],
            concat!(
                "setoption name RuleSet value L1,P0,R2,E0\n",
                "setoption name RuleSet value XX9\n",
                "setoption name RuleSet value L1,E1\n", // 反復規則欠如は受信時に拒否
                "setoption name RuleSet value R0\n",    // R0は選択可能コードとして提供しない
                "usinewgame\nposition startpos\nstate\n",
            ),
        );
        assert_eq!(error_lines(&output).len(), 3);
        // 直前の正当なpending（L1,R2）が生きており、commitが旧pendingで成功する。
        assert_eq!(state_rules(state_lines(&output)[0]), "L1,P0,R2,E0");
    }

    #[test]
    fn ruleset_changes_latch_until_the_next_game() {
        // PL設計判断「規則指定」・実施状況フェーズ3（InGameのSetPositionはactive規則で再構成）（D6-USI-07）。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);

        let output = run(
            &mut protocol,
            &mut engine,
            "position startpos moves 6i6h\nsetoption name RuleSet value L1,P0,R2,E0\nstate\n",
        );
        assert_eq!(state_rules(state_lines(&output)[0]), "L0,P0,R1,E0");

        // 対局中の全列再送でもactive規則は変わらない。
        let output = run(
            &mut protocol,
            &mut engine,
            "position startpos moves 6i6h 1d1e\nstate\n",
        );
        assert_eq!(state_rules(state_lines(&output)[0]), "L0,P0,R1,E0");

        // commit点（次局開始）で初めて反映される。
        let output = run(
            &mut protocol,
            &mut engine,
            "gameover win\nposition startpos\nstate\n",
        );
        assert_eq!(state_rules(state_lines(&output)[0]), "L1,P0,R2,E0");
    }

    #[test]
    fn usi_variant_accepts_only_chushogi() {
        // PL「規則オプション」: USI_Variantは値chushogiだけを受理し、他はエラー通知（D6-USI-09）。
        let output = session(
            &[RuleCode::R1],
            concat!(
                "position startpos\n",
                "setoption name USI_Variant value chushogi\n",
                "state\n",
                "setoption name USI_Variant value shogi\n",
                "setoption name USI_Variant value standard\n",
                "state\n",
            ),
        );
        let states = state_lines(&output);

        assert_eq!(error_lines(&output).len(), 2);
        // 受理も拒否も状態を変えない。
        assert_eq!(states[0], states[1]);
    }

    #[test]
    fn startpos_is_read_as_the_chu_shogi_initial_position() {
        // PL「規則オプション」末尾の読み替え仕様・RULES.md第5条（D6-USI-10）。
        let startpos = session(&[RuleCode::R1], "position startpos\nstate\nmoves\n");
        let sfen = session(
            &[RuleCode::R1],
            &format!("position sfen {INITIAL_BOARD} - 1\nstate\nmoves\n"),
        );

        assert_eq!(
            state_lines(&startpos)[0],
            format!("state rules L0,P0,R1,E0 board {INITIAL_BOARD} status ongoing")
        );
        assert_eq!(state_lines(&startpos), state_lines(&sfen));
        assert_eq!(moves_sets(&startpos), moves_sets(&sfen));
    }

    #[test]
    fn position_applies_atomically_or_not_at_all() {
        // PLコマンドenum（commitの原子性）・「思考開始指示と終局裁定の通知」（D6-USI-11、D6-ENG-01/05）。
        let output = session(
            &[RuleCode::R1, RuleCode::E2],
            concat!(
                "position startpos moves 6i6h\n",
                "state\nmoves\n",
                "position startpos moves 6i6h 1a1b\n", // 末尾だけが不合法
                "state\nmoves\n",
                "position sfen broken-input\n", // 不正なSFEN
                "state\n",
                "position startpos moves 1a1b\n", // 先頭手が不合法
                "state\n",
            ),
        );
        let states = state_lines(&output);
        let moves = moves_sets(&output);

        assert_eq!(error_lines(&output).len(), 3);
        // 失敗したpositionは全体が無効果であり、直前の有効状態が保持される。
        assert_eq!(states.len(), 4);
        assert!(states.iter().all(|state| *state == states[0]));
        assert_eq!(moves.len(), 2);
        assert_eq!(moves[0], moves[1]);
    }

    #[test]
    fn incremental_position_matches_full_replay_for_ongoing_and_finished_games() {
        let moves = "3i3h 5a4b 8l9k 8b9b";
        let mut full_engine = make_engine(&[RuleCode::R1]);
        let mut full_protocol = UsiProtocol::new(&full_engine);
        let full_output = run(
            &mut full_protocol,
            &mut full_engine,
            &format!("position startpos moves {moves}\nstate\n"),
        );

        let mut incremental_engine = make_engine(&[RuleCode::R1]);
        let mut incremental_protocol = UsiProtocol::new(&incremental_engine);
        let incremental_output = run(
            &mut incremental_protocol,
            &mut incremental_engine,
            concat!(
                "position startpos\n",
                "position startpos moves 3i3h\n",
                "position startpos moves 3i3h 5a4b\n",
                "position startpos moves 3i3h 5a4b 8l9k\n",
                "position startpos moves 3i3h 5a4b 8l9k 8b9b\n",
                "state\n",
            ),
        );

        assert_eq!(incremental_output, full_output);
        assert_eq!(
            incremental_engine.game().position(),
            full_engine.game().position()
        );
        assert_eq!(incremental_engine.ply(), full_engine.ply());

        let mut full_engine = make_engine(&[RuleCode::R1, RuleCode::E2]);
        let mut full_protocol = UsiProtocol::new(&full_engine);
        let full_output = run(
            &mut full_protocol,
            &mut full_engine,
            &format!("position sfen {ROYAL_SFEN} moves 7g7d\nstate\n"),
        );
        let mut incremental_engine = make_engine(&[RuleCode::R1, RuleCode::E2]);
        let mut incremental_protocol = UsiProtocol::new(&incremental_engine);
        let incremental_output = run(
            &mut incremental_protocol,
            &mut incremental_engine,
            &format!("position sfen {ROYAL_SFEN}\nposition sfen {ROYAL_SFEN} moves 7g7d\nstate\n"),
        );

        assert_eq!(incremental_output, full_output);
        assert_eq!(
            incremental_engine.game().position(),
            full_engine.game().position()
        );
        assert_eq!(incremental_engine.ply(), full_engine.ply());
    }

    #[test]
    fn repeated_and_replaced_positions_match_fresh_replay() {
        // PLのposition契約: 再送、短縮、同数の別手順、開始局面だけの変更は、
        // それぞれ新しいpositionコマンドを単独で再生した結果に一致する。
        let changed_board = INITIAL_BOARD.replace("LFCSGKEGSCFL b", "LFCSGKEGSCF1 b");
        for command in [
            "position startpos moves 6i6h 1d1e".to_owned(),
            "position startpos moves 6i6h".to_owned(),
            "position startpos moves 3i3h 1d1e".to_owned(),
            format!("position sfen {changed_board} - 1 moves 6i6h 1d1e"),
        ] {
            let actual = session(
                &[RuleCode::R1],
                &format!("position startpos moves 6i6h 1d1e\n{command}\nstate\nmoves\n"),
            );
            let expected = session(&[RuleCode::R1], &format!("{command}\nstate\nmoves\n"));
            assert!(error_lines(&expected).is_empty(), "{command}: {expected}");
            assert_eq!(actual, expected, "{command}");
        }
    }

    #[test]
    fn malformed_position_kind_and_fields_are_rejected_atomically() {
        // 監査「positionコマンドの黙示的な無視と補完」: 解釈不能な種別、
        // startposの余剰欄、SFENの不正な第5欄と第6欄を黙って補完・無視しない。
        let output = session(
            &[RuleCode::R1],
            &format!(
                concat!(
                    "position startpos\nstate\n",
                    "position unknown\nstate\n",
                    "position startpos extra\nstate\n",
                    "position sfen {} - 1 typo\nstate\n",
                    "position sfen {} - 1 - extra\nstate\n",
                ),
                INITIAL_BOARD, INITIAL_BOARD
            ),
        );
        let states = state_lines(&output);

        assert_eq!(error_lines(&output).len(), 4);
        assert_eq!(states.len(), 5);
        assert!(states.iter().all(|state| *state == states[0]));
    }

    #[test]
    fn failed_position_blocks_go_until_a_valid_position_recovers() {
        // EC「Lishogi-Bot経路」: 局面同期失敗状態では正当なposition受理までgoを拒否する（D6-USI-12）。
        let output = session(
            &[RuleCode::R1],
            concat!(
                "position startpos moves 6i6h\n",
                "state\n",
                "position startpos moves 6i6h 1a1b\n",
                "state\n",
                "go depth 1\n",
                "position startpos moves 6i6h\n",
                "go depth 1\n",
            ),
        );
        let lines: Vec<_> = output.lines().collect();
        let states = state_lines(&output);

        // Engine.game自体は直前の有効状態を保持する（stateは旧局面を返す）。
        assert_eq!(states[0], states[1]);
        assert_eq!(error_lines(&output).len(), 2);
        // 同期失敗中のgoはbestmoveを生成せず、回復後のgoだけがbestmoveを返す。
        assert_eq!(bestmoves(&output).len(), 1);
        // 変異検証(フェーズ4)補強: 2件目のエラーは同期エラーそのものであり、
        // stale局面の探索が始まってはならない。
        assert!(error_lines(&output)[1].contains("go requires a synchronized position"));
        assert!(!output.contains("go is already running"));
        let last_error = lines
            .iter()
            .rposition(|line| line.starts_with("info string error: "))
            .unwrap();
        let bestmove = lines
            .iter()
            .position(|line| line.starts_with("bestmove "))
            .unwrap();
        assert!(last_error < bestmove);
    }

    #[test]
    fn lishogi_bot_series_completes_without_usinewgame() {
        // PLコマンドenum（Lishogi-Botはusinewgameを送らない）・EC「局面同期」（毎手全列再送）（D6-USI-13）。
        let mut engine = Engine::new(parse_rule_set("lishogi").unwrap()).unwrap();
        let mut protocol = UsiProtocol::new(&engine);

        let first = run(
            &mut protocol,
            &mut engine,
            concat!(
                "usi\nisready\n",
                "setoption name USI_Variant value chushogi\n",
                "position startpos\ngo depth 1\n",
            ),
        );
        assert_eq!(bestmoves(&first).len(), 1);
        let engine_move = bestmoves(&first)[0]
            .strip_prefix("bestmove ")
            .unwrap()
            .to_owned();

        // 応手はmoves照会（wire）で得た合法手から選ぶ。
        let second = run(
            &mut protocol,
            &mut engine,
            &format!("position startpos moves {engine_move}\nmoves\n"),
        );
        let reply = second
            .lines()
            .find(|line| line.starts_with("moves "))
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_owned();

        // 着手列は増分ではなく毎回全列で送られる。
        let third = run(
            &mut protocol,
            &mut engine,
            &format!("position startpos moves {engine_move} {reply}\ngo depth 1\n"),
        );
        assert_eq!(bestmoves(&third).len(), 1);
        for output in [&first, &second, &third] {
            assert!(error_lines(output).is_empty());
        }
    }

    #[test]
    fn go_time_arguments_are_accepted_and_normalized_per_side() {
        // EC実施状況フェーズ1: 時間引数の受理とミリ秒正規化、手番側の時計選択（D6-USI-14）。
        let go = "go btime 1000 wtime 2000 binc 10 winc 20 byoyomi 0 nodes 1";
        let black = session(&[RuleCode::R1], &format!("position startpos\n{go}\n"));

        assert_eq!(bestmoves(&black).len(), 1);

        // wire解析と単位正規化はプロトコル側の責務（EC「責務分担」）。手番側の選択を単体で固定する。
        let tokens = [
            "btime", "1000", "wtime", "2000", "binc", "30", "winc", "40", "byoyomi", "500",
        ];
        assert_eq!(
            parse_go_config(&tokens, Color::Black, 37).unwrap().clock(),
            Some(ClockLimits::new(1000, 30, 500, 37).unwrap())
        );
        assert_eq!(
            parse_go_config(&tokens, Color::White, 37).unwrap().clock(),
            Some(ClockLimits::new(2000, 40, 500, 37).unwrap())
        );
    }

    #[test]
    fn position_tracks_absolute_ply_from_sfen_and_moves() {
        // strength-stage2.md「予算式」: Engineの絶対手数はSFENの手数欄−1と、
        // positionで再生した着手数の和である。USIはこの値をgoの時計へ渡す。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);

        let output = run(
            &mut protocol,
            &mut engine,
            "position startpos moves 6i6h 6d6e\n",
        );
        assert!(error_lines(&output).is_empty());
        assert_eq!(engine.ply(), 2);

        let output = run(
            &mut protocol,
            &mut engine,
            &format!("position sfen {INITIAL_BOARD} - 41 moves 6i6h 6d6e\n"),
        );
        assert!(error_lines(&output).is_empty());
        assert_eq!(engine.ply(), 42);
    }

    #[test]
    fn bare_go_and_unknown_arguments_yield_errors_without_bestmove() {
        // EC実施状況フェーズ1: 裸のgoと未知引数はエラーとし、暗黙の既定値でフォールバックしない（D6-USI-15）。
        let output = session(&[RuleCode::R1], "position startpos\ngo\ngo foobar 3\n");

        assert_eq!(error_lines(&output).len(), 2);
        assert!(bestmoves(&output).is_empty());
    }

    #[test]
    fn go_depth_outside_one_through_256_is_rejected_without_bestmove() {
        // search.md「時間管理」: 深さ制約は1以上・最大探索ply(256)以下だけを受理し、
        // 範囲外はプロトコル層の入力検査でも拒否する（D6登録簿SU-01の文書補修済み）。
        let output = session(
            &[RuleCode::R1],
            "position startpos\ngo depth 0\ngo depth 257\n",
        );

        assert_eq!(error_lines(&output).len(), 2);
        assert!(bestmoves(&output).is_empty());
    }

    #[test]
    fn go_depth_256_at_the_upper_bound_is_accepted() {
        // 変異検証(フェーズ4)補強: 受理側境界。最大探索ply(256、search.md)ちょうどの
        // 指定は拒否されない。探索自体はノード制限の併用で即座に打ち切る。
        let output = session(
            &[RuleCode::R1],
            "position startpos\ngo depth 256 nodes 64\n",
        );

        assert!(error_lines(&output).is_empty());
        assert_eq!(bestmoves(&output).len(), 1);
        assert!(output.contains("info string stop nodes\n"));
    }

    #[test]
    fn go_depth_returns_a_deterministic_legal_bestmove_without_applying_it() {
        // EC「探索とbestmove」: 複製上の探索でEngine.gameへ適用しない（D6-USI-16、D6-USI-24）。
        // 探索の具体的な着手は評価依存のため固定せず、合法手集合への所属で検証する。
        let output = session(
            &[RuleCode::R1],
            "position startpos\nmoves\nstate\ngo depth 1\nstate\nmoves\n",
        );
        let states = state_lines(&output);
        let moves = moves_sets(&output);
        let bestmove = bestmoves(&output)[0]
            .strip_prefix("bestmove ")
            .unwrap()
            .to_owned();

        assert!(moves[0].contains(&bestmove));
        // goはEngine.gameに対して読み取り専用である。
        assert_eq!(states[0], states[1]);
        assert_eq!(moves[0], moves[1]);

        // 同一局面・同一引数の再実行は同一のbestmoveを返す（決定性）。
        let again = session(&[RuleCode::R1], "position startpos\ngo depth 1\n");
        assert_eq!(bestmoves(&output), bestmoves(&again));
        assert_search_info(&again);
    }

    #[test]
    fn go_mate_answers_checkmate_notimplemented() {
        // PL「思考開始指示と終局裁定の通知」・EC（現行のまま維持）（D6-USI-17）。引数によらず同一応答。
        assert_eq!(
            session(&[RuleCode::R1], "position startpos\ngo mate 10\ngo mate\n"),
            "checkmate notimplemented\ncheckmate notimplemented\n"
        );
    }

    #[test]
    fn ponder_without_limits_and_idle_ponderhit_are_rejected() {
        // ponder.md設計判断「go ponderの受理」「先読み中のその他の入力」（D6-USI-18、D6-USI-44）。
        let output = session(&[RuleCode::R1], "position startpos\ngo ponder\nponderhit\n");

        assert_eq!(error_lines(&output).len(), 2);
        assert!(bestmoves(&output).is_empty());
    }

    /// 出力行を1行ずつチャネルへ流す観測用ライター（D6-USI-19の対話的観測に使う）。
    struct LineWriter {
        sender: std::sync::mpsc::Sender<String>,
        bytes: Vec<u8>,
    }

    impl std::io::Write for LineWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            while let Some(newline) = self.bytes.iter().position(|&byte| byte == b'\n') {
                let line: Vec<_> = self.bytes.drain(..=newline).collect();
                let line = String::from_utf8(line)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                self.sender
                    .send(line)
                    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "reader closed"))?;
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn go_infinite_withholds_bestmove_until_stop() {
        // EC「思考情報」: go infiniteでは停止指示（stop）までbestmoveを出さない（D6-USI-19、D6-USI-20）。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);
        let (command_sender, command_receiver) = std::sync::mpsc::channel();
        let (line_sender, line_receiver) = std::sync::mpsc::channel();

        std::thread::scope(|scope| {
            let worker = scope.spawn(move || {
                let mut output = LineWriter {
                    sender: line_sender,
                    bytes: Vec::new(),
                };
                protocol.run_channel(&mut engine, &command_receiver, &mut output)
            });
            command_sender
                .send(Ok("position startpos".to_owned()))
                .unwrap();
            command_sender.send(Ok("go infinite".to_owned())).unwrap();

            // stop前にはbestmove行が現れない（info行は現れてよい）。
            loop {
                let line = line_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
                assert!(!line.starts_with("bestmove "));
                if line.starts_with("info depth ") {
                    break;
                }
            }

            command_sender.send(Ok("stop".to_owned())).unwrap();
            let mut stop_reason = None;
            loop {
                let line = line_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
                if line.starts_with("info string stop ") {
                    stop_reason = Some(line);
                    continue;
                }
                if line.starts_with("bestmove ") {
                    break;
                }
            }
            assert_eq!(stop_reason.as_deref(), Some("info string stop external\n"));
            drop(command_sender);
            worker.join().unwrap().unwrap();
        });
    }

    #[test]
    fn duplicate_go_is_rejected_and_bestmove_stays_unique() {
        // EC「探索中に届くコマンド」: 重複goはエラー情報行、stopで単一のbestmove（D6-USI-20、D6-USI-21）。
        let output = session(
            &[RuleCode::R1],
            "position startpos\ngo infinite\ngo depth 1\nstop\n",
        );
        let lines: Vec<_> = output.lines().collect();

        assert_eq!(error_lines(&output).len(), 1);
        assert_eq!(bestmoves(&output).len(), 1);
        // 台本末尾まで遅延bestmoveが漏れない（出力はinfo・エラー・bestmoveだけ）。
        assert!(lines.iter().all(|line| {
            line.starts_with("info depth ")
                || line.starts_with("info string error: ")
                || line.starts_with("info string stop ")
                || line.starts_with("bestmove ")
        }));
    }

    #[test]
    fn commands_arriving_during_search_apply_after_bestmove() {
        // EC「探索中に届くコマンド」: 停止指示以外は探索のjoin後（bestmove送出後）に適用する（D6-USI-22）。
        let output = session(
            &[RuleCode::R1],
            concat!(
                "position startpos\ngo infinite\n",
                "setoption name Threads value 2\n",
                "position startpos moves 6i6h\nisready\nstate\n",
                "stop\ngo depth 1\n",
            ),
        );
        assert!(error_lines(&output).is_empty());
        assert_eq!(bestmoves(&output).len(), 2);
        let lines: Vec<_> = output.lines().collect();
        let bestmove = lines
            .iter()
            .position(|line| line.starts_with("bestmove "))
            .unwrap();
        let readyok = lines.iter().position(|line| *line == "readyok").unwrap();
        let state = lines
            .iter()
            .position(|line| line.starts_with("state "))
            .unwrap();

        // 受信順の因果が出力順に保存される。
        assert!(bestmove < readyok);
        assert!(readyok < state);
        // join後に適用されたpositionが1手進んだ局面（手番w）を作っている。
        assert_eq!(lines[state].split_whitespace().nth(5), Some("w"));
    }

    #[test]
    fn multi_worker_search_places_info_immediately_before_one_bestmove() {
        // D6-USI-38。補助ワーカーの採用深さは非決定的なので数値を固定せず、
        // 採用結果のinfoが必要な場合もbestmove直前の構造を保つことを確認する。
        let output = session(
            &[RuleCode::R1],
            concat!(
                "setoption name Threads value 2\n",
                "position startpos\n",
                "go depth 4\n",
            ),
        );
        assert_search_info(&output);
    }

    #[test]
    fn gameover_and_quit_during_search_discard_the_result() {
        // EC「探索中に届くコマンド」: gameoverとquitは探索を停止して結果を破棄しbestmoveを返さない（D6-USI-23）。
        let output = session(
            &[RuleCode::R1],
            "position startpos\ngo infinite\ngameover win\nmoves\n",
        );
        assert!(bestmoves(&output).is_empty());
        // AwaitingStartへの復帰は後続movesのエラーで観測する。
        assert_eq!(output.lines().last().unwrap(), MOVES_ERROR);

        let output = session(&[RuleCode::R1], "position startpos\ngo infinite\nquit\n");
        assert!(bestmoves(&output).is_empty());
    }

    #[test]
    fn go_outside_an_active_game_is_rejected_without_bestmove() {
        // EC「探索とbestmove」・設計判断「goのライフサイクル契約」（D6-USI-25）。
        // USI原典はgoにbestmoveを要求するが、本設計はこの系列を契約外と明記する（意図的逸脱の固定）。
        let awaiting = session(&[RuleCode::R1], "go depth 1\n");
        assert_eq!(error_lines(&awaiting).len(), 1);
        assert!(bestmoves(&awaiting).is_empty());

        let finished = session(
            &[RuleCode::R1, RuleCode::E2],
            &format!("position sfen {ROYAL_SFEN} moves 7g7d\ngo depth 1\n"),
        );
        assert_eq!(error_lines(&finished).len(), 1);
        assert!(bestmoves(&finished).is_empty());
    }

    // EC「思考情報」: 完了理由とinfoの構文を既存の1・複数ワーカー探索で検査する。
    fn assert_search_info(output: &str) {
        let lines: Vec<_> = output.lines().collect();

        assert!(lines.last().unwrap().starts_with("bestmove "));
        assert_eq!(lines[lines.len() - 2], "info string stop depth");
        let info_lines = &lines[..lines.len() - 2];
        assert!(!info_lines.is_empty());
        // infoはbestmoveより前にだけ現れる。
        for line in info_lines {
            let tokens: Vec<_> = line.split_whitespace().collect();
            assert_eq!(tokens[0], "info");
            assert_eq!(tokens[1], "depth");
            tokens[2].parse::<u64>().unwrap();
            assert_eq!(tokens[3], "score");
            assert!(tokens[4] == "cp" || tokens[4] == "mate");
            tokens[5].parse::<i64>().unwrap();
            assert_eq!(tokens[6], "nodes");
            tokens[7].parse::<u64>().unwrap();
            assert_eq!(tokens[8], "nps");
            tokens[9].parse::<u64>().unwrap();
            assert_eq!(tokens[10], "time");
            tokens[11].parse::<u64>().unwrap();
            assert_eq!(tokens[12], "pv");
            assert!(tokens.len() > 13);
            // PVの各手はlishogi系USI指し手構文の文字だけからなる。
            for mv in &tokens[13..] {
                assert!(
                    mv.chars()
                        .all(|c| c.is_ascii_digit() || ('a'..='l').contains(&c) || c == '+')
                );
            }
        }
    }

    #[test]
    fn clock_stop_reasons_have_distinct_protocol_words() {
        assert_eq!(stop_reason_text(StopReason::SoftLimit), "soft");
        assert_eq!(stop_reason_text(StopReason::HardLimit), "hard");
    }

    #[test]
    fn hash_rejects_invalid_sizes_and_resizes_without_changing_the_game() {
        // ECの非探索中リサイズと、極大の外部設定値によるオーバーフローの回帰。
        let output = session(
            &[RuleCode::R1],
            &format!(
                concat!(
                    "position startpos\nstate\n",
                    "setoption name USI_Hash value 0\n",
                    "setoption name USI_Hash value {}\n",
                    "state\n",
                    "setoption name USI_Hash value 1\n",
                    "setoption name USI_Hash value 2\n",
                    "state\ngo depth 1\n",
                ),
                usize::MAX
            ),
        );
        let states = state_lines(&output);
        assert_eq!(error_lines(&output).len(), 2);
        assert_eq!(states.len(), 3);
        assert!(states.iter().all(|state| *state == states[0]));
        assert_eq!(bestmoves(&output).len(), 1);
    }

    #[test]
    fn royal_capture_finishes_silently_and_state_reports_the_verdict() {
        // PL「思考開始指示と終局裁定の通知」（USIは終局を出力しない）・EC「終局責任」（D6-USI-28、D6-USI-34、D6-ENG-04）。
        let mut engine = make_engine(&[RuleCode::R1, RuleCode::E2]);
        let mut protocol = UsiProtocol::new(&engine);

        // 王駒捕獲（RULES.md第21条）で終局しても、USI経路には自発出力が一切ない。
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("position sfen {ROYAL_SFEN} moves 7g7d\n"),
            ),
            ""
        );
        // 裁定はrun終了後のGameStatusで検証する（PL検証の節が明示的に許す唯一の内部観測、D6-ENG-07）。
        assert_eq!(
            engine.status(),
            GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::RoyalCapture,
            })
        );

        // Finished局面への同一position全列の再送は終局通知や重複出力を生まない（D6-ENG-04）。
        let output = run(
            &mut protocol,
            &mut engine,
            &format!("state\nposition sfen {ROYAL_SFEN} moves 7g7d\n"),
        );
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines[0], ROYAL_FINISHED_STATE);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].starts_with("info string error: "));

        // gameoverは無応答でAwaitingStartへ戻る（後続movesのエラーで観測）。
        assert_eq!(
            run(&mut protocol, &mut engine, "gameover win\nmoves\n"),
            format!("{MOVES_ERROR}\n")
        );
    }

    #[test]
    fn unknown_commands_are_ignored_and_quit_is_silent() {
        // PL「USIの未知入力は原典準拠」: 未知コマンド行は無視する（D6-USI-29）。
        let noisy = session(
            &[RuleCode::R1],
            "foobar baz\nposition startpos\nstate\nquit\nusi\n",
        );
        let clean = session(&[RuleCode::R1], "position startpos\nstate\n");

        // 未知入力は解釈へ影響せず、quit後の入力（usi）は処理されない。
        assert_eq!(noisy, clean);
    }

    #[test]
    fn moves_returns_the_legal_move_set_as_one_line() {
        // BG「movesコマンド」: Game::legal_moves()の全要素を既存USI表記で1行、順序は契約外（D6-USI-30）。
        let output = session(
            &[RuleCode::R1],
            "position startpos\nmoves\nstate\nmoves\nstate\n",
        );
        let moves = moves_sets(&output);
        let states = state_lines(&output);

        // 期待集合はBGが定義するとおりGame::legal_moves()のUSI表記から作る。
        let game = Game::new(Rules::ENGINE_DEFAULT);
        let expected: HashSet<String> = game
            .legal_moves()
            .into_iter()
            .map(|mv| usi::text_generated(game.position(), mv))
            .collect();

        assert_eq!(moves[0], expected);
        // movesは読み取り専用（前後でstateもmovesも不変）。
        assert_eq!(moves[0], moves[1]);
        assert_eq!(states[0], states[1]);
    }

    #[test]
    fn moves_requires_an_active_game() {
        // BG: AwaitingStartとFinishedのmovesは固定エラー行、stateはFinishedで応答する非対称（D6-USI-31）。
        assert_eq!(
            session(&[RuleCode::R1], "moves\n"),
            format!("{MOVES_ERROR}\n")
        );

        let output = session(
            &[RuleCode::R1, RuleCode::E2],
            &format!("position sfen {ROYAL_SFEN} moves 7g7d\nmoves\nstate\n"),
        );
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines[0], MOVES_ERROR);
        assert_eq!(lines[1], ROYAL_FINISHED_STATE);
    }

    #[test]
    fn state_line_matches_the_exact_contract() {
        // BG「stateコマンド」: 単一行の完全一致契約（D6-USI-32）。
        assert_eq!(
            lishogi_session("position startpos\nstate\n"),
            format!("state rules L1,L2,P0,P3,R1,E1,E3 board {INITIAL_BOARD} status ongoing\n")
        );
    }

    #[test]
    fn state_status_vocabulary_matches_the_contract() {
        // BGの公開status語彙表（D6-USI-32）。勝因の語彙と勝者の色を検査する。
        let win_reasons = [
            (WinReason::RoyalCapture, "royal-capture"),
            (WinReason::Repetition, "repetition"),
            (WinReason::PieceExhaustion, "piece-exhaustion"),
            (WinReason::BareKing, "bare-king"),
            (WinReason::Stalemate, "stalemate"),
            (WinReason::Mate, "mate"),
        ];
        for (reason, text) in win_reasons {
            assert_eq!(
                state_status_text(GameStatus::Finished(GameResult::Win {
                    winner: Color::White,
                    reason,
                })),
                Ok(format!("win white {text}"))
            );
        }

        let draw_reasons = [
            (DrawReason::Repetition, "repetition"),
            (DrawReason::PieceExhaustion, "piece-exhaustion"),
            (DrawReason::BareKing, "bare-king"),
        ];
        for (reason, text) in draw_reasons {
            assert_eq!(
                state_status_text(GameStatus::Finished(GameResult::Draw { reason })),
                Ok(format!("draw {text}"))
            );
        }

        assert_eq!(
            state_status_text(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::RoyalCapture,
            })),
            Ok("win black royal-capture".to_owned())
        );
    }

    #[test]
    fn state_lifecycle_spans_finished_and_next_game_rules() {
        // BG: stateはInGameとFinishedで応答し、AwaitingStartではエラー。次局はactive規則を返す（D6-USI-33）。
        assert_eq!(
            session(&[RuleCode::R1], "state\n"),
            format!("{STATE_ERROR}\n")
        );

        let mut engine = make_engine(&[RuleCode::R1, RuleCode::E2]);
        let mut protocol = UsiProtocol::new(&engine);
        let output = run(
            &mut protocol,
            &mut engine,
            &format!("position sfen {ROYAL_SFEN} moves 7g7d\nstate\n"),
        );
        assert_eq!(output, format!("{ROYAL_FINISHED_STATE}\n"));

        // gameover後は終局済み局面を内部に保持していてもstateは応答しない。
        assert_eq!(
            run(&mut protocol, &mut engine, "gameover win\nstate\n"),
            format!("{STATE_ERROR}\n")
        );

        let output = run(
            &mut protocol,
            &mut engine,
            "setoption name RuleSet value L0,P0,R2,E2\nposition startpos\nstate\n",
        );
        assert_eq!(state_rules(state_lines(&output)[0]), "L0,P0,R2,E2");
    }

    #[test]
    fn failed_awaiting_start_commit_changes_nothing() {
        // PLコマンドenum: commitは全適用か無効果かの原子的操作（D6-ENG-01のwire観測）。
        let output = session(
            &[RuleCode::R1],
            concat!(
                "setoption name RuleSet value L0,P0,R2,E2\n",
                "position startpos moves 1a1b\n", // 失敗するcommit
                "moves\n",                        // ライフサイクルが遷移していない証拠
                "position startpos\n",
                "state\n",
            ),
        );
        let lines: Vec<_> = output.lines().collect();

        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("info string error: "));
        assert_eq!(lines[1], MOVES_ERROR);
        // pendingは失敗をまたいで保持され、次の成功したcommitで反映される。
        assert_eq!(state_rules(lines[2]), "L0,P0,R2,E2");
    }
    // ponder.md設計判断「go ponderの受理」「USI_Ponder」（D6-USI-18）。
    #[test]
    fn ponder_accepts_finite_limits_and_ignores_usi_ponder_option() {
        for args in [
            "ponder depth 1",
            "ponder nodes 100",
            "ponder btime 1000 wtime 1000 byoyomi 0",
            "ponder movetime 20",
            "depth 2 ponder nodes 100 movetime 20 btime 1000",
        ] {
            assert!(
                parse_go_config(
                    &args.split_whitespace().collect::<Vec<_>>(),
                    Color::Black,
                    0
                )
                .is_ok(),
                "{args}"
            );
        }
        for args in ["ponder", "ponder infinite", "ponder infinite depth 1"] {
            let output = session(
                &[RuleCode::R1],
                &format!("position startpos\ngo {args}\ngo depth 1\n"),
            );
            assert_eq!(error_lines(&output).len(), 1, "{output}");
            assert_eq!(bestmoves(&output).len(), 1);
        }
        for option in ["true", "false"] {
            let output = session(
                &[RuleCode::R1],
                &format!(
                    "usi\nsetoption name USI_Ponder value {option}\nposition startpos\ngo depth 2\n"
                ),
            );
            assert!(!output.contains("option name USI_Ponder"));
            assert!(error_lines(&output).is_empty());
            assert!(bestmoves(&output)[0].contains(" ponder "));
        }
    }

    /// 既存の観測用ライターで入力と出力を1行ずつ往復させる。
    fn ponder_dialogue(
        drive: impl FnOnce(&std::sync::mpsc::Sender<io::Result<String>>, &Receiver<String>),
    ) {
        let (commands, input) = std::sync::mpsc::channel();
        let (lines, output) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let worker = scope.spawn(move || {
                let mut engine = make_engine(&[RuleCode::R1]);
                let mut protocol = UsiProtocol::new(&engine);
                protocol
                    .run_channel(
                        &mut engine,
                        &input,
                        &mut LineWriter {
                            sender: lines,
                            bytes: Vec::new(),
                        },
                    )
                    .unwrap();
            });
            drive(&commands, &output);
            drop(commands);
            worker.join().unwrap();
            assert!(output.try_iter().all(|line| !line.starts_with("bestmove ")));
        });
    }

    fn receive_line(lines: &Receiver<String>) -> String {
        lines.recv_timeout(Duration::from_secs(5)).unwrap()
    }

    // ponder.md「USI層の契約」、設計判断「bestmoveの保留」「停止理由」
    // （D6-USI-39、D6-USI-41、D6-USI-42、D6-USI-43）。
    #[test]
    fn ponder_channel_withholds_completed_result_until_hit_or_stop() {
        for trigger in ["ponderhit", "stop"] {
            ponder_dialogue(|commands, lines| {
                commands.send(Ok("position startpos".into())).unwrap();
                commands.send(Ok("go ponder depth 2".into())).unwrap();
                let final_pv = loop {
                    let line = receive_line(lines);
                    assert!(!line.starts_with("bestmove "));
                    assert!(!line.starts_with("info string error:"));
                    if line.starts_with("info depth 2 ") {
                        break line;
                    }
                };
                commands.send(Ok("isready".into())).unwrap();
                // 重複goのエラーを入力処理の目印にする。readyokとbestmoveはまだ出ない。
                commands.send(Ok("go depth 1".into())).unwrap();
                let marker = receive_line(lines);
                assert!(marker.starts_with("info string error:"), "{marker}");
                commands.send(Ok(trigger.into())).unwrap();
                assert_eq!(receive_line(lines), "info string stop depth\n");
                let best = receive_line(lines);
                let words: Vec<_> = best.split_whitespace().collect();
                let pv: Vec<_> = final_pv
                    .split(" pv ")
                    .nth(1)
                    .unwrap()
                    .split_whitespace()
                    .collect();
                assert_eq!(words, ["bestmove", pv[0], "ponder", pv[1]]);
                assert_eq!(receive_line(lines), "readyok\n");
                // 外形でも予想手の合法性を確認する。
                commands
                    .send(Ok(format!("position startpos moves {}", words[1])))
                    .unwrap();
                commands.send(Ok("moves".into())).unwrap();
                assert!(
                    receive_line(lines)
                        .split_whitespace()
                        .any(|word| word == words[3])
                );
                commands.send(Ok("ponderhit".into())).unwrap();
                assert!(receive_line(lines).starts_with("info string error:"));
            });
        }
    }

    // ponder.md「USI層の契約」、設計判断「停止理由」「先読み中のその他の入力」
    // （D6-USI-42、D6-USI-43、D6-USI-44）。時計に依存せず有限ノードで終了させる。
    #[test]
    fn ponder_channel_hit_continues_search_and_miss_recovers() {
        for trigger in ["ponderhit", "stop"] {
            ponder_dialogue(|commands, lines| {
                commands.send(Ok("position startpos".into())).unwrap();
                commands
                    .send(Ok("go ponder depth 256 nodes 50000".into()))
                    .unwrap();
                let line = receive_line(lines);
                assert!(line.starts_with("info depth 1 "), "{line}");
                commands.send(Ok(trigger.into())).unwrap();
                let mut stop = String::new();
                loop {
                    let line = receive_line(lines);
                    if line.starts_with("info string stop ") {
                        stop = line.clone();
                    }
                    if line.starts_with("bestmove ") {
                        break;
                    }
                }
                assert_eq!(
                    stop,
                    if trigger == "ponderhit" {
                        "info string stop nodes\n"
                    } else {
                        "info string stop external\n"
                    }
                );
                commands
                    .send(Ok("position startpos moves 6i6h".into()))
                    .unwrap();
                commands.send(Ok("go depth 1".into())).unwrap();
                loop {
                    let line = receive_line(lines);
                    assert!(!line.starts_with("info string error:"));
                    if line.starts_with("bestmove ") {
                        break;
                    }
                }
            });
        }
    }

    // ponder.md設計判断「bestmoveの保留」「先読み中のその他の入力」
    // （D6-USI-41、D6-USI-42、D6-USI-43、D6-USI-44、D6-USI-45）。
    // 完了イベントを明示的に消費して保留状態に入り、スケジューリングに依存せず全遷移を通す。
    #[test]
    fn ponder_held_results_release_once_or_are_discarded() {
        for command in ["ponderhit", "stop", "gameover win", "quit", "eof"] {
            let mut engine = make_engine(&[RuleCode::R1]);
            let mut protocol = UsiProtocol::new(&engine);
            let (sender, lines) = std::sync::mpsc::channel();
            let mut output = LineWriter {
                sender,
                bytes: Vec::new(),
            };
            protocol
                .handle_idle_line(&mut engine, "position startpos", &mut output)
                .unwrap();
            let mut active = protocol
                .start_go(&engine, &["ponder", "depth", "2"], &mut output)
                .unwrap();
            while matches!(active, Some(ActiveSearch::Running { .. })) {
                protocol
                    .wait_search_event(&engine, &mut active, &mut output)
                    .unwrap();
            }
            assert!(matches!(active, Some(ActiveSearch::AwaitingStop { .. })));
            assert!(lines.try_iter().all(|line| !line.starts_with("bestmove ")));
            let mut pending = VecDeque::new();
            if command == "eof" {
                protocol.discard_search(&mut active).unwrap();
            } else {
                protocol
                    .handle_searching_line(&engine, &mut active, &mut pending, command, &mut output)
                    .unwrap();
            }
            assert!(active.is_none());
            let result: String = lines.try_iter().collect();
            if matches!(command, "ponderhit" | "stop") {
                assert_eq!(bestmoves(&result).len(), 1);
                assert!(result.contains("info string stop depth\n"));
                assert!(bestmoves(&result)[0].contains(" ponder "));
            } else {
                assert!(bestmoves(&result).is_empty());
            }
        }
    }

    // ponder.md設計判断「先読み中のその他の入力」（D6-USI-44、D6-USI-45）。
    #[test]
    fn ponder_invalid_hits_and_discard_commands_preserve_lifecycle_contracts() {
        let output = session(
            &[RuleCode::R1],
            "position startpos\ngo infinite\nponderhit\nstop\n",
        );
        assert_eq!(error_lines(&output).len(), 1);
        assert_eq!(bestmoves(&output).len(), 1);
        let output = session(
            &[RuleCode::R1],
            "position startpos\ngo ponder depth 256 nodes 10000\nponderhit\nponderhit\n",
        );
        assert_eq!(error_lines(&output).len(), 1);
        assert_eq!(bestmoves(&output).len(), 1);
        for tail in ["gameover win\nmoves\n", "quit\n", ""] {
            let output = session(
                &[RuleCode::R1],
                &format!("position startpos\ngo ponder depth 256\n{tail}"),
            );
            assert!(bestmoves(&output).is_empty());
            if tail.starts_with("gameover") {
                assert_eq!(error_lines(&output), [MOVES_ERROR]);
            }
        }
        let output = session(
            &[RuleCode::R1],
            "position startpos\ngo ponder depth 256\nposition startpos moves 6i6h\nstop\nstate\nquit\n",
        );
        assert_eq!(bestmoves(&output).len(), 1);
        assert_eq!(
            state_lines(&output),
            state_lines(&session(
                &[RuleCode::R1],
                "position startpos moves 6i6h\nstate\n"
            ))
        );
        assert!(output.find("bestmove ").unwrap() < output.find("state rules ").unwrap());
    }

    // ponder.md設計判断「逐次経路」（D6-USI-46、D6-USI-45）。
    // 次の入力を読もうとする時点でbestmoveが既に出力済みであることを観測する。
    #[test]
    fn ponder_sequential_hit_finishes_before_reading_another_command() {
        struct Input {
            lines: std::collections::VecDeque<&'static str>,
            output: Receiver<String>,
        }
        impl std::io::Read for Input {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                unreachable!()
            }
        }
        impl BufRead for Input {
            fn fill_buf(&mut self) -> io::Result<&[u8]> {
                unreachable!()
            }
            fn consume(&mut self, _: usize) {
                unreachable!()
            }
            fn read_line(&mut self, buffer: &mut String) -> io::Result<usize> {
                let Some(line) = self.lines.pop_front() else {
                    return Ok(0);
                };
                if line == "isready\n" {
                    let output: String = self.output.try_iter().collect();
                    assert_eq!(bestmoves(&output).len(), 1, "{output}");
                }
                buffer.push_str(line);
                Ok(line.len())
            }
        }
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);
        let (sender, output) = std::sync::mpsc::channel();
        let mut input = Input {
            lines: [
                "position startpos\n",
                "go ponder depth 2\n",
                "ponderhit\n",
                "isready\n",
            ]
            .into(),
            output,
        };
        protocol
            .run(
                &mut engine,
                &mut input,
                &mut LineWriter {
                    sender,
                    bytes: Vec::new(),
                },
            )
            .unwrap();
        let output: String = input.output.try_iter().collect();
        assert_eq!(output, "readyok\n");
        let mut output = Vec::new();
        protocol
            .run(
                &mut engine,
                &mut std::io::Cursor::new("go ponder depth 2\n"),
                &mut output,
            )
            .unwrap();
        assert!(bestmoves(&String::from_utf8(output).unwrap()).is_empty());
    }

    // ponder.md設計判断「予想手の出所」「予想手の検査」（D6-USI-39、D6-USI-40）。
    #[test]
    fn ponder_prediction_uses_adopted_pv_and_rejects_illegal_or_finishing_moves() {
        for threads in [1, 2] {
            let output = session(
                &[RuleCode::R1],
                &format!("setoption name Threads value {threads}\nposition startpos\ngo depth 2\n"),
            );
            let best = bestmoves(&output);
            assert_eq!(best.len(), 1, "{output}");
            let best: Vec<_> = best[0].split_whitespace().collect();
            let pv: Vec<_> = output
                .lines()
                .filter_map(|line| line.split_once(" pv ").map(|(_, pv)| pv))
                .next_back()
                .unwrap()
                .split_whitespace()
                .collect();
            // 共有置換表による打ち切りでは、深さ2でもPVが1手になり得る。
            match pv.as_slice() {
                [first] => assert_eq!(best, ["bestmove", *first], "{output}"),
                [first, second, ..] => {
                    assert_eq!(best, ["bestmove", *first, "ponder", *second], "{output}");
                }
                [] => panic!("empty PV: {output}"),
            }
        }
        let output = session(&[RuleCode::R1], "position startpos\ngo depth 1\n");
        assert_eq!(bestmoves(&output)[0].split_whitespace().count(), 2);
        use crate::PieceKind;
        use crate::test_util::{position, sq};
        let step = |from, to| Move {
            from,
            to,
            mid: None,
            promote: false,
        };
        let kings = position(
            Color::Black,
            &[
                (sq(3, 3), Color::Black, PieceKind::King),
                (sq(8, 8), Color::White, PieceKind::King),
            ],
        );
        // 探索のスケジュールによらず、2手のPVと1手のPVを両方検査する。
        let game = Game::from_position(
            Rules::from_codes(&[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E2]).unwrap(),
            kings.clone(),
        );
        let x = step(sq(3, 3), sq(3, 4));
        let y = step(sq(8, 8), sq(8, 7));
        assert_eq!(
            validated_ponder_move(&game, x, &[x, y]).as_deref(),
            Some("4d4e")
        );
        assert_eq!(validated_ponder_move(&game, x, &[x]), None);
        for rule in [RuleCode::R2, RuleCode::R3] {
            let rules =
                Rules::from_codes(&[RuleCode::L0, RuleCode::P0, rule, RuleCode::E2]).unwrap();
            let mut game = Game::from_position(rules, kings.clone());
            if rule == RuleCode::R3 {
                for _ in 0..2 {
                    for mv in [
                        step(sq(3, 3), sq(3, 4)),
                        step(sq(8, 8), sq(8, 7)),
                        step(sq(3, 4), sq(3, 3)),
                        step(sq(8, 7), sq(8, 8)),
                    ] {
                        game.play(mv).unwrap();
                    }
                }
            }
            game.play(step(sq(3, 3), sq(3, 4))).unwrap();
            game.play(step(sq(8, 8), sq(8, 7))).unwrap();
            let x = step(sq(3, 4), sq(3, 3));
            let y = step(sq(8, 7), sq(8, 8));
            assert_eq!(validated_ponder_move(&game, x, &[x, y]), None);
        }
        let board = position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(8, 8), Color::White, PieceKind::King),
                (sq(1, 8), Color::White, PieceKind::Rook),
            ],
        );
        let game = Game::from_position(
            Rules::from_codes(&[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E2]).unwrap(),
            board,
        );
        let x = step(sq(0, 0), sq(1, 0));
        let y = step(sq(1, 8), sq(1, 0));
        assert_eq!(validated_ponder_move(&game, x, &[x, y]), None);
        assert_eq!(validated_ponder_move(&game, y, &[y, x]), None); // Xが不合法。
        assert_eq!(validated_ponder_move(&game, x, &[x, x]), None); // Yが不合法。
    }

    // ponder.md設計判断「外れからの復帰」（D6-ENG-08、D6-USI-11、D6-ENG-05）。
    #[test]
    fn ponder_last_move_replacement_matches_replay_and_is_atomic() {
        let mut engine = make_engine(&[RuleCode::R2, RuleCode::E2]);
        let mut protocol = UsiProtocol::new(&engine);
        for moves in [
            "6i6h",
            "5i5h",
            "6i6h",
            "6i6h 6d6e",
            "6i6h 5d5e",
            "6i6h 6d6e",
            "6i6h 6d6e 5i5h",
            "5i5h 5d5e 6i6h",
        ] {
            let input = format!("position startpos moves {moves}\nstate\nmoves\n");
            let actual = run(&mut protocol, &mut engine, &input);
            assert_eq!(actual, session(&[RuleCode::R2, RuleCode::E2], &input));
            assert!(error_lines(&actual).is_empty());
        }
        let output = run(
            &mut protocol,
            &mut engine,
            "state\nposition startpos moves 5i5h 5d5e 1a1b\nstate\ngo depth 1\n",
        );
        assert_eq!(state_lines(&output)[0], state_lines(&output)[1]);
        assert_eq!(error_lines(&output).len(), 2);
        assert!(bestmoves(&output).is_empty());
        // 反復禁止の履歴も、末尾の交換とその後の延長を通じて全再生と一致する。
        let base = "12/12/12/8k3/12/12/12/12/3K8/12/12/12 b - 41";
        let mut engine = make_engine(&[RuleCode::R2, RuleCode::E2]);
        let mut protocol = UsiProtocol::new(&engine);
        for moves in ["9i9h 4d4e", "9i9h 4d5d", "9i9h 4d4e", "9i9h 4d4e 9h9i"] {
            let input = format!("position sfen {base} moves {moves}\nstate\nmoves\n");
            let actual = run(&mut protocol, &mut engine, &input);
            assert_eq!(actual, session(&[RuleCode::R2, RuleCode::E2], &input));
            assert!(error_lines(&actual).is_empty(), "{actual}");
            if moves.ends_with("9h9i") {
                assert!(!moves_sets(&actual)[0].contains("4e4d"));
            }
        }
    }
    /// 固定シードで非捕獲手を選び、終局を避けて4,000手の履歴を作る。
    /// 捕獲を避けるのは、終盤の合法手枯渇で標本が短くなるのを防ぐため。
    fn ponder_long_position() -> (Engine, UsiProtocol) {
        let rules = Rules::ENGINE_DEFAULT;
        let mut game = Game::new(rules);
        let mut rng = crate::rng::XorShift64::new(std::num::NonZeroU64::new(20260920).unwrap());
        let mut tokens = Vec::new();
        for _ in 0..4000 {
            let candidates: Vec<_> = game
                .legal_moves()
                .into_iter()
                .filter(|&mv| {
                    game.position()
                        .captured_squares(mv)
                        .iter()
                        .all(Option::is_none)
                })
                .collect();
            assert!(
                !candidates.is_empty(),
                "long-game fixture exhausted its quiet moves"
            );
            let start = rng.index(NonZeroUsize::new(candidates.len()).unwrap());
            let (mv, next) = (0..candidates.len())
                .find_map(|offset| {
                    let mv = candidates[(start + offset) % candidates.len()];
                    let mut next = game.clone();
                    (next.play(mv).unwrap() == GameStatus::Ongoing).then_some((mv, next))
                })
                .expect("long-game fixture must continue");
            tokens.push(usi::text_generated(game.position(), mv));
            game = next;
        }
        let mut engine = Engine::new(parse_rule_set("engine-default").unwrap()).unwrap();
        let mut protocol = UsiProtocol::new(&engine);
        let mut output = Vec::new();
        protocol
            .handle_idle_line(
                &mut engine,
                &format!("position startpos moves {}", tokens.join(" ")),
                &mut output,
            )
            .unwrap();
        assert!(output.is_empty());
        assert_eq!(engine.ply(), 4000);
        assert_eq!(engine.game().position(), game.position());
        (engine, protocol)
    }

    // ponder.md「検証」の固定費（D6-USI-39、D6-USI-40、D6-ENG-08）。
    #[test]
    #[ignore = "releaseで4,000手の履歴に対する固定費を測る"]
    fn ponder_long_game_output_and_replacement_stay_within_three_ms() {
        let (mut engine, mut protocol) = ponder_long_position();
        let x = engine
            .game()
            .legal_moves()
            .into_iter()
            .find(|&mv| {
                let mut next = engine.game().clone();
                next.play(mv).unwrap() == GameStatus::Ongoing
            })
            .unwrap();
        let mut after_x = engine.game().clone();
        after_x.play(x).unwrap();
        let y = after_x
            .legal_moves()
            .into_iter()
            .find(|&mv| {
                let mut next = after_x.clone();
                next.play(mv).unwrap() == GameStatus::Ongoing
            })
            .unwrap();
        let mut output = Vec::new();
        let started = std::time::Instant::now();
        let prediction = validated_ponder_move(engine.game(), x, &[x, y]);
        write_bestmove(
            &mut output,
            engine.game().position(),
            x,
            prediction.as_deref(),
            StopReason::DepthCompleted,
            false,
        )
        .unwrap();
        let output_time = started.elapsed();
        assert!(prediction.is_some());
        assert!(String::from_utf8(output).unwrap().contains(" ponder "));

        let previous = engine.before_last_move().unwrap();
        let replacement = previous
            .legal_moves()
            .into_iter()
            .find(|&mv| {
                let text = usi::text_generated(previous.position(), mv);
                if Some(&text)
                    == protocol
                        .accepted_position
                        .as_ref()
                        .unwrap()
                        .move_tokens
                        .last()
                {
                    return false;
                }
                let mut next = previous.clone();
                next.play(mv).unwrap() == GameStatus::Ongoing
            })
            .unwrap();
        let text = usi::text_generated(previous.position(), replacement);
        let mut tokens = protocol
            .accepted_position
            .as_ref()
            .unwrap()
            .move_tokens
            .clone();
        *tokens.last_mut().unwrap() = text;
        let input = format!("position startpos moves {}", tokens.join(" "));
        let mut expected = previous.clone();
        expected.play(replacement).unwrap();
        let mut output = Vec::new();
        let started = std::time::Instant::now();
        protocol
            .handle_idle_line(&mut engine, &input, &mut output)
            .unwrap();
        let replacement_time = started.elapsed();
        assert!(output.is_empty());
        assert_eq!(engine.game().position(), expected.position());
        assert_eq!(engine.ply(), 4000);
        eprintln!(
            "4000 plies: bestmove including prediction {:.3} ms; last-move position {:.3} ms",
            output_time.as_secs_f64() * 1000.0,
            replacement_time.as_secs_f64() * 1000.0
        );
        assert!(output_time <= Duration::from_millis(3), "{output_time:?}");
        assert!(
            replacement_time <= Duration::from_millis(3),
            "{replacement_time:?}"
        );
    }
}
