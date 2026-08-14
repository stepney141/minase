//! Chess Engine Communication Protocolのアダプター。

use std::collections::VecDeque;
use std::io::{self, BufRead, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use crate::core::game::{DrawReason, GameResult, IllegalMoveCause, WinReason};
use crate::core::piece::Color;
use crate::core::position::Position;
use crate::core::rules::parse_rule_set;
use crate::notation::cecp;
use crate::notation::sfen::{SetupPosition, parse_extended_sfen};
use crate::search::{
    self, ClockLimits, SearchEvent, SearchHandle, SearchLimits, SearchSnapshot, TranspositionTable,
};

use super::Protocol;
use super::engine::{
    Engine, EngineCommand, EngineLifecycle, EngineReply, RejectReason, canonical_rules_text,
};

/// 中将棋用のCECPアダプター。
pub struct CecpProtocol {
    /// feature宣言のRuleSet既定値に使う起動時規則の正準表記。
    startup_rules_text: String,
    /// CECPがエンジンに担当させている内部手番。
    engine_side: Option<Color>,
    /// `force`により両陣営の着手を受信のみする状態かどうか。
    force_mode: bool,
    /// 探索間で引き継ぐ置換表。探索スレッドへ貸し出し中は`None`。
    transposition_table: Option<TranspositionTable>,
    /// 次に開始する探索へ割り当てる識別子。
    next_search_id: u64,
    /// CECPの時間コマンドをミリ秒へ正規化した値。
    time_control: TimeControl,
}

/// CECPから受け取った探索制限の正規化値。
#[derive(Default)]
struct TimeControl {
    /// `time`が通知したエンジン側の残り時間(ms)。
    engine_remaining_ms: Option<u64>,
    /// `otim`が通知した相手側の残り時間(ms)。
    opponent_remaining_ms: Option<u64>,
    /// `level`が通知した1手ごとの加算時間(ms)。
    increment_ms: u64,
    /// `st`が通知した1手の固定時間(ms)。
    movetime_ms: Option<u64>,
    /// `sd`が通知した最大深さ。
    depth: Option<u32>,
}

/// 実行中の探索に対応する識別子。
struct SearchContext {
    /// 探索イベントの照合に使う探索識別子。
    id: u64,
}

/// 実行中の探索と操作ハンドル。
struct ActiveSearch {
    /// 探索開始時の識別情報。
    context: SearchContext,
    /// 探索スレッドへのハンドル。
    handle: SearchHandle,
}

/// 待機中に1コマンドを処理した後の動作。
enum LineAction {
    /// 待機状態を継続する。
    Continue,
    /// 新しい探索を開始する。
    Start(Box<ActiveSearch>),
    /// セッションを終了する。
    Quit,
}

impl CecpProtocol {
    /// エンジンの起動時active規則をfeature宣言の既定値として保持する。
    pub fn new(engine: &Engine) -> Self {
        Self {
            startup_rules_text: canonical_rules_text(engine.active_rule_codes()),
            engine_side: None,
            force_mode: true,
            transposition_table: None,
            next_search_id: 1,
            time_control: TimeControl::default(),
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
            "xboard" | "accepted" => {}
            "protover" => self.write_features(output)?,
            "rejected" => {
                let feature = tokens.get(1).copied().unwrap_or("");
                if matches!(feature, "setboard" | "usermove" | "ping") {
                    writeln!(
                        output,
                        "tellusererror minase requires the {feature} feature"
                    )?;
                    return Ok(LineAction::Quit);
                }
            }
            "variant" => {
                let name = tokens.get(1).copied().unwrap_or("");
                if name != "chu" {
                    writeln!(output, "Error (unsupported variant): {name}")?;
                }
            }
            "new" => self.handle_new(engine, output)?,
            "force" => {
                self.force_mode = true;
                self.engine_side = None;
            }
            "go" => {
                if engine.lifecycle() != EngineLifecycle::InGame {
                    writeln!(output, "Error (command not legal now): go")?;
                } else {
                    self.force_mode = false;
                    self.engine_side = Some(engine.game().position().side_to_move());
                    let Some(search) = self.start_search(engine, output)? else {
                        output.flush()?;
                        return Ok(LineAction::Continue);
                    };
                    output.flush()?;
                    return Ok(LineAction::Start(Box::new(search)));
                }
            }
            "setboard" => self.handle_setboard(engine, &tokens[1..], output)?,
            "usermove" => {
                let move_text = tokens.get(1).copied().unwrap_or("");
                if self.handle_usermove(engine, move_text, output)? {
                    let Some(search) = self.start_search(engine, output)? else {
                        output.flush()?;
                        return Ok(LineAction::Continue);
                    };
                    output.flush()?;
                    return Ok(LineAction::Start(Box::new(search)));
                }
            }
            "ping" => {
                let argument = tokens.get(1).copied().unwrap_or("");
                writeln!(output, "pong {argument}")?;
            }
            "result" => {
                let _ = engine.handle(EngineCommand::EndGame);
            }
            "option" => self.handle_option(engine, line, output)?,
            "memory" => self.handle_memory(&tokens[1..], output)?,
            "time" => self.handle_centiseconds(&tokens[1..], true, output)?,
            "otim" => self.handle_centiseconds(&tokens[1..], false, output)?,
            "level" => self.handle_level(&tokens[1..], output)?,
            "st" => self.handle_st(&tokens[1..], output)?,
            "sd" => self.handle_sd(&tokens[1..], output)?,
            "?" => {}
            "quit" => {
                let _ = engine.handle(EngineCommand::Quit);
                return Ok(LineAction::Quit);
            }
            "easy" | "hard" | "post" | "nopost" | "random" | "computer" | "name" | "hint"
            | "draw" => {}
            "undo" | "remove" | "analyze" => {
                writeln!(output, "Error (command not supported): {command}")?;
            }
            _ => writeln!(output, "Error (unknown command): {command}")?,
        }
        output.flush()?;
        Ok(LineAction::Continue)
    }

    /// 当該局面と正規化済み制限から非同期探索を開始する。
    fn start_search(
        &mut self,
        engine: &Engine,
        output: &mut dyn Write,
    ) -> io::Result<Option<ActiveSearch>> {
        let Some(limits) = self.time_control.search_limits() else {
            writeln!(output, "tellusererror search limits are not set")?;
            return Ok(None);
        };
        let game = engine.game();
        let root_moves = game.legal_moves();
        if root_moves.is_empty() {
            writeln!(output, "tellusererror no legal move to search")?;
            return Ok(None);
        }
        let snapshot = SearchSnapshot {
            position: game.position().clone(),
            rules: engine.active_rules(),
            history_keys: game.search_key_history().to_vec(),
            root_moves,
        };
        let search_id = self.next_search_id;
        self.next_search_id = self.next_search_id.wrapping_add(1);
        let transposition_table = self.transposition_table.take().unwrap_or_default();
        Ok(Some(ActiveSearch {
            context: SearchContext { id: search_id },
            handle: search::start_search(snapshot, limits, search_id, transposition_table),
        }))
    }

    /// reader threadが送るCECP入力と探索イベントを並行して処理する。
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

            // CECPは探索中の停止コマンドを完了イベントより先に取り込む。
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
            } else {
                self.wait_search_event(engine, &mut active, output)?;
            }
        }

        if active.is_some() {
            self.discard_search(&mut active)?;
        }
        Ok(())
    }

    /// 探索中に届いた1コマンドを即時停止または後続処理に分類する。
    fn handle_searching_line(
        &mut self,
        engine: &mut Engine,
        active: &mut Option<ActiveSearch>,
        pending: &mut VecDeque<String>,
        line: &str,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        match line.split_whitespace().next() {
            Some("?") => self.finish_search(engine, active, output, true)?,
            Some("force" | "result" | "new" | "quit") => {
                pending.push_back(line.to_owned());
                self.discard_search(active)?;
            }
            _ => pending.push_back(line.to_owned()),
        }
        output.flush()
    }

    /// 溜まっている探索イベントをブロックせずにすべて処理する。
    fn poll_search(
        &mut self,
        engine: &mut Engine,
        active: &mut Option<ActiveSearch>,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        loop {
            let event = match active.as_ref() {
                Some(search) => match search.handle.events().try_recv() {
                    Ok(event) => event,
                    Err(TryRecvError::Empty) => return Ok(()),
                    Err(TryRecvError::Disconnected) => {
                        return self.handle_search_disconnect(active);
                    }
                },
                None => return Ok(()),
            };
            self.handle_search_event(engine, active, event, output)?;
        }
    }

    /// 探索イベントを短時間待って処理する。
    fn wait_search_event(
        &mut self,
        engine: &mut Engine,
        active: &mut Option<ActiveSearch>,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let Some(search) = active.as_ref() else {
            return Ok(());
        };
        match search
            .handle
            .events()
            .recv_timeout(Duration::from_millis(50))
        {
            Ok(event) => self.handle_search_event(engine, active, event, output),
            Err(RecvTimeoutError::Timeout) => Ok(()),
            Err(RecvTimeoutError::Disconnected) => self.handle_search_disconnect(active),
        }
    }

    /// 完了通知なしで探索チャネルが切断された異常を処理する。
    fn handle_search_disconnect(&mut self, active: &mut Option<ActiveSearch>) -> io::Result<()> {
        let Some(search) = active.take() else {
            return Ok(());
        };
        self.transposition_table = Some(join_search(search.handle)?);
        Err(io::Error::other("search ended without a finished event"))
    }

    /// 探索イベントを破棄または通常のエンジン着手へ変換する。
    fn handle_search_event(
        &mut self,
        engine: &mut Engine,
        active: &mut Option<ActiveSearch>,
        event: SearchEvent,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let Some(search) = active.as_ref() else {
            return Ok(());
        };
        if event.search_id() != search.context.id {
            return Ok(());
        }
        match event {
            SearchEvent::Progress { .. } => Ok(()),
            SearchEvent::Finished { best_move, .. } => {
                let search = active.take().expect("active search must exist");
                self.transposition_table = Some(join_search(search.handle)?);
                self.apply_engine_move(engine, best_move, output)
            }
        }
    }

    /// 探索の停止または自然完了を待ち、通常のエンジ着手を処理する。
    fn finish_search(
        &mut self,
        engine: &mut Engine,
        active: &mut Option<ActiveSearch>,
        output: &mut dyn Write,
        request_stop: bool,
    ) -> io::Result<()> {
        let Some(search) = active.take() else {
            return Ok(());
        };
        if request_stop {
            search.handle.request_stop();
        }
        let best_move = loop {
            let event = search
                .handle
                .events()
                .recv()
                .map_err(|_| io::Error::other("search ended without a finished event"))?;
            if event.search_id() != search.context.id {
                continue;
            }
            if let SearchEvent::Finished { best_move, .. } = event {
                break best_move;
            }
        };
        self.transposition_table = Some(join_search(search.handle)?);
        self.apply_engine_move(engine, best_move, output)
    }

    /// 探索結果を出力せずに停止し、置換表だけを回収する。
    fn discard_search(&mut self, active: &mut Option<ActiveSearch>) -> io::Result<()> {
        let Some(search) = active.take() else {
            return Ok(());
        };
        search.handle.request_stop();
        self.transposition_table = Some(join_search(search.handle)?);
        Ok(())
    }

    /// 探索が選んだ手を現局へ適用し、`move`と新規終局結果を順に出力する。
    fn apply_engine_move(
        &mut self,
        engine: &mut Engine,
        best_move: crate::Move,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        match engine.handle(EngineCommand::ApplyMove(best_move)) {
            EngineReply::Accepted { newly_finished, .. } => {
                for leg in cecp::legs(best_move) {
                    writeln!(output, "move {leg}")?;
                }
                if let Some(result) = newly_finished {
                    write_result(output, result)?;
                }
            }
            EngineReply::Rejected(reason) => {
                writeln!(output, "tellusererror engine move rejected: {reason}")?;
                self.force_mode = true;
                self.engine_side = None;
            }
        }
        output.flush()
    }

    /// `protover`への応答としてfeature宣言を出力する。
    ///
    /// setboard・usermove・pingは動作の前提であり、拒否された場合は
    /// セッションを継続しない(handle_lineの`rejected`分岐)。
    fn write_features(&self, output: &mut dyn Write) -> io::Result<()> {
        writeln!(
            output,
            "feature myname=\"minase {}\"",
            env!("CARGO_PKG_VERSION")
        )?;
        writeln!(output, "feature variants=\"chu\"")?;
        writeln!(output, "feature setboard=1")?;
        writeln!(output, "feature usermove=1")?;
        writeln!(output, "feature ping=1")?;
        writeln!(output, "feature colors=0")?;
        writeln!(output, "feature sigint=0")?;
        writeln!(output, "feature sigterm=0")?;
        writeln!(output, "feature analyze=0")?;
        writeln!(output, "feature time=1")?;
        writeln!(output, "feature memory=1")?;
        writeln!(output, "feature draw=0")?;
        writeln!(
            output,
            "feature option=\"RuleSet -string {}\"",
            self.startup_rules_text
        )?;
        writeln!(output, "feature done=1")
    }

    /// `new`を処理し、初期局面から対局を開始する。
    fn handle_new(&mut self, engine: &mut Engine, output: &mut dyn Write) -> io::Result<()> {
        if !matches!(
            engine.handle(EngineCommand::NewGame),
            EngineReply::Accepted { .. }
        ) {
            return writeln!(output, "Error (command not legal now): new");
        }

        let setup = SetupPosition {
            position: Position::initial(),
            lion_capture: None,
            next_move_number: 1,
        };
        if matches!(
            engine.handle(EngineCommand::SetPosition {
                setup,
                moves: Vec::new(),
            }),
            EngineReply::Accepted { .. }
        ) {
            self.force_mode = false;
            self.engine_side = Some(Color::White);
            if let Some(transposition_table) = &mut self.transposition_table {
                transposition_table.clear();
            }
            Ok(())
        } else {
            writeln!(output, "Error (command not legal now): new")
        }
    }

    /// `setboard`のFEN風局面を解析して対局を設定する。
    fn handle_setboard(
        &self,
        engine: &mut Engine,
        fields: &[&str],
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let (Some(board), Some(side)) = (fields.first(), fields.get(1)) else {
            return writeln!(output, "tellusererror Illegal position");
        };
        // CECPの手番記号は白=先手でSFENと逆のため、読み替えて委譲する。
        let sfen_side = match *side {
            "w" => "b",
            "b" => "w",
            _ => return writeln!(output, "tellusererror Illegal position"),
        };
        let text = format!("{board} {sfen_side} - 1");
        let setup = match parse_extended_sfen(&text, engine.position_rules()) {
            Ok(setup) => setup,
            Err(_) => return writeln!(output, "tellusererror Illegal position"),
        };

        match engine.handle(EngineCommand::SetPosition {
            setup,
            moves: Vec::new(),
        }) {
            EngineReply::Accepted { .. } => Ok(()),
            EngineReply::Rejected(RejectReason::GameAlreadyOver) => {
                writeln!(output, "Error (command not legal now): setboard")
            }
            EngineReply::Rejected(_) => writeln!(output, "tellusererror Illegal position"),
        }
    }

    /// `usermove`の指し手を適用し、終局すれば結果行を出力する。
    fn handle_usermove(
        &mut self,
        engine: &mut Engine,
        move_text: &str,
        output: &mut dyn Write,
    ) -> io::Result<bool> {
        let mv = if move_text == "@@@@" {
            let Some(mv) = representative_jitto(engine) else {
                writeln!(output, "Illegal move: {move_text}")?;
                return Ok(false);
            };
            mv
        } else {
            match cecp::parse(engine.game().position(), move_text) {
                Ok(mv) => mv,
                Err(_) => {
                    writeln!(output, "Illegal move: {move_text}")?;
                    return Ok(false);
                }
            }
        };

        match engine.handle(EngineCommand::ApplyMove(mv)) {
            EngineReply::Accepted {
                newly_finished: Some(result),
                ..
            } => {
                write_result(output, result)?;
                Ok(false)
            }
            EngineReply::Accepted { .. } => Ok(!self.force_mode
                && engine.lifecycle() == EngineLifecycle::InGame
                && self.engine_side == Some(engine.game().position().side_to_move())),
            EngineReply::Rejected(RejectReason::IllegalMove {
                cause: IllegalMoveCause::Movement,
                ..
            }) => {
                writeln!(output, "Illegal move: {move_text}")?;
                Ok(false)
            }
            EngineReply::Rejected(RejectReason::IllegalMove {
                cause: IllegalMoveCause::Repetition,
                ..
            }) => {
                writeln!(output, "Illegal move (repetition): {move_text}")?;
                Ok(false)
            }
            EngineReply::Rejected(_) => {
                writeln!(
                    output,
                    "Error (command not legal now): usermove {move_text}"
                )?;
                Ok(false)
            }
        }
    }

    /// `option`のRuleSet設定を処理する。他のoption名は黙って無視する。
    fn handle_option(
        &mut self,
        engine: &mut Engine,
        line: &str,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let Some(remainder) = line.trim_start().strip_prefix("option ") else {
            return Ok(());
        };
        let Some((name, value)) = remainder.split_once('=') else {
            return Ok(());
        };
        if name != "RuleSet" {
            return Ok(());
        }

        let accepted = parse_rule_set(value).is_ok_and(|codes| {
            matches!(
                engine.handle(EngineCommand::SetRules(codes)),
                EngineReply::Accepted { .. }
            )
        });
        if accepted {
            if let Some(transposition_table) = &mut self.transposition_table {
                transposition_table.clear();
            }
            Ok(())
        } else {
            writeln!(output, "Error (invalid option value): {line}")
        }
    }

    /// `memory MB`を正の容量として受理し、待機中の置換表を作り直す。
    fn handle_memory(&mut self, tokens: &[&str], output: &mut dyn Write) -> io::Result<()> {
        let Some(size_mb) = tokens
            .first()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|&size| size > 0)
        else {
            return writeln!(output, "Error (invalid command): memory");
        };
        match &mut self.transposition_table {
            Some(transposition_table) => transposition_table.resize(size_mb),
            None => self.transposition_table = Some(TranspositionTable::new(size_mb)),
        }
        Ok(())
    }

    /// `time`または`otim`の1/100秒値をミリ秒へ正規化する。
    fn handle_centiseconds(
        &mut self,
        tokens: &[&str],
        engine_clock: bool,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        let command = if engine_clock { "time" } else { "otim" };
        let Some(milliseconds) = tokens
            .first()
            .and_then(|value| value.parse::<u64>().ok())
            .and_then(|value| value.checked_mul(10))
        else {
            return writeln!(output, "Error (invalid command): {command}");
        };
        if engine_clock {
            self.time_control.engine_remaining_ms = Some(milliseconds);
        } else {
            self.time_control.opponent_remaining_ms = Some(milliseconds);
        }
        Ok(())
    }

    /// `level`の基本持ち時間と加算秒をミリ秒へ正規化する。
    fn handle_level(&mut self, tokens: &[&str], output: &mut dyn Write) -> io::Result<()> {
        let parsed = (|| {
            let moves = tokens.first()?.parse::<u64>().ok()?;
            let base = parse_level_base_milliseconds(tokens.get(1)?)?;
            let increment = parse_seconds_milliseconds(tokens.get(2)?)?;
            Some((moves, base, increment))
        })();
        let Some((_moves_per_control, base_ms, increment_ms)) = parsed else {
            return writeln!(output, "Error (invalid command): level");
        };
        self.time_control.engine_remaining_ms = Some(base_ms);
        self.time_control.opponent_remaining_ms = Some(base_ms);
        self.time_control.increment_ms = increment_ms;
        Ok(())
    }

    /// `st`の秒値を1手の固定時間(ms)へ正規化する。
    fn handle_st(&mut self, tokens: &[&str], output: &mut dyn Write) -> io::Result<()> {
        let Some(milliseconds) = tokens
            .first()
            .and_then(|value| parse_seconds_milliseconds(value))
        else {
            return writeln!(output, "Error (invalid command): st");
        };
        self.time_control.movetime_ms = Some(milliseconds);
        Ok(())
    }

    /// `sd`の正の整数を探索深さ上限として保持する。
    fn handle_sd(&mut self, tokens: &[&str], output: &mut dyn Write) -> io::Result<()> {
        let Some(depth) = tokens
            .first()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|&depth| (1..=search::MAX_PLY).contains(&depth))
        else {
            return writeln!(output, "Error (invalid command): sd");
        };
        self.time_control.depth = Some(depth);
        Ok(())
    }
}

