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
    use crate::core::piece::{PieceCode, PieceKind};
    use crate::core::position::PositionBuilder;
    use crate::core::rules::RuleCode;
    use crate::protocol::UsiProtocol;
    use crate::protocol::engine::EngineLifecycle;
    use crate::test_util::sq;
    use crate::{GameStatus, Rules, WinReason, to_sfen};

    const INITIAL_BOARD: &str = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL";

    fn engine(codes: &[RuleCode]) -> Engine {
        Engine::new(codes.to_vec()).unwrap()
    }

    fn run(protocol: &mut CecpProtocol, engine: &mut Engine, input: &str) -> String {
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();
        protocol.run(engine, &mut input, &mut output).unwrap();
        String::from_utf8(output).unwrap()
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

    fn run_usi(protocol: &mut UsiProtocol, engine: &mut Engine, input: &str) -> String {
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();
        protocol.run(engine, &mut input, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    fn board(position: &Position) -> String {
        to_sfen(position).split_once(' ').unwrap().0.to_owned()
    }

    fn position_with(pieces: &[(crate::Square, Color, PieceKind)]) -> Position {
        let mut builder = PositionBuilder::new(Color::Black);
        for &(square, color, kind) in pieces {
            builder.put(square, PieceCode::new(color, kind)).unwrap();
        }
        builder.finish().unwrap()
    }

    #[test]
    fn handshake_matches_the_complete_feature_transcript() {
        let startup = [RuleCode::E2, RuleCode::R1, RuleCode::L1];
        let mut engine = engine(&startup);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(
            run(&mut protocol, &mut engine, "xboard\nprotover 2\nquit\n"),
            concat!(
                "feature myname=\"minase ",
                env!("CARGO_PKG_VERSION"),
                "\"\n",
                "feature variants=\"chu\"\n",
                "feature setboard=1\n",
                "feature usermove=1\n",
                "feature ping=1\n",
                "feature colors=0\n",
                "feature sigint=0\n",
                "feature sigterm=0\n",
                "feature analyze=0\n",
                "feature time=1\n",
                "feature memory=1\n",
                "feature draw=0\n",
                "feature option=\"RuleSet -string L1,R1,E2\"\n",
                "feature done=1\n",
            )
        );
    }

    #[test]
    fn required_feature_rejection_terminates_but_other_rejections_are_ignored() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);
        let input = "rejected draw\naccepted setboard\nping 007\nrejected ping\nprotover 2\n";

        assert_eq!(
            run(&mut protocol, &mut engine, input),
            concat!(
                "pong 007\n",
                "tellusererror minase requires the ping feature\n"
            )
        );
    }

    #[test]
    fn variant_accepts_only_exact_chu() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "variant chu\nvariant shogi\nquit\n"
            ),
            "Error (unsupported variant): shogi\n"
        );
    }

    #[test]
    fn cecp_and_usi_side_to_move_conventions_are_cross_checked() {
        let mut cecp_engine = engine(&[RuleCode::R1]);
        let mut cecp_protocol = CecpProtocol::new(&cecp_engine);
        assert_eq!(
            run(
                &mut cecp_protocol,
                &mut cecp_engine,
                &format!("setboard {INITIAL_BOARD} w\n")
            ),
            ""
        );

        let mut usi_engine = engine(&[RuleCode::R1]);
        let mut usi_protocol = UsiProtocol::new(&usi_engine);
        assert_eq!(
            run_usi(
                &mut usi_protocol,
                &mut usi_engine,
                &format!("position sfen {INITIAL_BOARD} b - 1\n")
            ),
            ""
        );
        assert_eq!(
            cecp_engine.game().position().side_to_move(),
            usi_engine.game().position().side_to_move()
        );
        assert_eq!(cecp_engine.game().position().side_to_move(), Color::Black);

        assert_eq!(
            run(
                &mut cecp_protocol,
                &mut cecp_engine,
                &format!("setboard {INITIAL_BOARD} b\nquit\n")
            ),
            ""
        );
        assert_eq!(cecp_engine.game().position().side_to_move(), Color::White);
    }

    #[test]
    fn new_starts_immediately_and_rules_latch_until_the_next_new() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(
            run(&mut protocol, &mut engine, "new\nforce\nusermove g4g5\n"),
            ""
        );
        assert_eq!(engine.game().ply_count(), 1);
        assert_eq!(engine.lifecycle(), EngineLifecycle::InGame);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                concat!(
                    "option RuleSet=R2,E2\n",
                    "option RuleSet=P3,E2\n",
                    "option Unknown=value\n",
                    "option Button\n",
                )
            ),
            "Error (invalid option value): option RuleSet=P3,E2\n"
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R1]);
        assert_eq!(engine.pending_rule_codes(), &[RuleCode::R2, RuleCode::E2]);

        assert_eq!(run(&mut protocol, &mut engine, "new\nquit\n"), "");
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R2, RuleCode::E2]);
        assert_eq!(engine.lifecycle(), EngineLifecycle::InGame);
    }

    #[test]
    fn engine_default_preset_sets_r1_as_pending_rules() {
        let mut engine = engine(&[RuleCode::R2]);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "option RuleSet=engine-default\nquit\n",
            ),
            ""
        );
        assert_eq!(engine.active_rule_codes(), &[RuleCode::R2]);
        assert_eq!(engine.pending_rule_codes(), &[RuleCode::R1]);
    }

    #[test]
    fn multileg_capture_and_igui_are_applied() {
        let two_stage_position = position_with(&[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(4, 5), Color::Black, PieceKind::Lion),
            (sq(3, 5), Color::White, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ]);
        let mut engine = engine(&[RuleCode::R1, RuleCode::E2]);
        let mut protocol = CecpProtocol::new(&engine);
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!(
                    "setboard {} w\nusermove e6d6,d6c6\n",
                    board(&two_stage_position)
                )
            ),
            ""
        );
        assert_eq!(
            engine.game().position().piece_at(sq(2, 5)),
            Some(PieceCode::new(Color::Black, PieceKind::Lion))
        );
        assert_eq!(engine.game().position().piece_at(sq(3, 5)), None);

        let igui_position = position_with(&[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Lion),
            (sq(6, 6), Color::White, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ]);
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!(
                    "setboard {} w\nusermove f6g7,g7f6\nquit\n",
                    board(&igui_position)
                )
            ),
            ""
        );
        assert_eq!(
            engine.game().position().piece_at(sq(5, 5)),
            Some(PieceCode::new(Color::Black, PieceKind::Lion))
        );
        assert_eq!(engine.game().position().piece_at(sq(6, 6)), None);
    }

    #[test]
    fn at_sign_jitto_uses_the_smallest_dense_origin() {
        let first = sq(5, 4);
        let second = sq(4, 5);
        let position = position_with(&[
            (sq(0, 0), Color::Black, PieceKind::King),
            (first, Color::Black, PieceKind::Lion),
            (second, Color::Black, PieceKind::Lion),
            (sq(11, 11), Color::White, PieceKind::King),
        ]);
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("setboard {} w\n", board(&position))
            ),
            ""
        );
        let candidates: Vec<_> = engine
            .game()
            .legal_moves()
            .into_iter()
            .filter(|mv| mv.mid.is_none() && mv.to == mv.from && !mv.promote)
            .collect();
        assert!(candidates.iter().any(|mv| mv.from == first));
        assert!(candidates.iter().any(|mv| mv.from == second));
        let expected = candidates
            .into_iter()
            .min_by_key(|mv| mv.from.dense_index())
            .unwrap();
        assert_eq!(expected.from, first);
        assert_eq!(representative_jitto(&engine), Some(expected));

        let mut expected_game = engine.game().clone();
        expected_game.play(expected).unwrap();
        assert_eq!(run(&mut protocol, &mut engine, "usermove @@@@\nquit\n"), "");
        assert_eq!(engine.game().ply_count(), 1);
        assert_eq!(engine.game().position(), expected_game.position());
    }

    #[test]
    fn promotion_suffix_is_applied() {
        let position = position_with(&[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(4, 7), Color::Black, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ]);
        let mut engine = engine(&[RuleCode::R1, RuleCode::E2]);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("setboard {} w\nusermove e8e9+\nquit\n", board(&position))
            ),
            ""
        );
        assert_eq!(
            engine.game().position().piece_at(sq(4, 8)),
            PieceCode::new(Color::Black, PieceKind::Pawn).promote()
        );
    }

    #[test]
    fn movement_and_repetition_illegal_moves_use_distinct_responses() {
        let mut movement_engine = engine(&[RuleCode::R1]);
        let mut movement_protocol = CecpProtocol::new(&movement_engine);
        assert_eq!(
            run(
                &mut movement_protocol,
                &mut movement_engine,
                "new\nusermove a1b1\nquit\n"
            ),
            "Illegal move: a1b1\n"
        );

        let base = "12/12/12/8k3/12/12/12/12/3K8/12/12/12";
        let mut repetition_engine = engine(&[RuleCode::R2, RuleCode::E2]);
        let mut repetition_protocol = CecpProtocol::new(&repetition_engine);
        let input = format!(
            "setboard {base} w\nusermove d4d5\nusermove i9i8\nusermove d5d4\nusermove i8i9\nquit\n"
        );
        assert_eq!(
            run(&mut repetition_protocol, &mut repetition_engine, &input),
            "Illegal move (repetition): i8i9\n"
        );
        assert_eq!(repetition_engine.game().ply_count(), 3);
    }

    #[test]
    fn royal_capture_emits_one_result_and_later_move_only_reports_lifecycle_error() {
        let board = "12/12/12/5k6/12/12/5R6/12/12/12/12/K11";
        let mut engine = engine(&[RuleCode::R1, RuleCode::E2]);
        let mut protocol = CecpProtocol::new(&engine);
        let input = format!("setboard {board} w\nusermove f6f9\nusermove f6f9\nquit\n");

        assert_eq!(
            run(&mut protocol, &mut engine, &input),
            concat!(
                "1-0 {royal capture}\n",
                "Error (command not legal now): usermove f6f9\n",
            )
        );
        assert_eq!(
            engine.status(),
            GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::RoyalCapture,
            })
        );
    }

    #[test]
    fn residual_command_errors_unknown_command_and_ping_match_the_transcript() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                concat!(
                    "time 100\notim 100\nlevel 40 5 0\nst 1\nsd 2\neasy\nhard\n",
                    "post\nnopost\nrandom\ncomputer\nname player\nhint\ndraw\nforce\n",
                    "go\nundo\nremove\nanalyze\nmystery value\nping token-0042\nquit\n",
                )
            ),
            concat!(
                "Error (command not legal now): go\n",
                "Error (command not supported): undo\n",
                "Error (command not supported): remove\n",
                "Error (command not supported): analyze\n",
                "Error (unknown command): mystery\n",
                "pong token-0042\n",
            )
        );
    }

    #[test]
    fn new_then_go_searches_and_applies_one_engine_move() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        let output = run(&mut protocol, &mut engine, "memory 1\nsd 1\nnew\ngo\n");

        assert!(output.lines().all(|line| line.starts_with("move ")));
        assert!(output.lines().any(|line| line.starts_with("move ")));
        assert_eq!(engine.game().ply_count(), 1);
    }

    #[test]
    fn usermove_automatically_starts_the_assigned_reply() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        let output = run(
            &mut protocol,
            &mut engine,
            "memory 1\nsd 1\nnew\nusermove g4g5\n",
        );

        assert!(output.lines().any(|line| line.starts_with("move ")));
        assert_eq!(engine.game().ply_count(), 2);
        assert_eq!(engine.game().position().side_to_move(), Color::Black);
    }

    #[test]
    fn force_accepts_both_sides_without_reply() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(
            run_queued(
                &mut protocol,
                &mut engine,
                concat!(
                    "memory 1\n",
                    "sd 256\n",
                    "new\n",
                    "go\n",
                    "force\n",
                    "usermove g4g5\n",
                    "usermove g10f8\n",
                ),
            ),
            ""
        );
        assert_eq!(engine.game().ply_count(), 2);
        assert!(protocol.force_mode);
        assert_eq!(protocol.engine_side, None);
    }

    #[test]
    fn go_leaves_force_and_assigns_the_current_side() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        let output = run(
            &mut protocol,
            &mut engine,
            "memory 1\nsd 1\nnew\nforce\nusermove g4g5\ngo\n",
        );

        assert!(output.lines().any(|line| line.starts_with("move ")));
        assert_eq!(engine.game().ply_count(), 2);
        assert!(!protocol.force_mode);
        assert_eq!(protocol.engine_side, Some(Color::White));
    }

    #[test]
    fn engine_finish_writes_move_then_one_result_and_rejects_later_go() {
        let position = position_with(&[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::White, PieceKind::King),
        ]);
        let mut engine = engine(&[RuleCode::R1, RuleCode::E2]);
        let mut protocol = CecpProtocol::new(&engine);
        let input = format!("memory 1\nsd 1\nsetboard {} w\ngo\ngo\n", board(&position));

        assert_eq!(
            run(&mut protocol, &mut engine, &input),
            concat!(
                "move f6f5\n",
                "1-0 {royal capture}\n",
                "Error (command not legal now): go\n",
            )
        );
    }

    #[test]
    fn move_now_stops_search_and_plays_the_current_best_move() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        let output = run_queued(&mut protocol, &mut engine, "memory 1\nsd 256\nnew\ngo\n?\n");

        assert!(output.lines().any(|line| line.starts_with("move ")));
        assert_eq!(engine.game().ply_count(), 1);
    }

    #[test]
    fn result_and_new_discard_running_search_without_a_move() {
        let mut result_engine = engine(&[RuleCode::R1]);
        let mut result_protocol = CecpProtocol::new(&result_engine);
        let result_output = run_queued(
            &mut result_protocol,
            &mut result_engine,
            "memory 1\nsd 256\nnew\ngo\nresult 1-0 {external}\n",
        );
        assert!(!result_output.contains("move "));
        assert_eq!(result_engine.lifecycle(), EngineLifecycle::AwaitingStart);

        let mut new_engine = engine(&[RuleCode::R1]);
        let mut new_protocol = CecpProtocol::new(&new_engine);
        let new_output = run_queued(
            &mut new_protocol,
            &mut new_engine,
            "memory 1\nsd 256\nnew\ngo\nnew\n",
        );
        assert!(!new_output.contains("move "));
        assert_eq!(new_engine.lifecycle(), EngineLifecycle::InGame);
        assert_eq!(new_engine.game().ply_count(), 0);
    }

    #[test]
    fn ping_during_search_is_answered_after_the_move() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        let output = run_queued(
            &mut protocol,
            &mut engine,
            "memory 1\nsd 1\nnew\ngo\nping token-0042\n",
        );
        let lines: Vec<_> = output.lines().collect();
        let last_move = lines
            .iter()
            .rposition(|line| line.starts_with("move "))
            .unwrap();
        let pong = lines
            .iter()
            .position(|line| *line == "pong token-0042")
            .unwrap();

        assert!(last_move < pong);
    }

    #[test]
    fn time_commands_normalize_units_and_follow_the_assigned_side() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        assert_eq!(parse_level_base_milliseconds("5"), Some(300_000));
        assert_eq!(parse_level_base_milliseconds("5:30"), Some(330_000));

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "memory 1\nlevel 40 5:30 2.5\nnew\n",
            ),
            ""
        );
        assert_eq!(protocol.engine_side, Some(Color::White));
        assert_eq!(protocol.time_control.engine_remaining_ms, Some(330_000));
        assert_eq!(protocol.time_control.opponent_remaining_ms, Some(330_000));
        assert_eq!(protocol.time_control.increment_ms, 2_500);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "time 123\notim 456\nst 3.25\nsd 7\n",
            ),
            ""
        );
        assert_eq!(protocol.time_control.opponent_remaining_ms, Some(4_560));
        assert_eq!(
            protocol.time_control.search_limits(),
            Some(SearchLimits {
                depth: Some(7),
                nodes: None,
                movetime_ms: Some(3_250),
                clock: Some(ClockLimits {
                    remaining_ms: 1_230,
                    increment_ms: 2_500,
                    byoyomi_ms: 0,
                }),
                infinite: false,
            })
        );

        let mut output = Vec::new();
        let LineAction::Start(search) = protocol
            .handle_idle_line(&mut engine, "go", &mut output)
            .unwrap()
        else {
            panic!("go must start a search");
        };
        assert_eq!(protocol.engine_side, Some(Color::Black));
        let mut active = Some(*search);
        protocol.discard_search(&mut active).unwrap();
    }

    #[test]
    fn memory_resizes_the_idle_transposition_table_and_searches_afterward() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        let output = run(
            &mut protocol,
            &mut engine,
            "memory 1\nmemory 2\nsd 1\nnew\ngo\n",
        );

        assert!(protocol.transposition_table.is_some());
        assert!(output.lines().any(|line| line.starts_with("move ")));
    }

    #[test]
    fn invalid_setboard_preserves_state_and_extra_fields_are_ignored() {
        let mut engine = engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);
        assert_eq!(run(&mut protocol, &mut engine, "new\n"), "");
        let position_before = engine.game().position().clone();
        let invalid_board = "13/12/12/12/12/12/12/12/12/12/12/12";

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("setboard {INITIAL_BOARD} x\nsetboard {invalid_board} w\nsetboard\n")
            ),
            concat!(
                "tellusererror Illegal position\n",
                "tellusererror Illegal position\n",
                "tellusererror Illegal position\n",
            )
        );
        assert_eq!(engine.game().position(), &position_before);

        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("setboard {INITIAL_BOARD} b invalid castling fields are ignored\nquit\n")
            ),
            ""
        );
        assert_eq!(engine.game().position().side_to_move(), Color::White);
        assert_eq!(
            engine.game().rules(),
            Rules::from_codes(&[RuleCode::R1]).unwrap()
        );
    }
}
