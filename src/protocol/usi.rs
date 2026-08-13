//! Universal Shogi Interfaceのアダプター。

use std::collections::VecDeque;
use std::io::{self, BufRead, Write};
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
            rules,
            history_keys: game.search_key_history().to_vec(),
            root_moves,
        };
        let search_id = self.next_search_id;
        self.next_search_id = self.next_search_id.wrapping_add(1);
        let transposition_table = self.transposition_table.take().unwrap_or_default();
        let infinite = config.infinite;
        let handle = search::start_search(snapshot, config, search_id, transposition_table);
        Ok(Some(ActiveSearch::Running {
            context: SearchContext {
                id: search_id,
                position,
                rules,
                infinite,
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
        let Some(ActiveSearch::Running { context, .. }) = active.as_ref() else {
            return Ok(());
        };
        if event.search_id() != context.id {
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
            } => write_info(output, context, depth, score, nodes, elapsed, &pv),
            SearchEvent::Finished { best_move, .. } => {
                let Some(ActiveSearch::Running { context, handle }) = active.take() else {
                    unreachable!();
                };
                self.transposition_table = Some(join_search(handle)?);
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
            ActiveSearch::Running { context, handle } => {
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
                        } => write_info(output, &context, depth, score, nodes, elapsed, &pv)?,
                        SearchEvent::Finished { best_move, .. } => break best_move,
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
        writeln!(output, "usiok")
    }

    /// `setoption`を処理する。RuleSet・USI_Variant・USI_Hashを受理し、
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
                match parse_sfen_fields(&tokens[1..end], rules) {
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
/// 第5欄はP1成り権保留欄にも`moves`なしで続く余分な語にもなり得るため、
/// 5欄解析が失敗した場合は、第5欄が成り権保留欄の形をしているときだけ
/// そのエラーを報告し、そうでなければ4欄解析の結果を採用する。
fn parse_sfen_fields(
    fields: &[&str],
    rules: crate::Rules,
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

/// 欄がP1成り権保留欄の形(`-`、コンマ区切り、または数字始まり)かを返す。
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
        let _ = position.make_move_unchecked(mv, context.rules);
    }
    writeln!(output)?;
    output.flush()
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
    let mut game = Game::from_position(rules, position).map_err(|error| error.to_string())?;
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
    use crate::core::piece::Color;
    use crate::core::rules::RuleCode;
    use crate::protocol::engine::EngineLifecycle;
    use crate::{GameResult, GameStatus, Rules, WinReason};

    const INITIAL_SFEN: &str = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b - 1";

    fn engine(codes: &[RuleCode]) -> Engine {
        Engine::new(codes.to_vec()).unwrap()
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
    fn handshake_and_readiness_match_the_complete_transcript() {
        let startup = [RuleCode::E2, RuleCode::R1, RuleCode::L1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(&mut protocol, &mut engine, "usi\nisready\nquit\n"),
            concat!(
                "id name minase ",
                env!("CARGO_PKG_VERSION"),
                "\n",
                "id author stepney141\n",
                "option name RuleSet type string default L1,R1,E2\n",
                "option name USI_Variant type string default chushogi\n",
                "option name USI_Hash type spin default 256\n",
                "usiok\n",
                "readyok\n",
            )
        );
    }

    #[test]
    fn ruleset_accepts_mixed_case_and_invalid_values_preserve_pending() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = concat!(
            "setoption name RuleSet value p3,r2,e2\n",
            "setoption name RuleSet value R2,ZZ\n",
            "setoption name RuleSet value R2,R2\n",
            "setoption name RuleSet value P3,E2\n",
            "quit\n",
        );

        assert_eq!(
            run(&mut protocol, &mut engine, input),
            concat!(
                "info string error: unknown rule code 'ZZ'\n",
                "info string error: duplicate rule code: R2\n",
                "info string error: missing repetition rule\n",
            )
        );
        assert_eq!(
            engine.pending_rule_codes(),
            &[RuleCode::P3, RuleCode::R2, RuleCode::E2]
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R1]);
    }

    #[test]
    fn ruleset_preset_is_accepted_and_combination_error_preserves_pending() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = concat!(
            "setoption name RuleSet value lishogi\n",
            "setoption name RuleSet value lishogi,P1\n",
            "quit\n",
        );

        assert_eq!(
            run(&mut protocol, &mut engine, input),
            "info string error: rule set preset 'lishogi' must be specified alone\n"
        );
        assert_eq!(
            engine.pending_rule_codes(),
            &[
                RuleCode::L1,
                RuleCode::L2,
                RuleCode::P3,
                RuleCode::R1,
                RuleCode::E1,
                RuleCode::E3,
            ]
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R1]);
    }

    #[test]
    fn variant_accepts_chushogi_case_insensitively_and_rejects_other_values() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                concat!(
                    "setoption name USI_Variant value CHUSHOGI\n",
                    "setoption name USI_Variant value shogi\n",
                    "quit\n",
                )
            ),
            "info string error: unsupported USI_Variant value 'shogi'\n"
        );
    }

    #[test]
    fn engine_default_preset_sets_r1_as_pending_rules() {
        let startup = [RuleCode::R2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "setoption name RuleSet value engine-default\nquit\n",
            ),
            ""
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R2]);
        assert_eq!(engine.pending_rule_codes(), &[RuleCode::R1]);
    }

    #[test]
    fn startpos_script_applies_two_stage_jitto_promotion_and_igui_moves() {
        let startup = [RuleCode::R1, RuleCode::E2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        // 20手目は2段階移動、26手目はじっと、31手目は成り、36手目は居喰い。
        let moves = "6i6h 6c7e 10l11k 4a4b 10i10h 7e5e 6h6g 7c6c 7j5h 5e5f 9j11h 8c7c 7i7h 10d10e 6j11e 9c11e 5h4g 8b9c 1i1h 5f6g7h 8j6j 7h8i7i 4g3g 9c10d 11h9f 7i8j7i 9f9g 7i8k 3g2g 8k7k8j 9g6d+ 5a4a 3l2k 8j10l 9h9g 10l10k10l";

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("position startpos moves {moves}\nquit\n")
            ),
            ""
        );
        assert_eq!(engine.lifecycle(), EngineLifecycle::InGame);
        assert_eq!(engine.game().ply_count(), 36);
        assert_eq!(engine.status(), GameStatus::Ongoing);
    }

    #[test]
    fn moves_matches_the_legal_move_notation_set() {
        let startup = [RuleCode::R1, RuleCode::E2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        let output = run(&mut protocol, &mut engine, "position startpos\nmoves\n");
        let mut fields = output.split_whitespace();
        assert_eq!(fields.next(), Some("moves"));
        let actual: HashSet<_> = fields.map(str::to_owned).collect();
        let expected: HashSet<_> = engine
            .game()
            .legal_moves()
            .into_iter()
            .map(|mv| usi::text(engine.game().position(), mv))
            .collect();

        assert_eq!(actual, expected);
    }

    #[test]
    fn queries_report_explicit_errors_before_the_game_starts() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(&mut protocol, &mut engine, "moves\nstate\nquit\n"),
            concat!(
                "info string error: moves requires an active game\n",
                "info string error: state requires an active or finished game\n",
            )
        );
    }

    #[test]
    fn state_reports_exact_ongoing_and_finished_lines() {
        let startup = [RuleCode::E2, RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(&mut protocol, &mut engine, "position startpos\nstate\n"),
            concat!(
                "state rules R1,E2 board ",
                "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/",
                "3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/",
                "A1B1TOXT1B1A/LFCSGKEGSCFL b status ongoing\n",
            )
        );

        let finished = concat!(
            "position sfen 12/12/12/5k6/12/12/5R6/12/12/12/12/K11 b - 1 ",
            "moves 7g7d\nstate\n",
        );
        assert_eq!(
            run(&mut protocol, &mut engine, finished),
            concat!(
                "state rules R1,E2 board ",
                "12/12/12/5R6/12/12/12/12/12/12/12/K11 w ",
                "status win black royal-capture\n",
            )
        );
    }

    #[test]
    fn state_uses_the_next_games_active_rules_after_gameover() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = concat!(
            "position startpos\n",
            "gameover draw\n",
            "state\n",
            "setoption name RuleSet value E2,R2,L1\n",
            "position startpos\n",
            "state\n",
        );

        assert_eq!(
            run(&mut protocol, &mut engine, input),
            concat!(
                "info string error: state requires an active or finished game\n",
                "state rules L1,R2,E2 board ",
                "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/",
                "3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/",
                "A1B1TOXT1B1A/LFCSGKEGSCFL b status ongoing\n",
            )
        );
    }

    #[test]
    fn state_status_uses_the_contract_vocabulary() {
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
                reason: WinReason::Resignation,
            })),
            Err("state cannot represent resignation")
        );
        assert_eq!(
            state_status_text(GameStatus::Finished(GameResult::Draw {
                reason: DrawReason::Agreement,
            })),
            Err("state cannot represent agreement")
        );
    }

    #[test]
    fn position_accepts_four_and_five_field_sfen_and_named_lion_capture_square() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let five_fields = "12/12/7S4/12/12/12/12/12/12/4s7/12/12 b - 17 8j,5c";
        let named_capture = "12/12/12/12/12/12/12/12/12/12/12/12 b 7f 9999";
        let input = format!(
            "position sfen {INITIAL_SFEN}\ngameover draw\nsetoption name RuleSet value P1,R1\nposition sfen {five_fields}\ngameover win\nposition sfen {named_capture}\nquit\n"
        );

        assert_eq!(run(&mut protocol, &mut engine, &input), "");
        assert_eq!(engine.lifecycle(), EngineLifecycle::InGame);
        assert_eq!(
            engine
                .game()
                .position()
                .lion_taken_by_non_lion()
                .map(|trigger| trigger.square),
            Some(crate::test_util::sq(5, 6))
        );

        // 5欄局面自体も独立にcommitし、P1保留集合を検査する。
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("gameover lose\nposition sfen {five_fields}\n")
            ),
            ""
        );
        let deferred = engine.game().position().promotion_deferred();
        assert!(deferred.contains(crate::test_util::sq(4, 2)));
        assert!(deferred.contains(crate::test_util::sq(7, 9)));
    }

    #[test]
    fn illegal_move_in_position_rejects_the_whole_replacement_as_repetition() {
        let startup = [RuleCode::R2, RuleCode::E2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let base = "12/12/12/8k3/12/12/12/12/3K8/12/12/12 b - 1";
        let first_three = "9i9h 4d4e 9h9i";
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("position sfen {base} moves {first_three}\n")
            ),
            ""
        );
        let position_before = engine.game().position().clone();

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("position sfen {base} moves {first_three} 4e4d\nquit\n")
            ),
            "info string error: illegal move '4e4d': forbidden repetition\n"
        );
        assert_eq!(engine.game().position(), &position_before);
        assert_eq!(engine.game().ply_count(), 3);
        assert_eq!(engine.status(), GameStatus::Ongoing);
    }

    #[test]
    fn go_depth_one_returns_a_legal_move_without_applying_it() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let position_before = engine.game().position().clone();

        let output = run(
            &mut protocol,
            &mut engine,
            "position startpos\ngo depth 1\n",
        );
        let lines: Vec<_> = output.lines().collect();
        assert!(lines[0].starts_with("info depth 1 score "));
        let move_text = lines[1]
            .strip_prefix("bestmove ")
            .expect("go must finish with a bestmove line");
        let best_move = usi::parse(engine.game().position(), move_text).unwrap();

        assert!(engine.game().legal_moves().contains(&best_move));
        assert_eq!(engine.game().position(), &position_before);
        assert_eq!(engine.game().ply_count(), 0);
    }

    #[test]
    fn go_rejects_inactive_and_finished_games_without_bestmove() {
        let startup = [RuleCode::R1, RuleCode::E2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = concat!(
            "go depth 1\n",
            "position sfen 12/12/12/12/12/12/12/12/12/12/12/12 b - 1\n",
            "go depth 1\n",
            "position sfen 12/12/12/5k6/12/12/5R6/12/12/12/12/K11 b - 1 ",
            "moves 7g7d\n",
            "go nodes 1\n",
            "quit\n",
        );

        assert_eq!(
            run(&mut protocol, &mut engine, input),
            concat!(
                "info string error: go requires an active game\n",
                "info string error: go requires at least one legal move\n",
                "info string error: go requires an active game\n",
            )
        );
    }

    #[test]
    fn go_rejects_missing_unsupported_and_invalid_limits() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = concat!(
            "position startpos\n",
            "go\n",
            "go depth 0\n",
            "go nodes nope\n",
            "go depth 257\n",
            "quit\n",
        );

        assert_eq!(
            run(&mut protocol, &mut engine, input),
            concat!(
                "info string error: go requires depth or nodes\n",
                "info string error: go depth must be a positive integer\n",
                "info string error: go nodes must be a positive integer\n",
                "info string error: go depth must not exceed 256\n",
            )
        );
    }

    #[test]
    fn go_accepts_nodes_and_combined_limits_and_keeps_go_mate_behavior() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let nodes = run(
            &mut protocol,
            &mut engine,
            "position startpos\ngo nodes 1\n",
        );
        let combined = run(&mut protocol, &mut engine, "go depth 1 nodes 1\n");
        let mate = run(&mut protocol, &mut engine, "go ignored mate 1000\n");

        assert!(nodes.starts_with("bestmove "));
        assert!(combined.starts_with("bestmove "));
        assert_eq!(mate, "checkmate notimplemented\n");
    }

    #[test]
    fn go_time_arguments_are_normalized_for_the_side_to_move() {
        let tokens = [
            "btime", "1000", "wtime", "2000", "binc", "30", "winc", "40", "byoyomi", "500",
            "movetime", "600", "depth", "7", "nodes", "800", "infinite",
        ];

        assert_eq!(
            parse_go_config(&tokens, Color::Black),
            Ok(SearchLimits {
                depth: Some(7),
                nodes: Some(800),
                movetime_ms: Some(600),
                clock: Some(ClockLimits {
                    remaining_ms: 1000,
                    increment_ms: 30,
                    byoyomi_ms: 500,
                }),
                infinite: true,
            })
        );
        assert_eq!(
            parse_go_config(&tokens, Color::White),
            Ok(SearchLimits {
                depth: Some(7),
                nodes: Some(800),
                movetime_ms: Some(600),
                clock: Some(ClockLimits {
                    remaining_ms: 2000,
                    increment_ms: 40,
                    byoyomi_ms: 500,
                }),
                infinite: true,
            })
        );
    }

    #[test]
    fn go_time_arguments_reject_duplicates_and_invalid_values() {
        assert_eq!(
            parse_go_config(&["movetime", "1", "movetime", "2"], Color::Black),
            Err("go movetime must be specified once".to_owned())
        );
        assert_eq!(
            parse_go_config(&["btime", "-1"], Color::Black),
            Err("go btime must be a non-negative integer".to_owned())
        );
        assert_eq!(
            parse_go_config(&["infinite", "infinite"], Color::Black),
            Err("go infinite must be specified once".to_owned())
        );
    }

    #[test]
    fn lishogi_bot_series_rebuilds_the_full_history_without_usinewgame() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        let first = run(
            &mut protocol,
            &mut engine,
            concat!(
                "setoption name RuleSet value lishogi\n",
                "position startpos\n",
                "go btime 60000 wtime 60000 byoyomi 10000 nodes 1\n",
            ),
        );
        assert!(first.starts_with("bestmove "));
        assert_eq!(engine.game().ply_count(), 0);

        let first_text = first
            .trim_end()
            .strip_prefix("bestmove ")
            .expect("the fixed-node search must return one bestmove");
        let mut server_game = engine.game().clone();
        let first_move = usi::parse(server_game.position(), first_text).unwrap();
        server_game.play(first_move).unwrap();
        let reply_move = server_game.legal_moves()[0];
        let reply_text = usi::text(server_game.position(), reply_move);

        let second = run(
            &mut protocol,
            &mut engine,
            &format!(
                "position startpos moves {first_text} {reply_text}\ngo btime 59900 wtime 59800 byoyomi 10000 nodes 1\n"
            ),
        );
        assert!(second.starts_with("bestmove "));
        assert_eq!(engine.game().ply_count(), 2);
        assert_eq!(
            engine.active_rule_codes(),
            &[
                RuleCode::L1,
                RuleCode::L2,
                RuleCode::P3,
                RuleCode::R1,
                RuleCode::E1,
                RuleCode::E3,
            ]
        );
    }

    #[test]
    fn failed_position_blocks_go_until_a_valid_position_recovers_sync() {
        let startup = [RuleCode::R1, RuleCode::E2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let output = run(
            &mut protocol,
            &mut engine,
            concat!(
                "position startpos\n",
                "position startpos moves 1a1b\n",
                "go nodes 1\n",
                "position startpos moves 6i6h\n",
                "go nodes 1\n",
            ),
        );
        let lines: Vec<_> = output.lines().collect();

        assert_eq!(
            lines[0],
            "info string error: illegal move '1a1b': illegal movement"
        );
        assert_eq!(
            lines[1],
            "info string error: go requires a synchronized position"
        );
        assert!(lines[2].starts_with("bestmove "));
        assert_eq!(engine.game().ply_count(), 1);
    }

    #[test]
    fn ponder_commands_return_only_error_information() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "position startpos\ngo ponder btime 1 wtime 1\nponderhit\n",
            ),
            concat!(
                "info string error: go ponder is not supported\n",
                "info string error: ponderhit is not supported\n",
            )
        );
    }

    #[test]
    fn duplicate_go_is_rejected_and_infinite_search_stops_with_one_bestmove() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let output = run(
            &mut protocol,
            &mut engine,
            "position startpos\ngo infinite\ngo depth 1\nstop\n",
        );
        let lines: Vec<_> = output.lines().collect();

        assert!(lines.contains(&"info string error: go is already running"));
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("bestmove "))
                .count(),
            1
        );
        assert!(lines.iter().all(|line| {
            line.starts_with("info depth ")
                || line.starts_with("info string error: ")
                || line.starts_with("bestmove ")
        }));
    }

    #[test]
    fn open_command_channel_reports_progress_and_accepts_stop() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
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
    fn sequential_protocol_defers_position_without_blocking_later_stop() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let mut input = std::io::Cursor::new(
            b"position startpos\ngo infinite\nposition startpos moves 6i6h\nstop\n",
        );
        let mut output = Vec::new();

        protocol.run(&mut engine, &mut input, &mut output).unwrap();

        assert!(String::from_utf8(output).unwrap().contains("bestmove "));
        assert_eq!(engine.game().ply_count(), 1);
    }

    #[test]
    fn gameover_and_quit_discard_running_search_results() {
        let startup = [RuleCode::R1];
        let mut gameover_engine = engine(&startup);
        let mut gameover_protocol = UsiProtocol::new(&gameover_engine);
        let gameover_output = run(
            &mut gameover_protocol,
            &mut gameover_engine,
            "position startpos\ngo infinite\ngameover draw\n",
        );
        assert!(!gameover_output.contains("bestmove "));
        assert_eq!(gameover_engine.lifecycle(), EngineLifecycle::AwaitingStart);

        let mut quit_engine = engine(&startup);
        let mut quit_protocol = UsiProtocol::new(&quit_engine);
        let quit_output = run(
            &mut quit_protocol,
            &mut quit_engine,
            "position startpos\ngo infinite\nquit\n",
        );
        assert!(!quit_output.contains("bestmove "));
    }

    #[test]
    fn commands_queued_during_search_apply_after_bestmove() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let output = run(
            &mut protocol,
            &mut engine,
            "position startpos\ngo depth 1\nposition startpos moves 6i6h\nisready\n",
        );
        let lines: Vec<_> = output.lines().collect();
        let bestmove_index = lines
            .iter()
            .position(|line| line.starts_with("bestmove "))
            .unwrap();
        let ready_index = lines.iter().position(|line| *line == "readyok").unwrap();

        assert!(bestmove_index < ready_index);
        assert_eq!(engine.game().ply_count(), 1);
    }

    #[test]
    fn usi_hash_accepts_positive_megabyte_values() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                concat!(
                    "setoption name USI_Hash value 1\n",
                    "setoption name USI_Hash value 2\n",
                    "setoption name USI_Hash value 0\n",
                ),
            ),
            "info string error: USI_Hash must be a positive integer\n"
        );
    }

    #[test]
    fn score_and_nps_formatting_use_search_contract_units() {
        assert_eq!(score_text(42), ("cp", 42));
        assert_eq!(score_text(search::MATE - 3), ("mate", 3));
        assert_eq!(score_text(-search::MATE + 4), ("mate", -4));
        assert_eq!(nodes_per_second(500, Duration::from_millis(250)), 2000);
        assert_eq!(nodes_per_second(500, Duration::ZERO), 0);
    }

    #[test]
    fn rules_latch_changes_only_the_next_game_for_position_and_usinewgame() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "position startpos\nsetoption name RuleSet value R2,E2\n"
            ),
            ""
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R1]);
        assert_eq!(engine.pending_rule_codes(), &[RuleCode::R2, RuleCode::E2]);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "position startpos\ngameover draw\nposition startpos\n"
            ),
            ""
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R2, RuleCode::E2]);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                concat!(
                    "gameover lose\n",
                    "setoption name RuleSet value R1,E2\n",
                    "usinewgame\n",
                    "position startpos\n",
                    "quit\n",
                )
            ),
            ""
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R1, RuleCode::E2]);
        assert_eq!(engine.lifecycle(), EngineLifecycle::InGame);
    }

    #[test]
    fn royal_capture_finishes_engine_without_usi_result_output() {
        let startup = [RuleCode::R1, RuleCode::E2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = "position sfen 12/12/12/5k6/12/12/5R6/12/12/12/12/K11 b - 1 moves 7g7d\nquit\n";

        assert_eq!(run(&mut protocol, &mut engine, input), "");
        assert_eq!(
            engine.status(),
            GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::RoyalCapture,
            })
        );
        assert_eq!(engine.lifecycle(), EngineLifecycle::Finished);
    }

    #[test]
    fn movement_error_in_the_middle_preserves_the_previous_position() {
        let startup = [RuleCode::R1, RuleCode::E2];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        assert_eq!(run(&mut protocol, &mut engine, "position startpos\n"), "");
        let position_before = engine.game().position().clone();

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "position startpos moves 6i6h 1a1b 6h6g\nquit\n"
            ),
            "info string error: illegal move '1a1b': illegal movement\n"
        );
        assert_eq!(engine.game().position(), &position_before);
        assert_eq!(engine.game().ply_count(), 0);
    }

    #[test]
    fn unknown_commands_options_and_tokens_are_ignored() {
        let startup = [RuleCode::R1];
        let mut engine = engine(&startup);
        let mut protocol = UsiProtocol::new(&engine);
        let input = format!(
            "unknown command\nsetoption name Unknown value anything\nposition startpos ignored tokens\nposition sfen {INITIAL_SFEN} - ignored tokens\nquit\n"
        );

        assert_eq!(run(&mut protocol, &mut engine, &input), "");
        assert_eq!(engine.game().rules(), Rules::from_codes(&startup).unwrap());
        assert_eq!(engine.lifecycle(), EngineLifecycle::InGame);
    }
}