impl TimeControl {
    /// 保持した正規化値を探索層の制限型へ写す。
    fn search_limits(&self) -> Option<SearchLimits> {
        let clock = self.engine_remaining_ms.map(|remaining_ms| ClockLimits {
            remaining_ms,
            increment_ms: self.increment_ms,
            byoyomi_ms: 0,
        });
        (self.depth.is_some() || self.movetime_ms.is_some() || clock.is_some()).then_some(
            SearchLimits {
                depth: self.depth,
                nodes: None,
                movetime_ms: self.movetime_ms,
                clock,
                infinite: false,
            },
        )
    }
}

impl Protocol for CecpProtocol {
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
                continue;
            }

            match self.handle_idle_line(engine, &command, output)? {
                LineAction::Continue => {}
                LineAction::Start(search) => {
                    active = Some(*search);
                    self.finish_search(engine, &mut active, output, false)?;
                }
                LineAction::Quit => return Ok(()),
            }
        }
        self.discard_search(&mut active)
    }
}

/// 探索スレッドの終了を待ち、貸し出した置換表を回収する。
fn join_search(handle: SearchHandle) -> io::Result<TranspositionTable> {
    handle
        .join()
        .map_err(|_| io::Error::other("search thread panicked"))
}

/// `level`の分または`分:秒`表記をミリ秒へ正規化する。
fn parse_level_base_milliseconds(input: &str) -> Option<u64> {
    let (minutes, seconds) = match input.split_once(':') {
        Some((minutes, seconds)) => {
            let minutes = minutes.parse::<u64>().ok()?;
            let seconds = seconds
                .parse::<u64>()
                .ok()
                .filter(|&seconds| seconds < 60)?;
            (minutes, seconds)
        }
        None => (input.parse::<u64>().ok()?, 0),
    };
    minutes
        .checked_mul(60)?
        .checked_add(seconds)?
        .checked_mul(1_000)
}

