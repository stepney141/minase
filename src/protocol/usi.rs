//! Universal Shogi Interfaceのアダプター。

use std::collections::VecDeque;
use std::io::{self, BufRead, Write};
use std::num::NonZeroUsize;
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
    self, ClockLimits, SearchEvent, SearchHandle, SearchLimits, SearchSnapshot, TranspositionTable,
};

use super::Protocol;
use super::engine::{
    Engine, EngineCommand, EngineLifecycle, EngineReply, RejectReason, canonical_rules_text,
};

/// lishogi系拡張を含むUSIプロトコル。
pub struct UsiProtocol {
    /// `option`宣言のdefault値に使う起動時規則の正準表記。
    startup_rules_text: String,
    /// 探索間で引き継ぐ置換表。探索スレッドへ貸し出している間は`None`。
    transposition_table: Option<TranspositionTable>,
    /// 最後の`position`コマンドが受理され、局面が同期済みかどうか。
    position_synchronized: bool,
    /// 次に開始する探索へ割り当てる識別子。
    next_search_id: u64,
    /// 次の探索に使うワーカー数。
    threads: NonZeroUsize,
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
    /// `go infinite`の探索が完了し、`stop`を待って`bestmove`を返す状態。
    AwaitingStop {
        /// 探索の局面と設定。
        context: SearchContext,
        /// `stop`受信時に返す最善手。
        best_move: Move,
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
            next_search_id: 1,
            threads: search::DEFAULT_THREADS,
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

        match command {
            "usi" => self.write_handshake(output)?,
            "isready" => writeln!(output, "readyok")?,
            "setoption" => self.handle_setoption(engine, &tokens[1..], output)?,
            "usinewgame" => {
                self.position_synchronized = false;
                self.apply_silent(engine, EngineCommand::NewGame, output)?;
            }
            "position" => self.handle_position(engine, &tokens[1..], output)?,
            "gameover" => {
                self.position_synchronized = false;
                self.apply_silent(engine, EngineCommand::EndGame, output)?;
            }
            "moves" => self.handle_moves(engine, output)?,
            "state" => self.handle_state(engine, output)?,
            "ponderhit" => write_error(output, "ponderhit is not supported")?,
            "go" if tokens[1..].contains(&"ponder") => {
                write_error(output, "go ponder is not supported")?;
            }
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
        let config = match parse_go_config(tokens, game.position().side_to_move()) {
            Ok(config) => config,
            Err(error) => {
                write_error(output, &error)?;
                return Ok(None);
            }
        };
        let root_moves = game.legal_moves();
        if root_moves.is_empty() {
            write_error(output, "go requires at least one legal move")?;
            return Ok(None);
        }
        let position = game.position().clone();
        let rules = engine.active_rules();
        let snapshot = SearchSnapshot {
            position: position.clone(),
            rules: rules.moves,
            history_keys: game.search_key_history().to_vec(),
            root_moves,
        };
        let search_id = self.next_search_id;
        self.next_search_id = self.next_search_id.wrapping_add(1);
        let transposition_table = self.transposition_table.take().unwrap_or_default();
        let infinite = config.infinite;
        let handle = search::start_search(
            snapshot,
            config,
            search_id,
            self.threads,
            transposition_table,
        );
        Ok(Some(ActiveSearch::Running {
            context: SearchContext {
                id: search_id,
                position,
                rules,
                infinite,
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

            self.poll_search(&mut active, output)?;
            if active.is_none() {
                continue;
            }

            // 入力もイベントもない間は、短い待ちで両者を交互に見張る。
            if input_open {
                match input.recv_timeout(Duration::from_millis(10)) {
                    Ok(Ok(line)) => self.handle_searching_line(
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
            } else if active.as_ref().is_some_and(ActiveSearch::is_infinite) {
                // 入力が閉じると`stop`は届かないため、無限探索は破棄する。
                self.discard_search(&mut active)?;
            } else {
                self.wait_search_event(&mut active, output)?;
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
        active: &mut Option<ActiveSearch>,
        pending: &mut VecDeque<String>,
        line: &str,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let command = line.split_whitespace().next();
        match command {
            Some("stop") => self.stop_search(active, output)?,
            Some("gameover" | "quit") => {
                pending.push_back(line.to_owned());
                self.discard_search(active)?;
            }
            Some("go") => write_error(output, "go is already running")?,
            Some("ponderhit") => write_error(output, "ponderhit is not supported")?,
            _ => pending.push_back(line.to_owned()),
        }
        output.flush()
    }

    /// 溜まっている探索イベントをブロックせずにすべて処理する。
    fn poll_search(
        &mut self,
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
            self.handle_search_event(active, event, output)?;
        }
    }

    /// 探索イベントを短時間だけ待って処理する。入力が閉じた後の待機に使う。
    fn wait_search_event(
        &mut self,
        active: &mut Option<ActiveSearch>,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        match active.as_ref() {
            Some(ActiveSearch::Running { handle, .. }) => {
                match handle.events().recv_timeout(Duration::from_millis(50)) {
                    Ok(event) => self.handle_search_event(active, event, output)?,
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
                if context.infinite {
                    *active = Some(ActiveSearch::AwaitingStop { context, best_move });
                    Ok(())
                } else {
                    write_bestmove(output, &context.position, best_move)
                }
            }
        }
    }

    /// `stop`に応じて探索を打ち切り、`bestmove`を返す。
    fn stop_search(
        &mut self,
        active: &mut Option<ActiveSearch>,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        self.finish_search(active, output, true)
    }

    /// 探索の完了を待ち切って`bestmove`を出力する。
    ///
    /// 完了までの進捗イベントも順に`info`行として出力する。
    fn finish_search(
        &mut self,
        active: &mut Option<ActiveSearch>,
        output: &mut dyn Write,
        request_stop: bool,
    ) -> io::Result<()> {
        let Some(search) = active.take() else {
            return Ok(());
        };
        match search {
            ActiveSearch::AwaitingStop { context, best_move } => {
                write_bestmove(output, &context.position, best_move)
            }
            ActiveSearch::Running {
                mut context,
                handle,
            } => {
                if request_stop {
                    handle.request_stop();
                }
                let best_move = loop {
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
                            break best_move;
                        }
                    }
                };
                self.transposition_table = Some(join_search(handle)?);
                write_bestmove(output, &context.position, best_move)
            }
        }
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
            "option name USI_Hash type spin default {}",
            search::DEFAULT_TT_SIZE_MB
        )?;
        writeln!(
            output,
            "option name Threads type spin default {} min 1 max 256",
            search::DEFAULT_THREADS
        )?;
        writeln!(output, "usiok")
    }

    /// `setoption`を処理する。RuleSet・USI_Variant・USI_Hash・Threadsを受理し、
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

        if name.eq_ignore_ascii_case("RuleSet") {
            let Some(value) = value else {
                return write_error(output, "RuleSet requires a value");
            };
            let codes = match parse_rule_set(value) {
                Ok(codes) => codes,
                Err(error) => return write_error(output, &error),
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
            let Some(size_mb) = value.parse::<usize>().ok().filter(|&size| size > 0) else {
                return write_error(output, "USI_Hash must be a positive integer");
            };
            match &mut self.transposition_table {
                Some(transposition_table) => transposition_table.resize(size_mb),
                None => self.transposition_table = Some(TranspositionTable::new(size_mb)),
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
            return write_error(output, "position requires startpos or sfen");
        };
        let moves_index = tokens.iter().position(|token| *token == "moves");
        let move_tokens = moves_index.map(|index| &tokens[index + 1..]).unwrap_or(&[]);
        let rules = engine.position_rules();
        let setup = match kind {
            "startpos" => SetupPosition {
                position: Position::initial(),
                lion_capture: None,
                next_move_number: 1,
            },
            "sfen" => {
                let end = moves_index.unwrap_or(tokens.len());
                match parse_sfen_fields(&tokens[1..end], rules.moves) {
                    Ok(setup) => setup,
                    Err(error) => return write_error(output, &error.to_string()),
                }
            }
            _ => return Ok(()),
        };

        let parsed = match parse_moves(&setup, rules, move_tokens) {
            Ok(parsed) => parsed,
            Err(error) => return write_error(output, &error),
        };
        match engine.handle(EngineCommand::SetPosition {
            setup,
            moves: parsed.moves,
        }) {
            EngineReply::Accepted { .. } => {
                self.position_synchronized = true;
                Ok(())
            }
            EngineReply::Rejected(reason) => write_error(
                output,
                &position_reject_reason_text(&reason, parsed.first_rejected_text.as_deref()),
            ),
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
            write!(output, " {}", usi::text(game.position(), mv))?;
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
        match engine.handle(command) {
            EngineReply::Accepted { .. } => {
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

/// `go`の引数列を探索制限へ変換する。
///
/// `depth`・`nodes`・`movetime`・時計引数(`btime`等)・`infinite`を受理し、
/// 時計引数からは手番側の残り時間だけを取り出す。
fn parse_go_config(tokens: &[&str], side_to_move: Color) -> Result<SearchLimits, String> {
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
    let clock = clock_specified.then(|| {
        let (remaining_ms, increment_ms) = match side_to_move {
            Color::Black => (btime.unwrap_or(0), binc.unwrap_or(0)),
            Color::White => (wtime.unwrap_or(0), winc.unwrap_or(0)),
        };
        ClockLimits {
            remaining_ms,
            increment_ms,
            byoyomi_ms: byoyomi.unwrap_or(0),
        }
    });

    Ok(SearchLimits {
        depth,
        nodes,
        movetime_ms,
        clock,
        infinite,
    })
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
///
/// 第5欄は成り権保留欄(P1・P2・P5)にも`moves`なしで続く余分な語にもなり得るため、
/// 5欄解析が失敗した場合は、第5欄が成り権保留欄の形をしているときだけ
/// そのエラーを報告し、そうでなければ4欄解析の結果を採用する。
fn parse_sfen_fields(
    fields: &[&str],
    rules: crate::MoveRules,
) -> Result<SetupPosition, crate::notation::sfen::SfenError> {
    let four_field_text = fields.iter().take(4).copied().collect::<Vec<_>>().join(" ");
    let four_field_setup = parse_extended_sfen(&four_field_text, rules)?;
    let Some(fifth) = fields.get(4) else {
        return Ok(four_field_setup);
    };

    let five_field_text = fields.iter().take(5).copied().collect::<Vec<_>>().join(" ");
    match parse_extended_sfen(&five_field_text, rules) {
        Ok(setup) => Ok(setup),
        Err(error) if looks_like_promotion_deferred(fifth) => Err(error),
        Err(_) => Ok(four_field_setup),
    }
}

/// 欄が成り権保留欄の形(`-`、コンマ区切り、または数字始まり)かを返す。
fn looks_like_promotion_deferred(field: &str) -> bool {
    field == "-" || field.contains(',') || field.as_bytes().first().is_some_and(u8::is_ascii_digit)
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
                self.handle_searching_line(&mut active, &mut pending, &command, output)?;
                self.poll_search(&mut active, output)?;
                continue;
            }

            match self.handle_idle_line(engine, &command, output)? {
                LineAction::Continue => {}
                LineAction::Start(search) => {
                    active = Some(*search);
                    if !active.as_ref().is_some_and(ActiveSearch::is_infinite) {
                        self.finish_search(&mut active, output, false)?;
                    }
                }
                LineAction::Quit => return Ok(()),
            }
        }
        self.discard_search(&mut active)
    }
}

impl ActiveSearch {
    /// `go infinite`による探索かどうかを返す。
    fn is_infinite(&self) -> bool {
        match self {
            Self::Running { context, .. } | Self::AwaitingStop { context, .. } => context.infinite,
        }
    }
}

/// 探索スレッドの終了を待ち、貸し出していた置換表を回収する。
fn join_search(handle: SearchHandle) -> io::Result<TranspositionTable> {
    handle
        .join()
        .map_err(|_| io::Error::other("search thread panicked"))
}

/// `bestmove`行を出力する。
fn write_bestmove(output: &mut dyn Write, position: &Position, mv: Move) -> io::Result<()> {
    writeln!(output, "bestmove {}", usi::text(position, mv))?;
    output.flush()
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
    let (score_kind, score_value) = score_text(score);
    write!(
        output,
        "info depth {depth} score {score_kind} {score_value} nodes {nodes} nps {} pv",
        nodes_per_second(nodes, elapsed)
    )?;
    let mut position = context.position.clone();
    for &mv in pv {
        write!(output, " {}", usi::text(&position, mv))?;
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

/// 評価値を`info score`の種別(`cp`または`mate`)と値へ変換する。
fn score_text(score: i32) -> (&'static str, i32) {
    if score >= search::MATE_THRESHOLD {
        ("mate", search::MATE - score)
    } else if score <= -search::MATE_THRESHOLD {
        ("mate", -(search::MATE + score))
    } else {
        ("cp", score)
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
    let mut position = setup.position.clone();
    position
        .set_lion_capture(setup.lion_capture)
        .map_err(|error| error.to_string())?;
    let mut game = Game::from_position(rules, position);
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
        // EC適用範囲: ponder非対応のためUSI_Ponderは宣言しない（D6-USI-18）。
        assert!(!output.contains("USI_Ponder"));
    }

    #[test]
    fn threads_accepts_boundaries_and_rejects_invalid_values() {
        // LS「プロトコル設定」: Threadsは1..=256だけを受理し、不正値には固定エラーを返す
        // （D6-USI-36）。正当値は無応答である。
        let output = session(
            &[RuleCode::R1],
            concat!(
                "setoption name Threads value 1\n",
                "setoption name Threads value 256\n",
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
        assert_eq!(parse_threads("1").unwrap().get(), 1);
        assert_eq!(parse_threads("256").unwrap().get(), 256);
        assert!(parse_threads("0").is_none());
        assert!(parse_threads("257").is_none());
        assert!(parse_threads("nope").is_none());
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
            "position startpos moves 6i6h\nsetoption name RuleSet value L0,P0,R2,E0\nstate\n",
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
        assert_eq!(state_rules(state_lines(&output)[0]), "L0,P0,R2,E0");
    }

    #[test]
    fn pending_rules_commit_with_or_without_usinewgame() {
        // PLコマンドenum: commit点はNewGame受信時とAwaitingStartでのSetPosition受信時（D6-USI-08）。
        let with_newgame = session(
            &[RuleCode::R1],
            concat!(
                "setoption name RuleSet value L0,P0,R2,E2\n",
                "usinewgame\nposition startpos\nstate\n",
            ),
        );
        let without_newgame = session(
            &[RuleCode::R1],
            concat!(
                "position startpos\ngameover win\n",
                "setoption name RuleSet value L0,P0,R2,E2\n",
                "position startpos\nstate\n",
            ),
        );

        // lishogi-bot互換の要: usinewgameの有無は次局規則の反映結果に影響しない。
        assert_eq!(state_rules(state_lines(&with_newgame)[0]), "L0,P0,R2,E2");
        assert_eq!(
            state_lines(&with_newgame).last().unwrap(),
            state_lines(&without_newgame).last().unwrap()
        );
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
    fn failed_position_blocks_go_until_a_valid_position_recovers() {
        // EC「Lishogi-Bot経路」: 局面同期失敗状態では正当なposition受理までgoを拒否する（D6-USI-12）。
        let output = session(
            &[RuleCode::R1],
            concat!(
                "position startpos\n",
                "state\n",
                "position startpos moves 1a1b\n",
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
        let white = session(
            &[RuleCode::R1],
            &format!("position startpos moves 6i6h\n{go}\n"),
        );
        assert_eq!(bestmoves(&black).len(), 1);
        assert_eq!(bestmoves(&white).len(), 1);

        // wire解析と単位正規化はプロトコル側の責務（EC「責務分担」）。手番側の選択を単体で固定する。
        let tokens = [
            "btime", "1000", "wtime", "2000", "binc", "30", "winc", "40", "byoyomi", "500",
        ];
        assert_eq!(
            parse_go_config(&tokens, Color::Black).unwrap().clock,
            Some(ClockLimits {
                remaining_ms: 1000,
                increment_ms: 30,
                byoyomi_ms: 500,
            })
        );
        assert_eq!(
            parse_go_config(&tokens, Color::White).unwrap().clock,
            Some(ClockLimits {
                remaining_ms: 2000,
                increment_ms: 40,
                byoyomi_ms: 500,
            })
        );
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
        let again2 = session(&[RuleCode::R1], "position startpos\ngo depth 1\n");
        assert_eq!(bestmoves(&again), bestmoves(&again2));
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
    fn ponder_commands_are_rejected_without_bestmove() {
        // EC「探索とbestmove」: go ponderとponderhitはエラー情報行のみでbestmoveを返さない（D6-USI-18）。
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
            loop {
                let line = line_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
                if line.starts_with("bestmove ") {
                    break;
                }
            }
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
                || line.starts_with("bestmove ")
        }));
    }

    #[test]
    fn commands_arriving_during_search_apply_after_bestmove() {
        // EC「探索中に届くコマンド」: 停止指示以外は探索のjoin後（bestmove送出後）に適用する（D6-USI-22）。
        let output = session(
            &[RuleCode::R1],
            "position startpos\ngo depth 1\nposition startpos moves 6i6h\nisready\nstate\n",
        );
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
    fn threads_arriving_during_search_applies_before_the_next_search() {
        // LS「プロトコル設定」: 探索中のThreadsはpendingへ積み、実行中探索を変えず、
        // join後に処理して次の探索へ適用する（D6-USI-37）。
        let output = session(
            &[RuleCode::R1],
            concat!(
                "position startpos\n",
                "go infinite\n",
                "setoption name Threads value 2\n",
                "stop\n",
                "position startpos\n",
                "go depth 1\n",
            ),
        );

        assert!(error_lines(&output).is_empty());
        assert_eq!(bestmoves(&output).len(), 2);
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
        let lines: Vec<_> = output.lines().collect();
        let bestmove_positions: Vec<_> = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| line.starts_with("bestmove ").then_some(index))
            .collect();

        assert_eq!(bestmove_positions.len(), 1);
        let bestmove = bestmove_positions[0];
        assert_eq!(bestmove, lines.len() - 1);
        assert!(bestmove > 0);
        assert!(lines[bestmove - 1].starts_with("info depth "));
        assert!(
            lines[..bestmove]
                .iter()
                .all(|line| line.starts_with("info depth "))
        );
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

    #[test]
    fn info_lines_follow_the_contract_token_order() {
        // EC「思考情報」のinfo行形式（D6-USI-26）。行数と数値の値は契約にしない。
        let output = session(&[RuleCode::R1], "position startpos\ngo depth 2\n");
        let lines: Vec<_> = output.lines().collect();

        assert!(lines.last().unwrap().starts_with("bestmove "));
        let info_lines = &lines[..lines.len() - 1];
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
            assert_eq!(tokens[10], "pv");
            assert!(tokens.len() > 11);
            // PVの各手はlishogi系USI指し手構文の文字だけからなる。
            for mv in &tokens[11..] {
                assert!(
                    mv.chars()
                        .all(|c| c.is_ascii_digit() || ('a'..='l').contains(&c) || c == '+')
                );
            }
        }
    }

    #[test]
    fn usi_hash_is_accepted_and_leaves_the_game_unchanged() {
        // EC実施状況フェーズ1: USI_Hash受理と非探索中リサイズの外形無害性（D6-USI-27）。
        // 不正値0の拒否は明文外の実装契約（SU-04）。
        let output = session(
            &[RuleCode::R1],
            concat!(
                "setoption name USI_Hash value 0\n",
                "position startpos\nstate\n",
                "setoption name USI_Hash value 64\n",
                "state\ngo depth 1\n",
            ),
        );
        let states = state_lines(&output);

        assert_eq!(error_lines(&output).len(), 1);
        // リサイズは対局状態を変えない。
        assert_eq!(states[0], states[1]);
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
    fn unknown_commands_and_tokens_are_ignored_and_quit_is_silent() {
        // PL「USIの未知入力は原典準拠」: 未知コマンド行と既知コマンド内の未知トークンは無視（D6-USI-29）。
        let noisy = session(
            &[RuleCode::R1],
            "foobar baz\nposition startpos extra tokens\nstate\nquit\nusi\n",
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
            .map(|mv| usi::text(game.position(), mv))
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
        // BGのstatus語彙表（D6-USI-32）。resignationとagreementは文法に含めず、
        // statusを出さない防御分岐は現行USIに到達経路がないため実装契約として固定する。
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
            assert_eq!(
                state_status_text(GameStatus::Finished(GameResult::Win {
                    winner: Color::Black,
                    reason,
                })),
                Ok(format!("win black {text}"))
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
            state_status_text(GameStatus::Ongoing),
            Ok("ongoing".to_owned())
        );
        assert!(
            state_status_text(GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::Resignation,
            }))
            .is_err()
        );
        assert!(
            state_status_text(GameStatus::Finished(GameResult::Draw {
                reason: DrawReason::Agreement,
            }))
            .is_err()
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
}