/// 非負の秒表記をミリ秒へ正規化し、小数第4位以下は切り捨てる。
fn parse_seconds_milliseconds(input: &str) -> Option<u64> {
    let (whole, fraction) = input.split_once('.').unwrap_or((input, ""));
    let whole_ms = whole.parse::<u64>().ok()?.checked_mul(1_000)?;
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut fraction_ms = 0_u64;
    let mut scale = 100_u64;
    for byte in fraction.bytes().take(3) {
        fraction_ms = fraction_ms.checked_add(u64::from(byte - b'0').checked_mul(scale)?)?;
        scale /= 10;
    }
    whole_ms.checked_add(fraction_ms)
}

/// `@@@@`入力へ割り当てる正準じっとを、移動元の内部密番号が最小の合法手から選ぶ。
fn representative_jitto(engine: &Engine) -> Option<crate::Move> {
    engine
        .game()
        .legal_moves()
        .into_iter()
        .filter(|mv| mv.mid.is_none() && mv.to == mv.from && !mv.promote)
        .min_by_key(|mv| mv.from.dense_index())
}

/// 対局結果をCECPの結果行(`1-0 {reason}`など)として出力する。
fn write_result(output: &mut dyn Write, result: GameResult) -> io::Result<()> {
    let (code, reason) = match result {
        GameResult::Win { winner, reason } => {
            let code = match winner {
                Color::Black => "1-0",
                Color::White => "0-1",
            };
            (code, win_reason_text(reason))
        }
        GameResult::Draw { reason } => ("1/2-1/2", draw_reason_text(reason)),
    };
    writeln!(output, "{code} {{{reason}}}")
}

/// 勝利理由を結果行の注記へ変換する。
const fn win_reason_text(reason: WinReason) -> &'static str {
    match reason {
        WinReason::RoyalCapture => "royal capture",
        WinReason::Mate => "checkmate",
        WinReason::Stalemate => "no legal moves",
        WinReason::Repetition => "repetition",
        WinReason::PieceExhaustion => "piece exhaustion",
        WinReason::BareKing => "bare king",
        WinReason::Resignation => "resignation",
    }
}

/// 引き分け理由を結果行の注記へ変換する。
const fn draw_reason_text(reason: DrawReason) -> &'static str {
    match reason {
        DrawReason::Repetition => "repetition",
        DrawReason::PieceExhaustion => "piece exhaustion",
        DrawReason::BareKing => "bare kings",
        DrawReason::Agreement => "agreement",
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::core::rules::RuleCode;
    use crate::protocol::UsiProtocol;
    use crate::{Game, Rules};

    /// RULES.md第5条の初期配置に対応するSFEN盤面部（setboard・position共用）。
    const INITIAL_BOARD: &str = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL";

    /// 先手玉d4・後手玉i9の2王局面。反復系台本の基底に使う。
    const KINGS: &str = "12/12/12/8k3/12/12/12/12/3K8/12/12/12";

    /// KINGSから開始局面へ戻る4手（同一局面の出現を1回増やす）。
    const KINGS_CYCLE: &str = "usermove d4d5\nusermove i9i8\nusermove d5d4\nusermove i8i9\n";

    /// 先手（White視点）が f6f9 で後手の最後の王駒を取れる局面。
    const ROYAL_BLACK_WINS: &str = "12/12/12/5k6/12/12/5R6/12/12/12/12/K11";

    /// 後手（CECPのBlack、内部White）が f6f4 で先手の最後の王駒を取れる局面。
    const ROYAL_WHITE_WINS: &str = "k11/12/12/12/12/12/5r6/12/5K6/12/12/12";

    /// 先手番で先手飛車f6の唯一の勝ち手（f6f5）が王駒捕獲になる局面。
    const ROOK_MATE: &str = "12/12/12/12/12/12/5R6/5k6/12/12/12/K11";

    /// 先手獅子e6が第1段階で後手歩兵d6を取りc6へ進める2段階移動局面。
    const LION_TWO_STAGE: &str = "11k/12/12/12/12/12/3pN7/12/12/12/12/K11";

    /// 先手獅子f6が隣接する後手歩兵g7を居喰いできる局面。
    const LION_IGUI: &str = "11k/12/12/12/12/6p5/5N6/12/12/12/12/K11";

    /// じっと可能な先手獅子が2枚ある局面（e6とf5）。
    const LIONS_JITTO: &str = "11k/12/12/12/12/12/4N7/5N6/12/12/12/K11";

    /// じっと可能な駒が存在しない2王局面。
    const KINGS_ONLY: &str = "11k/12/12/12/12/12/12/12/12/12/12/K11";

    /// 先手歩兵e8が次の前進e9で敵陣（RULES.md第18条）へ入る局面。
    const PAWN_PROMO: &str = "11k/12/12/12/4P7/12/12/12/12/12/12/K11";

    fn make_engine(codes: &[RuleCode]) -> Engine {
        Engine::new(codes.to_vec()).unwrap()
    }

    fn run(protocol: &mut CecpProtocol, engine: &mut Engine, input: &str) -> String {
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();
        protocol.run(engine, &mut input, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn time_and_otim_normalize_centiseconds_to_milliseconds() {
        // 変異検証(フェーズ4)補強・実装契約: time/otimの1/100秒→ミリ秒換算(×10、
        // engine-connectivity.mdの単位規定)はwire出力に現れず時間管理へだけ流れる
        // ため、ここに限り内部時計を直接観測して換算係数を固定する。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);
        let output = run(&mut protocol, &mut engine, "time 6000\notim 123\n");

        assert!(output.is_empty());
        assert_eq!(protocol.time_control.engine_remaining_ms, Some(60_000));
        assert_eq!(protocol.time_control.opponent_remaining_ms, Some(1_230));
    }

    fn run_queued(protocol: &mut CecpProtocol, engine: &mut Engine, input: &str) -> String {
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
        let mut protocol = CecpProtocol::new(&engine);
        run(&mut protocol, &mut engine, input)
    }

    fn queued_session(codes: &[RuleCode], input: &str) -> String {
        let mut engine = make_engine(codes);
        let mut protocol = CecpProtocol::new(&engine);
        run_queued(&mut protocol, &mut engine, input)
    }

    fn usi_session(codes: &[RuleCode], input: &str) -> String {
        let mut engine = make_engine(codes);
        let mut protocol = UsiProtocol::new(&engine);
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();
        protocol.run(&mut engine, &mut input, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    fn move_lines(output: &str) -> Vec<&str> {
        output
            .lines()
            .filter(|line| line.starts_with("move "))
            .collect()
    }

    /// レグ分割されたmove行列を論理的なエンジン着手数へ数える（非最終レグは末尾コンマ）。
    fn logical_move_count(output: &str) -> usize {
        move_lines(output)
            .iter()
            .filter(|line| !line.ends_with(','))
            .count()
    }

    #[test]
    fn protover_declares_the_feature_set_with_done_last() {
        // PL「CECPのfeature宣言」＋EC（time=1へ改定・memory=1追加。現行はECが正）（D6-CECP-01）。
        // 宣言の行順は仕様が任意とするため（CE第4章）、done=1の終端以外は集合として検証する。
        let output = session(
            &[RuleCode::E2, RuleCode::R1, RuleCode::L1],
            "xboard\nprotover 2\nquit\n",
        );
        let lines: Vec<_> = output.lines().collect();

        assert!(lines.iter().all(|line| line.starts_with("feature ")));
        assert_eq!(*lines.last().unwrap(), "feature done=1");
        for expected in [
            "feature variants=\"chu\"",
            "feature setboard=1",
            "feature usermove=1",
            "feature ping=1",
            "feature colors=0",
            "feature sigint=0",
            "feature sigterm=0",
            "feature analyze=0",
            "feature time=1",
            "feature memory=1",
            "feature draw=0",
            "feature option=\"RuleSet -string L1,R1,E2\"",
        ] {
            assert!(lines.contains(&expected), "missing: {expected}");
        }
        // 文字列値は二重引用符で囲む（PL実施状況フェーズ5のレビュー修正）。
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("feature myname=\"minase ") && line.ends_with('"'))
        );
        // debug・highlight・san・reuseは宣言しない（PL）。
        for absent in ["debug=", "highlight=", "san=", "reuse="] {
            assert!(!output.contains(absent), "unexpected: {absent}");
        }
    }

    #[test]
    fn required_feature_rejections_terminate_the_session() {
        // PL「プロトコル固有の制御コマンド」: 必須feature（setboard・usermove・ping）の拒否は
        // tellusererrorを出して終了し、他の拒否は無視する（D6-CECP-02）。
        for feature in ["setboard", "usermove", "ping"] {
            let output = session(
                &[RuleCode::R1],
                &format!("protover 2\nrejected {feature}\nping 9\n"),
            );
            assert!(
                output
                    .lines()
                    .any(|line| line.starts_with("tellusererror "))
            );
            // セッションが終了しており、後続のpingは処理されない。
            assert!(!output.contains("pong 9"));
        }

        let output = session(
            &[RuleCode::R1],
            "protover 2\nrejected time\nrejected memory\nrejected debug\naccepted setboard\nnew\nping 1\n",
        );
        assert!(!output.contains("tellusererror"));
        assert!(output.ends_with("pong 1\n"));
    }

    #[test]
    fn variant_accepts_only_exact_chu() {
        // PL「プロトコル固有の制御コマンド」: variantはchuだけを受理する（D6-CECP-03）。
        // エラー本文の完全形はSU-12により前方一致で検証する。
        let output = session(
            &[RuleCode::R1],
            "new\nvariant chu\nvariant shogi\nvariant dai\nforce\nusermove g4g5\n",
        );
        let lines: Vec<_> = output.lines().collect();

        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert!(line.starts_with("Error (unsupported variant): "));
        }
        // 拒否後も状態は不変であり、末尾のusermoveが黙って受理されている。
    }

    #[test]
    fn cecp_white_matches_the_usi_first_player() {
        // PL「内部正準と手番文字の変換責任」・「setboardの受理契約」（D6-CECP-04）。
        // 同一盤面をCECPのwとUSIのbで設定すると、先手の同じ着手が双方で受理される。
        assert_eq!(
            session(
                &[RuleCode::R1],
                &format!("setboard {INITIAL_BOARD} w\nusermove g4g5\n"),
            ),
            ""
        );
        assert_eq!(
            usi_session(
                &[RuleCode::R1],
                &format!("position sfen {INITIAL_BOARD} b - 1 moves 6i6h\n"),
            ),
            ""
        );

        // 逆の手番文字では同じ先手の着手が双方で拒否される。
        assert_eq!(
            session(
                &[RuleCode::R1],
                &format!("setboard {INITIAL_BOARD} b\nusermove g4g5\n"),
            ),
            "Illegal move: g4g5\n"
        );
        let output = usi_session(
            &[RuleCode::R1],
            &format!("position sfen {INITIAL_BOARD} w - 1 moves 6i6h\n"),
        );
        assert_eq!(output.lines().count(), 1);
        assert!(output.starts_with("info string error: "));

        // 不正な手番文字は拒否する（D6-CECP-24の失敗経路）。
        assert_eq!(
            session(&[RuleCode::R1], &format!("setboard {INITIAL_BOARD} x\n")),
            "tellusererror Illegal position\n"
        );
    }

    #[test]
    fn result_lines_take_the_white_viewpoint_and_appear_once() {
        // PL「内部正準…」（1-0は先手勝ち）・「思考開始指示と終局裁定の通知」の理由表・
        // EC「着手と結果の順序」（usermove終局はnewly_finishedからRESULT1回）（D6-CECP-05、D6-CECP-16）。
        let output = session(
            &[RuleCode::R1, RuleCode::E2],
            &format!("setboard {ROYAL_BLACK_WINS} w\nusermove f6f9\nping 3\n"),
        );
        // usermoveでの終局はmove行なしのRESULT1回であり、後続コマンドで再生成されない。
        assert_eq!(output, "1-0 {royal capture}\npong 3\n");

        assert_eq!(
            session(
                &[RuleCode::R1, RuleCode::E2],
                &format!("setboard {ROYAL_WHITE_WINS} b\nusermove f6f4\n"),
            ),
            "0-1 {royal capture}\n"
        );

        // R1の4回目の同一局面は引き分け裁定でありIllegal moveではない（RULES.md第31条R1、D6-ENG-03境界）。
        // force相当（起動直後）の棋譜再生中でも裁定は生きている（D6-CECP-13）。
        let cycles = KINGS_CYCLE.repeat(3);
        assert_eq!(
            session(
                &[RuleCode::R1, RuleCode::E2],
                &format!("setboard {KINGS} w\n{cycles}"),
            ),
            "1/2-1/2 {repetition}\n"
        );
    }

    #[test]
    fn usermove_legs_must_be_continuous() {
        // PL「Move文字列表記2形式」: 受信はコンマ区切り単一文字列、第2レグの始点不一致は拒否（D6-CECP-07）。
        assert_eq!(
            session(
                &[RuleCode::R1, RuleCode::E2],
                &format!("setboard {LION_TWO_STAGE} w\nusermove e6d6,d6c6\nusermove l12k12\n"),
            ),
            "" // 2段階捕獲が受理され、後続の後手の着手も受理される
        );
        assert_eq!(
            session(
                &[RuleCode::R1, RuleCode::E2],
                &format!("setboard {LION_TWO_STAGE} w\nusermove e6d6,b6a6\n"),
            ),
            "Illegal move: e6d6,b6a6\n"
        );
        // 3レグ以上は中将棋の合法手に対応しないため拒否する。
        assert_eq!(
            session(
                &[RuleCode::R1, RuleCode::E2],
                &format!("setboard {LION_TWO_STAGE} w\nusermove e6d6,d6c6,c6b6\n"),
            ),
            "Illegal move: e6d6,d6c6,c6b6\n"
        );
        // 居喰い（RULES.md第12条）は@@@@ではなく明示レグで送る（D6-CECP-09境界）。
        assert_eq!(
            session(
                &[RuleCode::R1, RuleCode::E2],
                &format!("setboard {LION_IGUI} w\nusermove f6g7,g7f6\nusermove l12k12\n"),
            ),
            ""
        );
    }

    #[test]
    fn promotion_suffixes_are_interpreted_on_the_final_leg_only() {
        // PL「CECP指し手表記の関数契約」: +は最終レグの末尾だけ、=は不成として受理（D6-CECP-08）。
        // 成りの成立は、成駒（歩兵→金将の動き、RULES.md第9条）の横移動の受理で観測する。
        assert_eq!(
            session(
                &[RuleCode::R1, RuleCode::E2],
                &format!(
                    "setboard {PAWN_PROMO} w\nusermove e8e9+\nusermove l12k12\nusermove e9f9\n"
                ),
            ),
            ""
        );
        // 無印と=はどちらも不成であり、歩兵は横へ動けない。
        for unpromoted in ["e8e9", "e8e9="] {
            assert_eq!(
                session(
                    &[RuleCode::R1, RuleCode::E2],
                    &format!(
                        "setboard {PAWN_PROMO} w\nusermove {unpromoted}\nusermove l12k12\nusermove e9f9\n"
                    ),
                ),
                "Illegal move: e9f9\n"
            );
        }
        // 非最終レグへの+は拒否する。
        assert_eq!(
            session(
                &[RuleCode::R1, RuleCode::E2],
                &format!("setboard {LION_TWO_STAGE} w\nusermove e6d6+,d6c6\n"),
            ),
            "Illegal move: e6d6+,d6c6\n"
        );
        // 成れない着手への+も拒否する（RULES.md第26条）。
        assert_eq!(
            session(&[RuleCode::R1], "new\nforce\nusermove g4g5+\n"),
            "Illegal move: g4g5+\n"
        );
    }

    #[test]
    fn at_at_plays_a_canonical_jitto_or_is_rejected() {
        // PL「Move文字列表記2形式」の@@@@契約（D6-CECP-09）。本領域は受理・拒否の外形のみを
        // 固定し、代表選択が局面遷移へ影響しないことの検証はD5表記領域が分担する。
        assert_eq!(
            session(
                &[RuleCode::R1, RuleCode::E2],
                &format!("setboard {LIONS_JITTO} w\nusermove @@@@\nusermove l12k12\n"),
            ),
            "" // じっとが受理され手番が渡っている（後手の着手が受理される）
        );
        // じっと可能な駒がない局面では拒否する。
        assert_eq!(
            session(
                &[RuleCode::R1, RuleCode::E2],
                &format!("setboard {KINGS_ONLY} w\nusermove @@@@\n"),
            ),
            "Illegal move: @@@@\n"
        );
    }

    #[test]
    fn engine_moves_are_emitted_as_move_lines_with_a_legal_payload() {
        // EC「着手と結果の順序」・PL送信形式（D6-CECP-10）。探索の具体的な着手は評価依存の
        // ため固定せず、レグ連結をparseへ往復して合法手集合への所属だけを検証する。
        let output = session(&[RuleCode::R1], "sd 1\nnew\ngo\n");
        let lines = move_lines(&output);

        assert_eq!(output.lines().count(), lines.len());
        assert_eq!(logical_move_count(&output), 1);
        let payload: String = lines
            .iter()
            .map(|line| line.strip_prefix("move ").unwrap())
            .collect();

        let game = Game::new(Rules::from_codes(&[RuleCode::R1]).unwrap()).unwrap();
        if payload == "@@@@" {
            // 正準じっと（移動元=移動先・中間升なし・不成）が初期局面に存在することの確認。
            assert!(
                game.legal_moves()
                    .iter()
                    .any(|mv| mv.mid.is_none() && mv.to == mv.from && !mv.promote)
            );
        } else {
            let mv = cecp::parse(game.position(), &payload).unwrap();
            assert!(game.legal_moves().contains(&mv));
        }
    }

    #[test]
    fn illegal_moves_use_the_two_canonical_forms() {
        // PL「思考開始指示と終局裁定の通知」: Movementは省略形、Repetitionは注記付き
        // （D6-CECP-11、D6-ENG-03）。RULES.md第27条第4項（R2の禁止手は不合法な着手）。
        let output = session(
            &[RuleCode::R1],
            "new\nforce\nusermove a1b1\nusermove g4g5\n",
        );
        // 拒否後も同じ局面で別の合法手が受理される（状態不変）。
        assert_eq!(output, "Illegal move: a1b1\n");

        let output = session(
            &[RuleCode::R2, RuleCode::E2],
            &format!(
                "setboard {KINGS} w\nusermove d4d5\nusermove i9i8\nusermove d5d4\nusermove i8i9\nusermove i8i7\n"
            ),
        );
        // 既出局面を再現する4手目だけが拒否され、続く別の合法手i8i7は受理される。
        assert_eq!(output, "Illegal move (repetition): i8i9\n");
    }

    #[test]
    fn usermove_triggers_exactly_one_automatic_reply() {
        // EC「状態機械」: usermove後、¬force∧継続中∧手番=担当なら探索する（D6-CECP-12）。
        // PL「newの意味論」: newは初期局面でInGameへ入り、エンジンは着手を自発しない（D6-CECP-06）。
        let output = session(&[RuleCode::R1], "sd 1\nnew\nusermove g4g5\n");

        // new直後に自発着手はなく、usermoveへの応手だけが出る。
        assert!(!output.is_empty());
        assert!(output.lines().all(|line| line.starts_with("move ")));
        // 自分が指した直後（手番が相手側）には探索しないため、応手はちょうど1手。
        assert_eq!(logical_move_count(&output), 1);
    }

    #[test]
    fn force_applies_both_sides_without_replies() {
        // EC「状態機械」: forceは着手を出さずusermoveを両陣営分適用する（D6-CECP-13）。
        // 不合法手への応答はillegal_moves_use_the_two_canonical_formsが、裁定（RESULT）の
        // 維持はresult_lines_take_the_white_viewpoint_and_appear_onceが検証する。
        assert_eq!(
            session(
                &[RuleCode::R1],
                "sd 1\nnew\nforce\nusermove g4g5\nusermove g9g8\n"
            ),
            ""
        );
    }

    #[test]
    fn go_adopts_the_current_side_and_rejects_out_of_game_states() {
        // EC「状態機械」: goは継続中に限りforceを解除し現在手番を担当して探索する（D6-CECP-14）。
        let output = session(&[RuleCode::R1], "sd 1\nnew\nforce\nusermove g4g5\ngo\n");
        assert!(!output.is_empty());
        assert!(output.lines().all(|line| line.starts_with("move ")));

        // AwaitingStartのgoは探索なしのエラー（具体形はECが「エラーを返し」とだけ規定）。
        let output = session(&[RuleCode::R1], "sd 1\ngo\n");
        assert_eq!(output.lines().count(), 1);
        assert!(!output.contains("move "));
        assert!(output.starts_with("Error") || output.starts_with("tellusererror"));
        // Finished後のgo拒否はan_engine_finishing_move_emits_move_then_result_onceが検証する。
    }

    #[test]
    fn go_without_search_limits_is_a_tellusererror() {
        // EC実施状況フェーズ2の実装判断: 探索制限のないgoは暗黙の既定値でフォールバック
        // せずtellusererrorを返す（D6-CECP-15。絶対規則: 暗黙フォールバック禁止）。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        let output = run(&mut protocol, &mut engine, "new\ngo\n");
        assert_eq!(output.lines().count(), 1);
        assert!(output.starts_with("tellusererror"));
        assert!(!output.contains("move "));

        // sd設定後のgoは正常に探索する。
        let output = run(&mut protocol, &mut engine, "sd 1\ngo\n");
        assert!(output.lines().any(|line| line.starts_with("move ")));
    }

    #[test]
    fn an_engine_finishing_move_emits_move_then_result_once() {
        // EC「着手と結果の順序」: ApplyMove→move行→newly_finished時のみRESULT1回（D6-CECP-16、
        // D6-ENG-04）。RESULT後のgoは拒否される（D6-CECP-14の(b)）。
        let output = session(
            &[RuleCode::R1, RuleCode::E2],
            &format!("sd 1\nsetboard {ROOK_MATE} w\ngo\nping 5\ngo\n"),
        );
        let lines: Vec<_> = output.lines().collect();
        let result = lines
            .iter()
            .position(|line| *line == "1-0 {royal capture}")
            .expect("the winning engine move must emit the exact RESULT line");
        let pong = lines.iter().position(|line| *line == "pong 5").unwrap();

        // move行はすべてRESULTより前で、RESULTはセッション全体で1回だけ。
        assert!(logical_move_count(&output) >= 1);
        assert!(lines[..result].iter().all(|line| line.starts_with("move ")));
        assert_eq!(
            lines
                .iter()
                .filter(|line| **line == "1-0 {royal capture}")
                .count(),
            1
        );
        assert!(result < pong);
        // RESULT後のgoは探索もmoveもなしのエラー応答。
        let last = lines.last().unwrap();
        assert!(last.starts_with("Error") || last.starts_with("tellusererror"));
        assert!(
            lines[result + 1..]
                .iter()
                .all(|line| !line.starts_with("move "))
        );
    }

    #[test]
    fn result_is_accepted_as_the_external_verdict() {
        // EC「状態機械」: resultは外部要因を含む確定通知であり、エンジンは異議を唱えず
        // AwaitingStartへ戻る（D6-CECP-17）。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        let output = run(
            &mut protocol,
            &mut engine,
            "sd 1\nnew\nforce\nusermove g4g5\nresult 1-0 {time forfeit}\nusermove g9g8\n",
        );
        let lines: Vec<_> = output.lines().collect();
        // result自体は無応答で、対局外になった後のusermoveは拒否系応答になる。
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].starts_with("move "));
        assert!(!lines[0].starts_with("Illegal move"));

        // newで次局を開始できる。
        assert_eq!(
            run(&mut protocol, &mut engine, "new\nforce\nusermove g4g5\n"),
            ""
        );
    }

    #[test]
    fn move_now_plays_immediately_or_is_ignored() {
        // EC「状態機械」: ?は探索中なら停止してその時点の最善手で通常の着手処理を行い、
        // 探索中でなければ無視する（D6-CECP-18）。
        let output = queued_session(&[RuleCode::R1], "sd 256\nnew\ngo\n?\n");
        assert!(output.lines().any(|line| line.starts_with("move ")));

        assert_eq!(session(&[RuleCode::R1], "?\n"), "");
    }

    #[test]
    fn stop_class_commands_discard_the_running_search() {
        // EC実施状況フェーズ2: 探索中のforce・result・new・quitは停止・破棄で、遅延した
        // move行が漏れない（D6-CECP-31、D6-CECP-17/06/29の探索中経路）。
        assert!(
            !queued_session(&[RuleCode::R1], "sd 256\nnew\ngo\nresult 1-0 {external}\n")
                .contains("move ")
        );
        assert!(!queued_session(&[RuleCode::R1], "sd 256\nnew\ngo\nquit\n").contains("move "));
        assert_eq!(
            queued_session(
                &[RuleCode::R1],
                "sd 256\nnew\ngo\nforce\nusermove g4g5\nusermove g9g8\n",
            ),
            ""
        );

        // 探索中のnewは破棄のうえ次局を開始する。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);
        assert!(
            !run_queued(&mut protocol, &mut engine, "sd 256\nnew\ngo\nnew\n").contains("move ")
        );
        // 次局がInGameで始まっている（初期局面の先手の着手が受理される）。
        assert_eq!(
            run(&mut protocol, &mut engine, "force\nusermove g4g5\n"),
            ""
        );
    }

    #[test]
    fn pong_echoes_after_prior_commands_complete() {
        // PL（ping Nにpong N）・EC実施状況フェーズ2（探索中のpongはmove行の後）（D6-CECP-19）。
        assert_eq!(session(&[RuleCode::R1], "ping 42\n"), "pong 42\n");

        let output = queued_session(&[RuleCode::R1], "sd 1\nnew\ngo\nping 7\n");
        let lines: Vec<_> = output.lines().collect();
        let last_move = lines
            .iter()
            .rposition(|line| line.starts_with("move "))
            .unwrap();
        let pong = lines.iter().position(|line| *line == "pong 7").unwrap();
        assert!(last_move < pong);
    }

    #[test]
    fn time_commands_are_accepted_and_normalized_to_milliseconds() {
        // EC「責務分担」・実施状況フェーズ2: time/otimは1/100秒、levelは分・分:秒・加算秒、
        // stは秒、sdは深さで、正規化先はミリ秒のSearchLimits（D6-CECP-20〜22）。
        let output = session(
            &[RuleCode::R1],
            "time 6000\notim 6000\nlevel 40 5 0\nlevel 0 0:30 1\nst 5\nsd 3\nnew\nsd 1\ngo\n",
        );
        // すべて無応答で受理され、後続のgoが正常に着手する。
        assert!(!output.is_empty());
        assert!(output.lines().all(|line| line.starts_with("move ")));

        // 正規化値の検証はマトリクスの指示どおり引数解析の単体レベルで行う。
        assert_eq!(parse_level_base_milliseconds("5"), Some(300_000));
        assert_eq!(parse_level_base_milliseconds("0:30"), Some(30_000));
        assert_eq!(parse_level_base_milliseconds("5:30"), Some(330_000));
        assert_eq!(parse_seconds_milliseconds("1"), Some(1_000));
        assert_eq!(parse_seconds_milliseconds("2.5"), Some(2_500));
        assert_eq!(parse_seconds_milliseconds("3.25"), Some(3_250));
    }

    #[test]
    fn memory_is_accepted_while_idle_and_search_still_works() {
        // EC: feature memory=1とmemory <MB>の非探索中リサイズ（D6-CECP-23）。置換表の内部
        // 効果はSU-13により観測せず、受理の外形（エラーなし・後続goの正常）だけを契約とする。
        let output = session(&[RuleCode::R1], "memory 64\nmemory 1\nsd 1\nnew\ngo\n");

        assert!(!output.is_empty());
        assert!(output.lines().all(|line| line.starts_with("move ")));
    }

    #[test]
    fn setboard_takes_two_fields_ignores_extras_and_fails_atomically() {
        // PL「setboardの受理契約」: 2欄必須・3欄目以降は無視・解析失敗はtellusererror
        // Illegal positionで状態不変（D6-CECP-24、D6-ENG-01/05の外形）。
        let output = session(
            &[RuleCode::R1, RuleCode::E2],
            &format!(
                concat!(
                    "new\nforce\n",
                    "setboard 13/12/12/12/12/12/12/12/12/12/12/12 w\n", // 壊れた盤面部
                    "usermove g4g5\n",       // 直前の初期局面が保持されている
                    "setboard {} w - 0 1\n", // 余剰欄は無視して現局を置換
                    "usermove d4e5\n",       // 新局面基準の合法判定（初期局面では不合法な手）
                    "setboard {} x\n",       // 不正な手番文字
                    "usermove i9i8\n",       // 失敗後も置換されていない（後手番のまま）
                    "setboard {}\n",         // 手番欄の欠如
                ),
                KINGS, KINGS, KINGS
            ),
        );

        assert_eq!(
            output,
            concat!(
                "tellusererror Illegal position\n",
                "tellusererror Illegal position\n",
                "tellusererror Illegal position\n",
            )
        );
    }

    #[test]
    fn ruleset_option_latches_until_new_and_rejects_invalid_values() {
        // PL「規則オプション」: option NAME=VALUEで検証し、正当ならpendingのみ更新、不正なら
        // Error (invalid option value): <受信行>。commitはnew（D6-CECP-25、D6-ENG-02）。
        let mut engine = make_engine(&[RuleCode::R1, RuleCode::E2]);
        let mut protocol = CecpProtocol::new(&engine);
        let cycle = KINGS_CYCLE;
        let output = run(
            &mut protocol,
            &mut engine,
            &format!(
                concat!(
                    "setboard {} w\n",
                    "{}", // R1では2回目の同一局面は合法
                    "option RuleSet=R2,E2\n",
                    "{}", // 変更は対局中に反映されない（3回目の出現も合法のまま）
                    "option RuleSet=XX9\n",
                    "option RuleSet=lishogi,P1\n",
                    "option RuleSet=L1,E1\n", // 反復規則欠如も受信時に拒否
                    "new\nforce\n",
                    "setboard {} w\n",
                    "usermove d4d5\nusermove i9i8\nusermove d5d4\n",
                    "usermove i8i9\n", // R2がcommit済みなら反復禁止手になる
                ),
                KINGS, cycle, cycle, KINGS
            ),
        );

        assert_eq!(
            output,
            concat!(
                "Error (invalid option value): option RuleSet=XX9\n",
                "Error (invalid option value): option RuleSet=lishogi,P1\n",
                "Error (invalid option value): option RuleSet=L1,E1\n",
                "Illegal move (repetition): i8i9\n",
            )
        );

        // プリセット名は大文字小文字を区別せず単独指定で受理する（R33第5・6項）。
        assert_eq!(session(&[RuleCode::R1], "option RuleSet=LISHOGI\n"), "");
        assert_eq!(
            session(&[RuleCode::R1], "option RuleSet=engine-default\n"),
            ""
        );
    }

    #[test]
    fn ignored_commands_produce_no_output_or_state_change() {
        // PL「コマンド対応の残余」の無視リストからECが処理へ昇格したものを除いた集合
        // （easy・hard・post・nopost・random・computer・name・hint・draw）（D6-CECP-26）。
        assert_eq!(
            session(
                &[RuleCode::R1],
                "ping a\neasy\nhard\npost\nnopost\nrandom\ncomputer\nname foo\nhint\ndraw\nping b\n",
            ),
            "pong a\npong b\n"
        );
    }

    #[test]
    fn undo_remove_and_analyze_report_command_not_supported() {
        // PL「コマンド対応の残余」: 無視すると不整合を生む3コマンドだけがエラーになる（D6-CECP-27）。
        assert_eq!(
            session(
                &[RuleCode::R1],
                "new\nforce\nundo\nremove\nanalyze\nusermove g4g5\n"
            ),
            concat!(
                "Error (command not supported): undo\n",
                "Error (command not supported): remove\n",
                "Error (command not supported): analyze\n",
            )
        );
    }

    #[test]
    fn unknown_commands_echo_only_the_first_token() {
        // PL「コマンド対応の残余」・CE第3章: 未知の行はError (unknown command): <第1トークン>
        // （D6-CECP-28。未知行を無視するUSIとの意図的な差）。
        assert_eq!(
            session(
                &[RuleCode::R1],
                "foobar baz qux\nnew\nforce\nusermove g4g5\n"
            ),
            "Error (unknown command): foobar\n"
        );
    }

    #[test]
    fn xboard_is_ignored_and_quit_ends_the_session_silently() {
        // PL「プロトコル固有の制御コマンド」: xboardは無視、quitは無応答で終了（D6-CECP-29）。
        // quit後の入力は処理されない。
        assert_eq!(session(&[RuleCode::R1], "xboard\nquit\nping 1\n"), "");
    }

    #[test]
    fn result_reason_table_matches_the_documented_mapping() {
        // PL「思考開始指示と終局裁定の通知」の理由文字列対応表と、BG「裁定理由enumの分割」
        // によるBareKingの訳語（D6-CECP-05）。
        let wins = [
            (WinReason::RoyalCapture, "royal capture"),
            (WinReason::Mate, "checkmate"),
            (WinReason::Stalemate, "no legal moves"),
            (WinReason::Repetition, "repetition"),
            (WinReason::PieceExhaustion, "piece exhaustion"),
            (WinReason::BareKing, "bare king"),
            (WinReason::Resignation, "resignation"),
        ];
        for (reason, text) in wins {
            assert_eq!(win_reason_text(reason), text);
        }
        let draws = [
            (DrawReason::Repetition, "repetition"),
            (DrawReason::PieceExhaustion, "piece exhaustion"),
            (DrawReason::BareKing, "bare kings"),
            (DrawReason::Agreement, "agreement"),
        ];
        for (reason, text) in draws {
            assert_eq!(draw_reason_text(reason), text);
        }

        // 結果コードは白（先手）視点: 先手勝ち1-0、後手勝ち0-1、引き分け1/2-1/2。
        let mut buffer = Vec::new();
        write_result(
            &mut buffer,
            GameResult::Win {
                winner: Color::Black,
                reason: WinReason::Mate,
            },
        )
        .unwrap();
        write_result(
            &mut buffer,
            GameResult::Win {
                winner: Color::White,
                reason: WinReason::Mate,
            },
        )
        .unwrap();
        write_result(
            &mut buffer,
            GameResult::Draw {
                reason: DrawReason::BareKing,
            },
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(buffer).unwrap(),
            "1-0 {checkmate}\n0-1 {checkmate}\n1/2-1/2 {bare kings}\n"
        );
    }
}
