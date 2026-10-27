//! Universal Shogi Interface（USI）のアダプター。
//!
//! 待機中のコマンドは次の表に従う。入力行の第1語で分類し、空行は無視する。
//!
//! | コマンド | 応答と処理 |
//! | --- | --- |
//! | `usi` | `UsiProtocol::write_handshake`の握手列を返す。 |
//! | `isready` | 必要なら置換表を確保し、`readyok`を返す。 |
//! | `setoption` | `UsiProtocol::handle_setoption`で設定を検査する。 |
//! | `usinewgame` | [`EngineCommand::NewGame`]を適用する。 |
//! | `position` | `UsiProtocol::handle_position`で局面を同期する。 |
//! | `gameover` | [`EngineCommand::EndGame`]を適用する。 |
//! | `moves`、`state` | `handle_moves`と`handle_state`の独自照会を処理する。 |
//! | `d`、`eval`、`tt` | `handle_diagnostic`の診断を出力する。 |
//! | 引数に`mate`を含む`go` | `checkmate notimplemented`を1行返す。 |
//! | それ以外の`go` | `parse_go_config`で手番側と相手側の時計を検査し、`start_go`で探索を開始する。 |
//! | `quit` | 無応答で終了する。 |
//! | `debug`、`register`、`stop` | 無応答で無視する。 |
//! | `ponderhit` | `info string error: ponderhit requires an active ponder search`を返す。 |
//! | それ以外 | `info string error: unknown command <第1語>`を返す。 |
//!
//! コマンドの拒否は`info string error: <内容>`の1行で通知し、対局や設定への適用を行わない。
//! ただし拒否された`position`は同期済みの扱いを解除する。
//! 診断のコマンドと未知コマンドの扱いの理由は
//! `docs/adr/show-position-evaluation-and-tt-as-usi-commands.md`にある。
//! `position startpos`は中将棋の初期局面を指す。
//! 本エンジンは中将棋だけを扱い、Lishogi-Botも`startpos`を送るためである。
//! 接続の根拠は`docs/research/protocols/usi-lishogi.md`の「lishogi-botブリッジ」にある。
//! USIに終局の裁定をエンジンから知らせる標準手段がないため、自発的な裁定通知は出さない。
//! 理由は`docs/adr/keep-searched-moves-out-of-the-usi-game.md`にある。
//!
//! 探索中の`stop`は探索を止めて`bestmove`を返す。
//! `gameover`と`quit`は探索を止めて結果を捨て、`bestmove`を出さずに通常の処理へ戻す。
//! 将棋所は先読み中に相手が投了すると`stop`なしで`gameover`を送るが、対局待ちへ移る通知なので着手を返さない。
//! 探索中の`go`は`info string error: go is already running`で拒否し、`ponderhit`は`UsiProtocol::ponderhit`で処理する。
//! それ以外の入力は`isready`、`setoption`、`position`も含めて待ち行列に積み、探索の解放後に受信順で処理する。
//! 入力が閉じた場合は有限探索の完了を待って`bestmove`を出すが、
//! 結果を保留する`go infinite`と`go ponder`は停止も的中も届かないため捨てる。

use std::collections::VecDeque;
use std::io::{self, BufRead, Write};
#[cfg(feature = "tuning")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use crate::eval::nnue;
use crate::search::{
    self, ClockLimits, SearchEvent, SearchHandle, SearchLimits, SearchOptionKind,
    SearchOptionValue, SearchSnapshot, Searcher, StopReason, TranspositionTable,
};
use minase_core::game::{DrawReason, Game, GameResult, GameStatus, WinReason};
use minase_core::mv::Move;
use minase_core::notation::sfen::{
    SetupPosition, parse_extended_sfen, square_to_text, to_extended_sfen, to_sfen,
};
use minase_core::notation::usi;
use minase_core::piece::{Color, PieceCode, PieceKind};
use minase_core::position::Position;
use minase_core::rules::parse_rule_set;
use minase_core::{MoveGenerator, Square};

use super::Protocol;
use super::engine::{
    Engine, EngineCommand, EngineLifecycle, EngineReply, RejectReason, canonical_rules_text,
};

/// 投了閾値の既定値20,000 cp。判定は[`UsiProtocol::should_resign`]に従う。
/// 通常の評価値がほとんど6,000 cpを超えないことを踏まえ、詰まされる読みを対象にする。
/// 理由は`docs/adr/decide-resignation-in-the-usi-layer.md`にある。
const DEFAULT_RESIGN_VALUE: i32 = 20_000;

/// プロセス内で一度でもgoを受信したら調整係数を固定する。
#[cfg(feature = "tuning")]
static TUNING_LOCKED: AtomicBool = AtomicBool::new(false);

/// lishogi系拡張を含むUSIプロトコル。
pub struct UsiProtocol {
    /// `option`宣言のdefault値に使う起動時規則の正準表記。
    startup_rules_text: String,
    /// 対局内で持ち越す探索器。探索スレッドへ貸し出している間は`None`。
    searcher: Option<Searcher>,
    /// 最後の`position`コマンドが受理され、局面が同期済みかどうか。
    position_synchronized: bool,
    /// 最後に受理した`position`の開始局面と着手のトークン列。
    accepted_position: Option<AcceptedPosition>,
    /// 次に開始する探索へ割り当てる識別子。
    next_search_id: u64,
    /// 根の手番側から見たcpに適用する投了閾値。条件は[`Self::should_resign`]に従う。
    resign_value: i32,
    /// 根が先手番のときに使う、根の手番側から見た引き分けの評価値(cp)。
    draw_value_black: i32,
    /// 根が後手番のときに使う、根の手番側から見た引き分けの評価値(cp)。
    draw_value_white: i32,
    /// 秒読みつき時計の締切に残す余裕(ms)。
    byoyomi_margin_ms: u64,
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
    rules: minase_core::Rules,
    /// `go infinite`による探索かどうか。
    infinite: bool,
    /// 的中または停止まで結果を保留する先読みか。
    ponder: bool,
    /// 最後に`info`として出力した完了深さ。
    last_info_depth: Option<u32>,
}

/// 実行中の探索、または結果を保持して停止や的中を待つ状態。
///
/// `go infinite`は`stop`まで、`go ponder`は`stop`か`ponderhit`まで`bestmove`を保留する。
/// USIの規定により、詰みを読み切っても先に返さない。
/// 深さ上限などで先に探索が終われば`AwaitingStop`へ移り、最善手、検査済みの予想手、
/// 評価値、完了深さ、および実際の停止理由を保持する。
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
            searcher: Some(Searcher::alpha_beta()),
            position_synchronized: false,
            accepted_position: None,
            next_search_id: 1,
            resign_value: DEFAULT_RESIGN_VALUE,
            draw_value_black: search::DRAW_SCORE,
            draw_value_white: search::DRAW_SCORE,
            byoyomi_margin_ms: ClockLimits::DEFAULT_BYOYOMI_MARGIN_MS,
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
                self.searcher
                    .as_mut()
                    .expect("idle protocol must own its searcher")
                    .prepare();
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
            "d" | "eval" | "tt" => self.handle_diagnostic(engine, command, output)?,
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
            "debug" | "register" | "stop" => {}
            _ => write_error(output, &format!("unknown command {command}"))?,
        }
        output.flush()?;
        Ok(LineAction::Continue)
    }

    /// 同期済みの対局から探索を開始する。
    ///
    /// 対局中でなければ`go requires an active game`、未同期なら`go requires a synchronized position`を
    /// `info string error: `に続けて出し、`bestmove`は返さない。
    /// 引数の検査と単位変換は[`parse_go_config`]、時間予算の計算は[`crate::search`]が担う。
    /// 探索は対局の複製から作るスナップショットで行い、返す手を`Engine`へ適用しない。
    /// 理由は`docs/adr/keep-searched-moves-out-of-the-usi-game.md`にある。
    /// 引き分けの評価値は、根が先手番なら`DrawValueBlack`、後手番なら`DrawValueWhite`を使う。
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
        let config = match parse_go_config(
            tokens,
            game.position().side_to_move(),
            engine.ply(),
            self.byoyomi_margin_ms,
        ) {
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
        let snapshot =
            match snapshot.with_draw_value(self.root_draw_value(game.position().side_to_move())) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    write_error(output, &error.to_string())?;
                    return Ok(None);
                }
            };
        let search_id = self.next_search_id;
        self.next_search_id = self.next_search_id.wrapping_add(1);
        let infinite = config.is_infinite();
        let searcher = self
            .searcher
            .take()
            .expect("idle protocol must own its searcher");
        let handle = match searcher.start(snapshot, config, search_id, tokens.contains(&"ponder")) {
            Ok(handle) => handle,
            Err(failure) => {
                self.searcher = Some(failure.searcher);
                write_error(output, &failure.error.to_string())?;
                return Ok(None);
            }
        };
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

    /// 入力スレッドが送る改行なしのUSI入力と探索イベントを並行して処理する。
    ///
    /// 探索中は入力を優先し、入力がなければ探索通知を刈り取る。どちらもなければ10 ms待つ。
    /// 送信側が閉じたら有限探索は完了を待つが、結果を保留する無限探索と先読みは捨てる。
    /// 入力スレッドの読み取りエラーでは探索を捨ててエラーを返す。
    /// コマンドの処理順は本モジュールの探索中の入力の契約に従う。
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
        self.searcher = Some(join_search(handle)?);
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
                write_info(output, context, depth, score, nodes, elapsed, &pv, false)?;
                context.last_info_depth = Some(depth);
                Ok(())
            }
            SearchEvent::Finished {
                partial_score,
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
                self.searcher = Some(join_search(handle)?);
                write_final_info_if_deeper(
                    output,
                    &mut context,
                    depth,
                    score,
                    nodes,
                    elapsed,
                    &pv,
                    partial_score,
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

    /// 先読みの結果保留を解除し、同じ探索を続ける。
    ///
    /// 実行中なら[`SearchHandle::ponderhit`]を通知し、この時点から時間予算を測る。
    /// 以後の完了は通常探索と同じく直ちに`bestmove`を出し、探索を作り直さない。
    /// すでに完了して保留中なら、保持していた結果をその場で出す。
    /// 先読み中でなければ`info string error: ponderhit requires an active ponder search`を返す。
    /// 理由は`docs/adr/measure-budget-from-ponderhit-and-add-a-quarter-after-pondering.md`にある。
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
                            write_info(output, &context, depth, score, nodes, elapsed, &pv, false)?;
                            context.last_info_depth = Some(depth);
                        }
                        SearchEvent::Finished {
                            partial_score,
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
                                partial_score,
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
                self.searcher = Some(join_search(handle)?);
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

    /// 根の手番に対応する引き分けの評価値を返す。
    fn root_draw_value(&self, side: Color) -> i32 {
        match side {
            Color::Black => self.draw_value_black,
            Color::White => self.draw_value_white,
        }
    }

    /// 完了深さが1以上で、完了した探索値が`-resign_value`以下なら投了する。
    ///
    /// 評価値は根の手番側から見たcpであり、詰み帯も同じ比較を使う。
    /// 負の専用の帯は`ResignValue`によらず投了から除く。
    /// 帯の値は1回の再現で判定する近似に基づく見込みであり、実際の裁定より早く負けを確定させないためである。
    /// 深さ0の静的評価や、途中結果として採用した手の下界は判定に使わない。
    /// 通常の完了、的中後の完了、および`stop`による無限探索や外れた先読みの解放に適用する。
    /// 投了しても`Engine`の対局状態は変えない。
    /// 既定の閾値は[`DEFAULT_RESIGN_VALUE`]に従う。
    fn should_resign(&self, score: i32, depth: u32) -> bool {
        depth >= 1 && score <= -self.resign_value && !search::in_repetition_band(score)
    }

    /// `bestmove`を出力せずに探索を破棄する。`gameover`・`quit`と入力断で使う。
    fn discard_search(&mut self, active: &mut Option<ActiveSearch>) -> io::Result<()> {
        let Some(search) = active.take() else {
            return Ok(());
        };
        if let ActiveSearch::Running { handle, .. } = search {
            handle.request_stop();
            self.searcher = Some(join_search(handle)?);
        }
        Ok(())
    }

    /// 起動時の規則を含む握手列を、次の順に出力する。
    ///
    /// ```text
    /// id name minase <CARGO_PKG_VERSION>
    /// id author stepney141
    /// option name RuleSet type string default <起動時の--rulesの正準表記>
    /// option name USI_Variant type string default chushogi
    /// option name USI_Hash type spin default 256 min 1 max 65536
    /// option name Threads type spin default 1 min 1 max 256
    /// option name ResignValue type spin default 20000 min 1 max 99999
    /// option name ByoyomiMargin type spin default 30 min 0 max 60000
    /// option name DrawValueBlack type spin default 0 min -28999 max 28999
    /// option name DrawValueWhite type spin default 0 min -28999 max 28999
    /// ```
    ///
    /// `tuning`ビルドだけ係数表の順に`option name Tune_<名前> type spin default <既定値> min <最小> max <最大>`を続け、
    /// 最後に`usiok`を出す。オプション名と範囲の変更は互換性を壊す変更として扱う。
    /// 時間予算が値に依存しないため`USI_Ponder`は宣言しない。
    /// 理由は`docs/adr/measure-budget-from-ponderhit-and-add-a-quarter-after-pondering.md`にある。
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
        for option in self
            .searcher
            .as_ref()
            .expect("idle protocol must own its searcher")
            .options()
        {
            match option.kind {
                SearchOptionKind::Spin { default, min, max } => writeln!(
                    output,
                    "option name {} type spin default {default} min {min} max {max}",
                    option.name
                )?,
            }
        }
        writeln!(
            output,
            "option name ResignValue type spin default {DEFAULT_RESIGN_VALUE} min 1 max 99999"
        )?;
        writeln!(
            output,
            "option name ByoyomiMargin type spin default {} min 0 max 60000",
            ClockLimits::DEFAULT_BYOYOMI_MARGIN_MS
        )?;
        for name in ["DrawValueBlack", "DrawValueWhite"] {
            writeln!(
                output,
                "option name {name} type spin default {} min {} max {}",
                search::DRAW_SCORE,
                -search::DRAW_VALUE_LIMIT,
                search::DRAW_VALUE_LIMIT
            )?;
        }
        #[cfg(feature = "tuning")]
        for &(name, default, min, max) in search::alphabeta::params::PARAMETERS {
            writeln!(
                output,
                "option name Tune_{name} type spin default {default} min {min} max {max}"
            )?;
        }
        writeln!(output, "usiok")
    }

    /// `setoption name <名前> value <値>`の設定を検査して適用する。
    ///
    /// `name`と`value`、および標準のオプション名は大文字小文字を区別しない。
    /// 各キーワードの直後の1語を名前と値として読み、名前がなければ無応答で無視する。
    /// `RuleSet`は[`parse_rule_set`]の規則コードのコンマ区切り、または
    /// `engine-default`か`lishogi`の単独指定として読み、次局の規則だけを変える。
    /// 受理時の置換表、history表、および局面トークン履歴の消去は[`Self::apply_silent`]に従う。
    /// `USI_Variant`は大小文字を区別せず`chushogi`だけを受理する。
    /// `USI_Hash`は1〜65,536 MiBで置換表を確保または作り直し、history表を空にする。
    /// `Threads`は1〜256で、変更時はhistory表をその数で作り直し、置換表は保つ。
    /// `ResignValue`はASCII数字だけの1〜99,999、`ByoyomiMargin`はASCII数字だけの0〜60,000 msである。
    /// この2値は先頭ゼロを許すが、符号、小数点、指数表記を許さない。
    /// `DrawValueBlack`と`DrawValueWhite`は先頭の`-`を1つ許すASCII数字の列で、絶対値が28,999以下である。
    /// 先頭ゼロを許す。
    /// 値が変わった受理では置換表とhistory表を空にする。
    /// 置換表のキーは根の手番とオプションの値を区別せず、引き分けの値を含めて求めた親の値が次の探索で再利用されるためである。
    ///
    /// 値の欠落または不正では直前の設定を保ち、次の内容を`info string error: `に続ける。
    ///
    /// | オプション | 欠落時 | 不正時 |
    /// | --- | --- | --- |
    /// | `RuleSet` | `RuleSet requires a value`を返す。 | 規則パーサまたは状態機械の拒否理由を返す。 |
    /// | `USI_Variant` | `USI_Variant requires a value`を返す。 | `unsupported USI_Variant value '<値>'`を返す。 |
    /// | `USI_Hash` | `USI_Hash requires a value`を返す。 | `USI_Hash must be an integer from 1 to 65536`を返す。 |
    /// | `Threads` | `Threads requires a value`を返す。 | `Threads must be an integer from 1 to 256`を返す。 |
    /// | `ResignValue` | `missing ResignValue value`を返す。 | `invalid ResignValue value`を返す。 |
    /// | `ByoyomiMargin` | `missing ByoyomiMargin value`を返す。 | `invalid ByoyomiMargin value`を返す。 |
    /// | `DrawValueBlack` | `missing DrawValueBlack value`を返す。 | `invalid DrawValueBlack value`を返す。 |
    /// | `DrawValueWhite` | `missing DrawValueWhite value`を返す。 | `invalid DrawValueWhite value`を返す。 |
    ///
    /// `tuning`ビルドの`Tune_`名は大文字小文字を区別し、最初の`go`以後は変更を拒否する。
    /// 未知の名前は`USI_Ponder`も含め無応答で無視する。
    /// 探索中の設定は待ち行列へ入り、探索終了後に適用されるため次の探索から効く。
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
        } else if let Some(option) = self
            .searcher
            .as_ref()
            .expect("idle protocol must own its searcher")
            .options()
            .into_iter()
            .find(|option| name.eq_ignore_ascii_case(option.name))
        {
            let name = option.name;
            let Some(value) = value else {
                return write_error(output, &format!("{name} requires a value"));
            };
            let value = match option.kind {
                SearchOptionKind::Spin { min, max, .. } => {
                    let Some(value) = parse_spin(value, min, max) else {
                        return write_error(
                            output,
                            &format!("{name} must be an integer from {min} to {max}"),
                        );
                    };
                    SearchOptionValue::Spin(value)
                }
            };
            match self
                .searcher
                .as_mut()
                .expect("idle protocol must own its searcher")
                .set_option(name, value)
            {
                Ok(()) => Ok(()),
                Err(error) => write_error(output, &error.to_string()),
            }
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
        } else if name.eq_ignore_ascii_case("ByoyomiMargin") {
            let Some(value) = value else {
                return write_error(output, "missing ByoyomiMargin value");
            };
            let Some(value) = value
                .bytes()
                .all(|byte| byte.is_ascii_digit())
                .then(|| value.parse::<u64>().ok())
                .flatten()
                .filter(|value| *value <= 60_000)
            else {
                return write_error(output, "invalid ByoyomiMargin value");
            };
            self.byoyomi_margin_ms = value;
            Ok(())
        } else if name.eq_ignore_ascii_case("DrawValueBlack")
            || name.eq_ignore_ascii_case("DrawValueWhite")
        {
            let (name, current) = if name.eq_ignore_ascii_case("DrawValueBlack") {
                ("DrawValueBlack", &mut self.draw_value_black)
            } else {
                ("DrawValueWhite", &mut self.draw_value_white)
            };
            let Some(value) = value else {
                return write_error(output, &format!("missing {name} value"));
            };
            let Some(value) = parse_draw_value(value) else {
                return write_error(output, &format!("invalid {name} value"));
            };
            if *current != value {
                *current = value;
                self.searcher
                    .as_mut()
                    .expect("idle protocol must own its searcher")
                    .clear();
            }
            Ok(())
        } else {
            Ok(())
        }
    }

    /// `position startpos [moves <着手>...]`または
    /// `position sfen <4欄または5欄の拡張SFEN> [moves <着手>...]`で局面を同期する。
    ///
    /// `startpos`の余分な欄は`position startpos has unexpected fields`で拒否し、
    /// 種別の欠落や不正は`position requires startpos or sfen`で拒否する。
    /// SFENは[`Engine::position_rules`]に従い、開始前なら次局、それ以外なら現局の規則で解析する。
    ///
    /// 対局中で開始局面のトークン列が前回と同じなら、着手列の差分を使う。
    /// 着手数が同じで末尾1手だけが異なり、直前の対局があれば[`EngineCommand::ReplaceLastMove`]を選ぶ。
    /// 前回の列を接頭辞に持つ場合は、追加が0手でも[`EngineCommand::ExtendPosition`]を選ぶ。
    /// それ以外は[`EngineCommand::SetPosition`]で全手を再適用する。
    /// 理由は`docs/adr/apply-position-by-token-difference.md`にある。
    ///
    /// 処理開始時に未同期とし、受理した場合だけ同期済みにしてトークン履歴を更新する。
    /// 拒否時は`Engine`を保ち、不合法手なら`info string error: illegal move '<表記>': <原因>`を返す。
    /// 未同期の間は[`Self::start_go`]が探索を拒否する。
    /// 理由は`docs/adr/keep-searched-moves-out-of-the-usi-game.md`にある。
    /// SFENは反復履歴を運ばないため、着手列のない途中局面では履歴を空から始め、補完しない。
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

    /// 独自照会`moves`へ、`moves <着手1> <着手2> ...`の1行を返す。
    ///
    /// [`Game::legal_moves`]の全要素を、2段階移動、居喰い、じっと、成りも含め`position`と同じUSI表記で出す。
    /// 順序は契約に含めない。対局中以外は`info string error: moves requires an active game`を返す。
    /// CECPに同等の照会はなく、Lishogi-BotとXBoardの経路も使わない。
    /// 用途は`docs/research/protocols/usi-lishogi.md`の「minase固有のUSI拡張」にある。
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

    /// 独自照会へ`state rules <正準規則列> board <2欄SFEN> status <状態>`の1行を返す。
    ///
    /// 対局中と終局後に応答し、起動直後や`gameover`後の開始待ちでは
    /// `info string error: state requires an active or finished game`を返す。
    /// 状態の表記と表現できない裁定は[`state_status_text`]に従う。
    /// 2欄SFENは先獅子の状態、成り権の保留、および反復履歴を運ばないため、
    /// GUIは着手列を保持して`position`を再送する。
    /// USIに標準の終局通知がないため、自作GUIがminaseを審判として使う際の照会である。
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

    /// 対局中または終局後に、独自照会`d`、`eval`、`tt`を処理する。
    ///
    /// 対局開始前は`info string error:`の1行で拒否する。
    /// 受理済みの局面を表示し、後続の`position`が失敗して未同期でも、保持している局面を照会できる。
    /// 探索中の照会は待ち行列に入り、探索終了後に処理する。
    /// `eval`と`tt`は各行に`info string`を付け、最後を`info string end`とする。
    /// `d`は接頭辞のない行を出し、最後の行を`rights-zobrist`とする。
    /// 表示は全行を組み立ててから出力し、構築の失敗時はエラー1行だけで終端行を付けない。
    /// 未知コマンドとUSIの既知の無応答コマンドは、本モジュールのコマンド表に従う。
    /// 理由は`docs/adr/show-position-evaluation-and-tt-as-usi-commands.md`にある。
    fn handle_diagnostic(
        &self,
        engine: &Engine,
        command: &str,
        output: &mut dyn Write,
    ) -> io::Result<()> {
        if engine.lifecycle() == EngineLifecycle::AwaitingStart {
            return write_error(
                output,
                &format!("{command} requires an active or finished game"),
            );
        }
        let lines = match command {
            "d" => diagnostic_position(engine),
            "eval" => diagnostic_evaluation(engine.game().position()),
            "tt" => self
                .searcher
                .as_ref()
                .expect("idle protocol must own its searcher")
                .transposition_table()
                .ok_or_else(|| "tt requires an allocated transposition table".to_owned())
                .and_then(|table| diagnostic_table(engine, table)),
            _ => unreachable!("only diagnostic commands are dispatched here"),
        };
        match lines {
            Ok(lines) if command == "d" => {
                for line in lines {
                    writeln!(output, "{line}")?;
                }
                Ok(())
            }
            Ok(lines) => {
                for line in lines {
                    writeln!(output, "info string {line}")?;
                }
                writeln!(output, "info string end")
            }
            Err(error) => write_error(output, &error),
        }
    }

    /// 受理時に何も出力しない対局コマンドを適用し、関連する探索状態を消去する。
    ///
    /// `SetRules`と`NewGame`では置換表とhistory表を空にし、前局や別規則の評価を持ち越さない。
    /// `EndGame`では両表を保つ。
    /// この3コマンドは`position`の差分履歴を捨て、`NewGame`と`EndGame`は局面を未同期にする。
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
                if clears_transposition_table {
                    self.searcher
                        .as_mut()
                        .expect("idle protocol must own its searcher")
                        .clear();
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

/// USIの`go`引数を単位の正規化を伴う探索制限へ変換する。
///
/// `depth`は1〜256、`nodes`は1以上、`movetime`、`btime`、`wtime`、`binc`、`winc`、
/// `byoyomi`は整数ミリ秒として解析する。時間の負値は拒否し、`movetime`は0も拒否する。
/// `ponder`と`infinite`を含め、同じ引数の重複はすべて拒否する。
/// 未知の語は`unsupported go argument '<語>'`、引数なしは`go requires depth or nodes`とする。
/// `infinite`は他の制限とも`ponder`とも併用できず、制限のない`go ponder`も拒否する。
/// 深さの範囲は[`SearchLimits::new`]でも検査する。
///
/// 時計は手番側の残り時間と加算を選び、省略値を0、秒読みを共通値とする。
/// 手番でない側の残り時間は`Option`のまま保持する。秒読みが0で`movetime`を併用しない時計では、
/// 相手との残り時間の差による目安時間の補正に必要なため、省略を拒否する。
/// 秒読みが正の時計と`movetime`を併用する時計ではこの補正を使わないため、省略を受理する。
/// 手番側の時計の3値がすべて0なら拒否し、[`Engine::ply`]と`ByoyomiMargin`を[`ClockLimits`]へ渡す。
/// 先読みの時計引数は、的中後に自分が使える時間としてそのまま渡す。
fn parse_go_config(
    tokens: &[&str],
    side_to_move: Color,
    ply: u32,
    byoyomi_margin_ms: u64,
) -> Result<SearchLimits, String> {
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
            let (remaining_ms, increment_ms, opponent_remaining_ms) = match side_to_move {
                Color::Black => (btime.unwrap_or(0), binc.unwrap_or(0), wtime),
                Color::White => (wtime.unwrap_or(0), winc.unwrap_or(0), btime),
            };
            ClockLimits::new(
                remaining_ms,
                increment_ms,
                byoyomi.unwrap_or(0),
                opponent_remaining_ms,
                ply,
            )
            .map(|clock| clock.with_byoyomi_margin_ms(byoyomi_margin_ms))
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
    rules: minase_core::MoveRules,
) -> Result<SetupPosition, minase_core::notation::sfen::SfenError> {
    parse_extended_sfen(&fields.join(" "), rules)
}

impl Protocol for UsiProtocol {
    /// 台本入力を逐次処理する。
    ///
    /// 有限の`go`はその場で探索の完了を待ち、`bestmove`を出す。
    /// `go infinite`と`go ponder`は探索を持ち越して次の入力を読み、
    /// 先読みに`ponderhit`を受けたら完了まで待って`bestmove`を出す。
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
    /// [`ActiveSearch`]の契約に従い、停止または的中まで`bestmove`を保留するかを返す。
    fn withholds_bestmove(&self) -> bool {
        match self {
            Self::Running { context, .. } | Self::AwaitingStop { context, .. } => {
                context.infinite || context.ponder
            }
        }
    }
}

/// 探索スレッドの終了を待ち、貸し出していた探索器を回収する。
fn join_search(handle: SearchHandle) -> io::Result<Searcher> {
    handle
        .join()
        .map_err(|error| io::Error::other(error.to_string()))
}

/// 採用した読み筋の2手目だけから、検査済みの予想手を得る。
///
/// 読み筋が1手以下なら付けない。対局の複製へ最善手Xと予想手Yを順に適用し、
/// 両方が受理され、Yの後も対局が続く場合だけXの後の局面での表記を返す。
/// 反復規則R2とR3で禁止されるYや、Yで終局する場合は除く。
/// これによりGUIが送る`position`や`go ponder`を自ら拒否し、Lishogi-Botの待ちが終わらなくなることを防ぐ。
/// GUIの先読み設定によらず予想手を出力する。
/// 理由は`docs/adr/take-ponder-move-from-the-pv.md`にある。
fn validated_ponder_move(game: &Game, best_move: Move, pv: &[Move]) -> Option<String> {
    let &prediction = pv.get(1)?;
    let mut game = game.clone();
    game.play(best_move).ok()?;
    let position = game.position().clone();
    (game.play(prediction).ok()? == GameStatus::Ongoing)
        .then(|| usi::text_generated(&position, prediction))
}

/// `info string stop <理由>`に続けて`bestmove <着手>[ ponder <予想手>]`を出す。
///
/// 投了時は`bestmove resign`とし、予想手を付けない。
/// 停止理由は[`stop_reason_text`]で表し、先読み用に上書きしない。
/// 理由の行は対局ハーネスが手ごとの停止理由を保存するために使う。
/// 正常に開始して結果を返す`go`ごとに`bestmove`は1回であり、
/// `gameover`、`quit`、または入力の切断で捨てた探索には出さない。
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

/// 探索を実際に終わらせた[`StopReason`]を固定語へ1対1に写す。
/// 指定深さの完了は`depth`、ノード上限は`nodes`、softとhardの時間制限は`soft`と`hard`、
/// 外部の停止要求は`external`とする。
const fn stop_reason_text(reason: StopReason) -> &'static str {
    match reason {
        StopReason::DepthCompleted => "depth",
        StopReason::NodeLimit => "nodes",
        StopReason::SoftLimit => "soft",
        StopReason::HardLimit => "hard",
        StopReason::ExternalStop => "external",
    }
}

/// `info depth <d> score <評価> nodes <n> nps <nps> time <ms> pv <着手>...`を出す。
///
/// 評価は[`score_text`]の`cp N`または符号つきの詰み手数であり、途中結果の下界には`lowerbound`を添える。
/// 主ワーカーの完了反復ごとに出力する。経過時間は探索開始からのミリ秒で、先読み時間を含む。
/// 経過時間0なら毎秒ノード数（nps）は0とする。
/// 読み筋は探索開始時の局面から順に進めながらlishogi系のUSI表記へ変換する。
/// 完了時の追加出力は[`write_final_info_if_deeper`]に従う。
#[allow(clippy::too_many_arguments)]
fn write_info(
    output: &mut dyn Write,
    context: &SearchContext,
    depth: u32,
    score: i32,
    nodes: u64,
    elapsed: Duration,
    pv: &[Move],
    lowerbound: bool,
) -> io::Result<()> {
    let score = score_text(score);
    let bound = if lowerbound { " lowerbound" } else { "" };
    write!(
        output,
        "info depth {depth} score {score}{bound} nodes {nodes} nps {} time {} pv",
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

/// 完了した採用結果が最後の進捗より深ければ、その結果を1行出す。
///
/// 補助ワーカーの深い結果もこの経路で通知する。深さ1の完了前に停止し、途中結果もなければ出さない。
/// 途中結果を採用した場合は、`info string partial`に続けて完了深さ+1の情報を出し、
/// 評価値を`lowerbound`つきの下界として示す。完了した反復の評価値とは区別する。
#[allow(clippy::too_many_arguments)]
fn write_final_info_if_deeper(
    output: &mut dyn Write,
    context: &mut SearchContext,
    depth: u32,
    score: i32,
    nodes: u64,
    elapsed: Duration,
    pv: &[Move],
    partial_score: Option<i32>,
) -> io::Result<()> {
    // 途中結果は下界として通知し、完了した反復の評価値と区別する。
    if let Some(partial_score) = partial_score {
        writeln!(output, "info string partial")?;
        return write_info(
            output,
            context,
            depth + 1,
            partial_score,
            nodes,
            elapsed,
            pv,
            true,
        );
    }
    if depth <= context.last_info_depth.unwrap_or(0) {
        return Ok(());
    }
    write_info(output, context, depth, score, nodes, elapsed, pv, false)?;
    context.last_info_depth = Some(depth);
    Ok(())
}

/// 評価値を`info score`用の文字列へ変換する。
///
/// 詰み帯では`MATE − |score|`が王駒の捕獲までの手数を表す。
/// 中将棋では詰んだ後も詰まされた側の1手と王駒を取る1手が続くため、2を引いて0以上に切り詰める。
/// 0手にも勝敗の符号を付ける`mate +0`と`mate -0`は、やねうら王とShogiGUIの慣行に倣う。
/// 詰み帯の外は内部の評価値を換算せず`cp`として出力する。
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

/// 先頭の`-`を1つ許すASCII数字の列を、引き分けの評価値の範囲内で解析する。
fn parse_draw_value(value: &str) -> Option<i32> {
    let digits = match value.strip_prefix('-') {
        Some(digits) => digits,
        None => value,
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value
        .parse::<i32>()
        .ok()
        .filter(|value| (-search::DRAW_VALUE_LIMIT..=search::DRAW_VALUE_LIMIT).contains(value))
}

/// 宣言された範囲の整数オプションを解析する。
fn parse_spin(value: &str, min: usize, max: usize) -> Option<usize> {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| (min..=max).contains(value))
}

/// `moves`以降の指し手列を解析し、局面を進めながら検証する。
///
/// 不合法な指し手が現れても解析済みの列は返し、拒否理由の報告用に
/// その表記を保持する。合否の最終判定は状態機械側の再適用が行う。
fn parse_moves(
    setup: &SetupPosition,
    rules: minase_core::Rules,
    move_tokens: &[&str],
) -> Result<ParsedMoves, String> {
    let position = setup
        .restore_position()
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

/// 筋と段の見出しを付け、SFENと同じ向きの12行12列を作る。
fn diagnostic_grid(width: usize, mut cell: impl FnMut(Square) -> String) -> Vec<String> {
    let mut lines = vec![format!(
        "  {}",
        (1..=12)
            .rev()
            .map(|file| format!("{file:>width$}"))
            .collect::<Vec<_>>()
            .join(" ")
    )];
    for rank in (0..12).rev() {
        let cells = (0..12)
            .map(|file| {
                let square = Square::new(file, rank).expect("board coordinates must be valid");
                format!("{:>width$}", cell(square))
            })
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!("{} {cells}", char::from(b'a' + 11 - rank)));
    }
    lines
}

/// RULES.md第5条・第9条・第10条に従い、現在の駒種を漢字1文字にする。
fn diagnostic_piece_kanji(piece: PieceCode) -> char {
    match piece.kind().expect("a board piece must have a kind") {
        PieceKind::Pawn => '歩',
        PieceKind::GoBetween => '仲',
        PieceKind::Lance => '香',
        PieceKind::ReverseChariot => '反',
        PieceKind::SideMover => '横',
        PieceKind::VerticalMover => '竪',
        PieceKind::Bishop => '角',
        PieceKind::Rook => '飛',
        PieceKind::DragonHorse => '馬',
        PieceKind::DragonKing => '龍',
        PieceKind::FreeKing => '奔',
        PieceKind::King => match piece.color().expect("a board piece must have a color") {
            Color::Black => '王',
            Color::White => '玉',
        },
        PieceKind::DrunkElephant => '醉',
        PieceKind::FerociousLeopard => '猛',
        PieceKind::BlindTiger => '盲',
        PieceKind::CopperGeneral => '銅',
        PieceKind::SilverGeneral => '銀',
        PieceKind::GoldGeneral => '金',
        PieceKind::Kirin => '麒',
        PieceKind::Phoenix => '鳳',
        PieceKind::Lion => '獅',
        PieceKind::CrownPrince => '太',
        PieceKind::WhiteHorse => '白',
        PieceKind::Whale => '鯨',
        PieceKind::FlyingOx => '牛',
        PieceKind::FreeBoar => '猪',
        PieceKind::FlyingStag => '鹿',
        PieceKind::HornedFalcon => '鷹',
        PieceKind::SoaringEagle => '鷲',
    }
}

/// `d`用に、漢字の駒と筋・段の見出しを持つ罫線付きの盤を作る。
fn diagnostic_board(position: &Position) -> Vec<String> {
    let border = format!("{}+", "+----".repeat(12));
    let mut lines = vec![
        format!(
            " {}",
            (1..=12)
                .rev()
                .map(|file| format!("{file:>4}"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        border.clone(),
    ];
    for rank in (0..12).rev() {
        let mut row = String::from("|");
        for file in 0..12 {
            let square = Square::new(file, rank).expect("board coordinates must be valid");
            if let Some(piece) = position.piece_at(square) {
                row.push(
                    match piece.color().expect("a board piece must have a color") {
                        Color::Black => ' ',
                        Color::White => '^',
                    },
                );
                row.push(if piece.is_promoted() { '+' } else { ' ' });
                row.push(diagnostic_piece_kanji(piece));
            } else {
                row.push_str("  ・");
            }
            row.push('|');
        }
        row.push(' ');
        row.push(char::from(b'a' + 11 - rank));
        lines.push(row);
        lines.push(border.clone());
    }
    lines
}

/// 盤面、拡張SFEN、対局状態、およびキーを表示用の行へ変換する。
///
/// 先に[`Position::validate`]を呼び、失敗時はキーを表示せず理由を返す。
/// 駒の表示は[`diagnostic_board`]に従い、後手に`^`、成駒に`+`を付ける。
/// 漢字は[`diagnostic_piece_kanji`]が成った後の駒種から選ぶ。
fn diagnostic_position(engine: &Engine) -> Result<Vec<String>, String> {
    let position = engine.game().position();
    position.validate().map_err(|error| error.to_string())?;
    let next_move = engine
        .ply()
        .checked_add(1)
        .ok_or_else(|| "move number overflow".to_owned())?;
    let setup = SetupPosition::new(position.clone(), position.lion_capture_square(), next_move)
        .map_err(|error| error.to_string())?;
    let status = state_status_text(engine.status())?;
    let mut lines = diagnostic_board(position);
    lines.push(String::new());
    lines.push(format!("sfen {}", to_extended_sfen(&setup)));
    lines.push(format!(
        "side {}",
        match position.side_to_move() {
            Color::Black => "black",
            Color::White => "white",
        }
    ));
    lines.push(format!("move {next_move}"));
    lines.push(format!("status {status}"));
    lines.push(format!(
        "lion {}",
        position
            .lion_capture_square()
            .map_or_else(|| "none".to_owned(), square_to_text)
    ));
    let deferred = Square::all()
        .filter(|&square| position.promotion_deferred().contains(square))
        .map(square_to_text)
        .collect::<Vec<_>>();
    lines.push(format!(
        "promotion-deferred {}",
        if deferred.is_empty() {
            "none".to_owned()
        } else {
            deferred.join(",")
        }
    ));
    lines.push(format!("zobrist {:016x}", position.zobrist()));
    lines.push(format!("rights-zobrist {:016x}", position.rights_zobrist()));
    Ok(lines)
}

/// NNUEを全計算した手番側の評価値と、線形経路の寄与を表示する。
///
/// 入力特徴集合はP型である。
/// 升ごとの線形寄与は720で割り、0.1 cpへ丸めて表示する。
/// 線形評価の整数除算と表示時の丸めがあるため、表示した寄与の和は評価値と一致するとは限らない。
/// 非線形出力は別行で示し、合計評価の±28,999 cpへの切り詰めも注記する。
fn diagnostic_evaluation(position: &Position) -> Result<Vec<String>, String> {
    position.validate().map_err(|error| error.to_string())?;
    let weights = nnue::weights().map_err(|error| error.to_string())?;
    let detail = nnue::breakdown(&weights, position);
    let mut lines = vec![
        format!("evaluation {}", detail.score),
        format!("q {}", detail.q),
    ];
    lines.extend(diagnostic_grid(7, |square| {
        format!("{:.1}", detail.board[square.dense_index()] as f64 / 720.0)
    }));
    lines.push(format!("lion {:.1}", detail.lion as f64 / 720.0));
    lines.push(format!("nonlinear {}", detail.nonlinear));
    lines.push("note linear contributions are rounded to 0.1 cp; linear evaluation uses integer truncation by 720; the sum with nonlinear output is clamped to +/-28999 cp".to_owned());
    Ok(lines)
}

/// 置換表の項目を、詰み手数補正後の探索値、静的評価、および記録手のUSI表記へ変換する。
fn diagnostic_hit(
    position: &Position,
    generator: &MoveGenerator,
    hit: Option<search::Hit>,
) -> Result<String, String> {
    let Some(hit) = hit else {
        return Ok("none".to_owned());
    };
    let best_move = match hit.best_move {
        None => "none".to_owned(),
        Some(mv) if mv.from == mv.to && mv.mid.is_none() => {
            // 助言手は別の保存操作に由来し得る。じっとの中間升を補う前に検証する。
            usi::text(position, mv, generator).map_err(|error| error.to_string())?
        }
        Some(mv) => usi::text_generated(position, mv),
    };
    let static_eval = hit
        .static_eval
        .map_or_else(|| "none".to_owned(), |value| value.to_string());
    let Some(value) = hit.value else {
        return Ok(format!(
            "search none bestmove {best_move} static-eval {static_eval}"
        ));
    };
    let bound = match value.bound {
        search::Bound::Exact => "exact",
        search::Bound::Lower => "lower",
        search::Bound::Upper => "upper",
    };
    Ok(format!(
        "depth {} bound {bound} score {} bestmove {best_move} static-eval {static_eval}",
        value.depth, value.score
    ))
}

/// 置換表を変更せず、現局面と全合法手の子局面を照会する。
///
/// 探索と同じ照会関数で検証キーを照合し、現局面は根から0手、子局面は1手として詰みの値を補正する。
/// 子局面の評価値は子局面の手番側の視点である。
/// 最善手は評価値と別の語に保存されるため、同じ探索や保存操作に由来するとは限らない。
fn diagnostic_table(engine: &Engine, table: &TranspositionTable) -> Result<Vec<String>, String> {
    let game = engine.game();
    let position = game.position();
    let generator = MoveGenerator::new(game.rules().moves);
    let mut lines = vec![format!(
        "current {}",
        diagnostic_hit(position, &generator, table.inspect(position, 0))?
    )];
    for mv in game.legal_moves() {
        let mut child = position.clone();
        child
            .try_make_move(mv, &generator)
            .map_err(|error| error.to_string())?;
        lines.push(format!(
            "{} {}",
            usi::text_generated(position, mv),
            diagnostic_hit(&child, &generator, table.inspect(&child, 1))?
        ));
    }
    lines.push(
        "note bestmove and static-eval may come from a different store operation than the score"
            .to_owned(),
    );
    Ok(lines)
}

/// `position`拒否の理由文を、拒否された指し手の表記を添えて作る。
fn position_reject_reason_text(reason: &RejectReason, rejected_text: Option<&str>) -> String {
    if let (RejectReason::IllegalMove { cause, .. }, Some(text)) = (reason, rejected_text) {
        format!("illegal move '{text}': {cause}")
    } else {
        reason.to_string()
    }
}

/// 状態を`ongoing`、`win <勝者> <理由>`、`draw <理由>`へ変換する。
///
/// 勝者は`black`か`white`である。
/// 勝ちの理由は`royal-capture`、`repetition`、`piece-exhaustion`、`bare-king`、`stalemate`、`mate`、
/// 引き分けの理由は`repetition`、`piece-exhaustion`、`bare-king`である。
/// `piece-exhaustion`はRULES.md第22条の駒枯れ、`bare-king`は第32条E3の裸玉裁定に対応する。
/// 投了は`state cannot represent resignation`、合意引き分けは`state cannot represent agreement`として拒否する。
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
    mod history;
    mod tuning;
    use std::collections::HashSet;

    use super::*;
    use minase_core::Rules;
    use minase_core::rules::RuleCode;

    /// RULES.md第5条の初期配置に対応する2欄SFEN（盤面部と手番部）。
    const INITIAL_BOARD: &str = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b";

    /// 王駒捕獲1手前の局面。7g7dで先手が後手の最後の王駒を取る。
    const ROYAL_SFEN: &str = "12/12/12/5k6/12/12/5R6/12/12/12/12/K11 b - 1";

    /// [`UsiProtocol::handle_state`]に従う、7g7d適用後のFinished局面のstate行。
    const ROYAL_FINISHED_STATE: &str = "state rules L0,P0,R1,E2 board 12/12/12/5R6/12/12/12/12/12/12/12/K11 w status win black royal-capture";

    /// [`UsiProtocol::handle_moves`]に従う、開始前と終局後のmovesエラー行。
    const MOVES_ERROR: &str = "info string error: moves requires an active game";

    /// [`UsiProtocol::handle_state`]に従う、AwaitingStartのstateエラー行。
    const STATE_ERROR: &str = "info string error: state requires an active or finished game";

    // [`score_text`]の表示例と詰み帯の境界を検査する。
    #[test]
    fn score_text_formats_mate_and_cp_values() {
        for (score, expected) in [
            (search::MATE - 1, "mate +0"),
            (search::MATE - 3, "mate 1"),
            (search::MATE - 5, "mate 3"),
            (-search::MATE, "mate -0"),
            (-search::MATE + 2, "mate -0"),
            (-search::MATE + 4, "mate -2"),
            (search::MATE_THRESHOLD, "mate 254"),
            (-search::MATE_THRESHOLD, "mate -254"),
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

    /// 最後の`info`の主変化と`bestmove`を照合し、予想手があれば最善手との組を返す。
    fn assert_bestmove_matches_final_pv(output: &str) -> Option<(&str, &str)> {
        let bestmoves = bestmoves(output);
        assert_eq!(bestmoves.len(), 1, "{output}");
        let bestmove: Vec<_> = bestmoves[0].split_whitespace().collect();
        let pv: Vec<_> = output
            .lines()
            .filter_map(|line| line.split_once(" pv ").map(|(_, pv)| pv))
            .next_back()
            .unwrap_or_else(|| panic!("missing final PV: {output}"))
            .split_whitespace()
            .collect();
        match pv.as_slice() {
            [first] => {
                assert_eq!(bestmove, ["bestmove", *first], "{output}");
                None
            }
            [first, second, ..] => {
                assert_eq!(
                    bestmove,
                    ["bestmove", *first, "ponder", *second],
                    "{output}"
                );
                Some((first, second))
            }
            [] => panic!("empty final PV: {output}"),
        }
    }

    /// 開始局面で最善手を指した後の`moves`に予想手が含まれることを確認する。
    fn assert_startpos_ponder_is_legal(best_move: &str, ponder_move: &str) {
        let output = session(
            &[RuleCode::R1],
            &format!("position startpos moves {best_move}\nmoves\n"),
        );
        assert!(error_lines(&output).is_empty(), "{output}");
        let replies = moves_sets(&output);
        assert_eq!(replies.len(), 1, "{output}");
        assert!(replies[0].contains(ponder_move), "{output}");
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
        // [`UsiProtocol::write_handshake`]の固定宣言を検査する。
        // USI_Hashの宣言位置は規範文書に明記されていないため、存在だけを確認する。
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
        // [`UsiProtocol::write_handshake`]に従い、USI_Hashの宣言を検査する。
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("option name USI_Hash"))
        );
        // [`UsiProtocol::write_handshake`]に従い、Threadsの宣言はdefaultを探索層の既定値から表示する。
        assert!(lines.contains(&"option name Threads type spin default 1 min 1 max 256"));
        // [`UsiProtocol::write_handshake`]のResignValue宣言を検査する。
        assert_eq!(
            lines
                .iter()
                .copied()
                .filter(|line| line.starts_with("option name ResignValue "))
                .collect::<Vec<_>>(),
            ["option name ResignValue type spin default 20000 min 1 max 99999"]
        );
        // [`UsiProtocol::handle_setoption`]のByoyomiMarginの単位と受理範囲を検査する。
        assert_eq!(
            lines
                .iter()
                .copied()
                .filter(|line| line.starts_with("option name ByoyomiMargin "))
                .collect::<Vec<_>>(),
            ["option name ByoyomiMargin type spin default 30 min 0 max 60000"]
        );
        // [`UsiProtocol::write_handshake`]はUSI_Ponderを宣言しない。
        assert!(!output.contains("USI_Ponder"));
    }

    /// RULES.md第31条R1の引き分け値は、両手番とも既定値0、範囲±28,999で宣言する。
    #[test]
    fn draw_value_r1_handshake_declares_defaults_and_bounds() {
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);
        let output = run(&mut protocol, &mut engine, "usi\n");
        let lines: Vec<_> = output.lines().collect();
        let margin = lines
            .iter()
            .position(|line| line.starts_with("option name ByoyomiMargin "))
            .unwrap();
        let expected = [
            "option name DrawValueBlack type spin default 0 min -28999 max 28999",
            "option name DrawValueWhite type spin default 0 min -28999 max 28999",
        ];
        assert_eq!(&lines[margin + 1..margin + 3], &expected);
        assert_eq!(
            lines
                .iter()
                .copied()
                .filter(|line| line.starts_with("option name DrawValue"))
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(protocol.draw_value_black, 0);
        assert_eq!(protocol.draw_value_white, 0);
    }

    /// RULES.md第31条R1の引き分け値は、符号と範囲を検査し、もう一方の設定を保つ。
    #[test]
    fn draw_value_r1_parsing_accepts_signed_bounds_and_preserves_rejected_values() {
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);
        for (name, side) in [
            ("DrawValueBlack", Color::Black),
            ("DrawValueWhite", Color::White),
        ] {
            let other = match side {
                Color::Black => Color::White,
                Color::White => Color::Black,
            };
            let other_value = protocol.root_draw_value(other);
            for (value, expected) in [
                ("-50", -50),
                ("50", 50),
                ("0", 0),
                ("-28999", -28_999),
                ("28999", 28_999),
                ("5 0", 5),
                ("5\t0", 5),
                ("-007", -7),
            ] {
                let output = run(
                    &mut protocol,
                    &mut engine,
                    &format!(
                        "setoption NAME {} VALUE {value}\n",
                        name.to_ascii_lowercase()
                    ),
                );
                assert_eq!(output, "", "{name}: {value}");
                assert_eq!(protocol.root_draw_value(side), expected);
                assert_eq!(protocol.root_draw_value(other), other_value);
                // readyokやttの空項目では、未確保と確保済みの空表を区別できない。
                // 初回探索前の確保という契約に限り内部状態を観測する。
                assert!(
                    protocol
                        .searcher
                        .as_ref()
                        .unwrap()
                        .transposition_table()
                        .is_none()
                );
            }
            for value in [
                "29000",
                "-29000",
                "+5",
                "--5",
                "1.5",
                "1e3",
                "abc",
                "-",
                "- 5",
                "５",
                "2147483648",
                "-2147483648",
                "-2147483649",
            ] {
                let output = run(
                    &mut protocol,
                    &mut engine,
                    &format!("setoption name {name} value {value}\n"),
                );
                assert_eq!(
                    output,
                    format!("info string error: invalid {name} value\n"),
                    "{value}"
                );
                assert_eq!(protocol.root_draw_value(side), -7);
                assert_eq!(protocol.root_draw_value(other), other_value);
            }
            for suffix in ["", " value", " value "] {
                assert_eq!(
                    run(
                        &mut protocol,
                        &mut engine,
                        &format!("setoption name {name}{suffix}\n")
                    ),
                    format!("info string error: missing {name} value\n")
                );
                assert_eq!(protocol.root_draw_value(side), -7);
                assert_eq!(protocol.root_draw_value(other), other_value);
            }
        }
    }

    /// RULES.md第31条R1の引き分け値が変わった受理だけが、置換表と全ワーカーの履歴を消す。
    #[test]
    fn draw_value_r1_clears_tables_only_when_an_accepted_value_changes() {
        for name in ["DrawValueBlack", "DrawValueWhite"] {
            let mut engine = make_engine(&[RuleCode::R1]);
            let mut protocol = UsiProtocol::new(&engine);
            assert_eq!(
                run(
                    &mut protocol,
                    &mut engine,
                    "setoption name USI_Hash value 1\nsetoption name Threads value 2\nposition startpos\n"
                ),
                ""
            );
            for (value, clears, error) in [
                ("-50", true, false),
                ("-50", false, false),
                ("29000", false, true),
            ] {
                let output = run(&mut protocol, &mut engine, "go depth 1\n");
                assert!(error_lines(&output).is_empty(), "{output}");
                // probeは探索層に限定されるため、それを呼ぶinspectで同じ照合を使う。
                let before = protocol
                    .searcher
                    .as_ref()
                    .unwrap()
                    .transposition_table()
                    .unwrap()
                    .inspect(engine.game().position(), 0)
                    .unwrap();
                let histories = protocol.searcher.as_mut().unwrap().histories_mut_for_test();
                for history in &mut histories.workers {
                    for value in history.iter_mut().flatten().flatten() {
                        *value = -100;
                    }
                }
                for history in &mut histories.capture {
                    history.values_mut().fill(-100);
                }
                let output = run(
                    &mut protocol,
                    &mut engine,
                    &format!("setoption name {name} value {value}\n"),
                );
                if error {
                    assert_eq!(output, format!("info string error: invalid {name} value\n"));
                } else {
                    assert_eq!(output, "");
                }
                let after = protocol
                    .searcher
                    .as_ref()
                    .unwrap()
                    .transposition_table()
                    .unwrap()
                    .inspect(engine.game().position(), 0);
                if clears {
                    assert!(after.is_none());
                } else {
                    let after = after.unwrap();
                    assert_eq!(
                        (after.best_move, after.static_eval, after.value),
                        (before.best_move, before.static_eval, before.value)
                    );
                }
                let histories = protocol.searcher.as_ref().unwrap().histories_for_test();
                let quiet_expected = if clears { -5 } else { -100 };
                let capture_expected = if clears { -742 } else { -100 };
                assert_eq!(histories.workers.len(), 2);
                assert_eq!(histories.capture.len(), 2);
                assert!(histories.workers.iter().all(|history| {
                    history
                        .iter()
                        .flatten()
                        .flatten()
                        .all(|&value| value == quiet_expected)
                }));
                assert!(histories.capture.iter().all(|history| {
                    history
                        .values()
                        .iter()
                        .all(|&value| value == capture_expected)
                }));
            }
        }
    }

    /// RULES.md第31条R1の引き分け値は、根の手番に対応するオプションを選ぶ。
    #[test]
    fn draw_value_r1_selects_the_root_side_option() {
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                "setoption name DrawValueBlack value -50\nsetoption name DrawValueWhite value 70\n"
            ),
            ""
        );
        for (position, side, expected) in [
            ("position startpos", Color::Black, -50),
            ("position startpos moves 6i6h", Color::White, 70),
        ] {
            assert_eq!(run(&mut protocol, &mut engine, position), "");
            let game = engine.game();
            assert_eq!(game.position().side_to_move(), side);
            let snapshot = SearchSnapshot::from_game(game)
                .unwrap()
                .with_draw_value(protocol.root_draw_value(game.position().side_to_move()))
                .unwrap();
            assert_eq!(snapshot.draw_value(), expected);
        }
    }

    /// RULES.md第31条R1の既定値0では、新規対局を挟んだ同じ探索の最善手と最終評価値が一致する。
    #[test]
    fn draw_value_r1_default_search_is_deterministic_across_new_games() {
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);
        let first = run(
            &mut protocol,
            &mut engine,
            "position startpos\ngo depth 3\n",
        );
        let second = run(
            &mut protocol,
            &mut engine,
            "usinewgame\nposition startpos\ngo depth 3\n",
        );
        let final_score = |output: &str| -> i32 {
            assert!(error_lines(output).is_empty(), "{output}");
            let line = output
                .lines()
                .rev()
                .find(|line| line.starts_with("info depth "))
                .unwrap();
            assert!(line.starts_with("info depth 3 "), "{output}");
            line.split_once(" score cp ")
                .unwrap()
                .1
                .split_whitespace()
                .next()
                .unwrap()
                .parse()
                .unwrap()
        };
        assert_eq!(bestmoves(&first).len(), 1);
        assert_eq!(bestmoves(&first), bestmoves(&second));
        assert_eq!(final_score(&first), final_score(&second));
    }

    // [`UsiProtocol::handle_setoption`]のByoyomiMarginの単位と受理範囲を検査する。
    #[test]
    fn byoyomi_margin_accepts_bounds_and_rejects_invalid_values_without_changing_it() {
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);
        assert_eq!(protocol.byoyomi_margin_ms, 30);
        for (value, expected) in [("0", 0), ("60000", 60_000), ("00123", 123)] {
            assert_eq!(
                run(
                    &mut protocol,
                    &mut engine,
                    &format!("setoption name byoyomimargin value {value}\n")
                ),
                ""
            );
            let config = parse_go_config(
                &["btime", "0", "wtime", "0", "byoyomi", "1000"],
                Color::Black,
                0,
                protocol.byoyomi_margin_ms,
            )
            .unwrap();
            assert_eq!(config.clock().unwrap().byoyomi_margin_ms(), expected);
        }
        for value in [
            "",
            "value",
            "value -1",
            "value +1",
            "value 60001",
            "value nope",
            "value 1.5",
            "value 999999999999999999999999999",
        ] {
            let output = run(
                &mut protocol,
                &mut engine,
                &format!("setoption name ByoyomiMargin {value}\n"),
            );
            assert_eq!(
                output,
                if value.is_empty() || value == "value" {
                    "info string error: missing ByoyomiMargin value\n"
                } else {
                    "info string error: invalid ByoyomiMargin value\n"
                }
            );
            assert_eq!(protocol.byoyomi_margin_ms, 123);
        }
    }

    #[test]
    fn threads_accepts_boundaries_and_rejects_invalid_values() {
        // [`UsiProtocol::handle_setoption`]に従い、Threadsは1..=256だけを受理し、
        // 不正値には固定エラーを返す。正当値は無応答である。
        assert_eq!(parse_spin("1", 1, 256), Some(1));
        assert_eq!(parse_spin("256", 1, 256), Some(256));
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);
        // 探索開始へ渡す値を受信後に観測し、設定の無視や丸めを検出する。
        for (value, threads) in [("1", 1), ("2", 2), ("+2", 2)] {
            assert_eq!(
                run(
                    &mut protocol,
                    &mut engine,
                    &format!("setoption name tHREADS value {value}\n")
                ),
                ""
            );
            assert_eq!(protocol.searcher.as_ref().unwrap().threads().get(), threads);
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
        assert_eq!(protocol.searcher.as_ref().unwrap().threads().get(), 2);
    }

    /// 王将の逃げ先が奔王、竪行、飛車に覆われ、反車でも捕獲を防げない局面。
    /// 投了判定と反復の照合に使う拡張SFEN。
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

    // [`UsiProtocol::handle_setoption`]と[`UsiProtocol::should_resign`]の契約に従い、設定の保持は次の探索の応答で観測する。
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
            "value 0",
            "value 100000",
            "value +1",
            "value 999999999999999999999999999",
        ] {
            let output = run(
                &mut protocol,
                &mut engine,
                &format!("setoption name ResignValue {value}\n"),
            );
            assert_eq!(
                output,
                if value.is_empty() {
                    "info string error: missing ResignValue value\n"
                } else {
                    "info string error: invalid ResignValue value\n"
                }
            );
        }
        let output = run(&mut protocol, &mut engine, "go depth 1\n");
        assert_eq!(bestmoves(&output), ["bestmove resign"], "{output}");
    }

    // [`UsiProtocol::should_resign`]の契約に従い、深さ0の静的評価では投了しない。
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

    // [`UsiProtocol::should_resign`]の既定閾値と投了の出力形式を検証する。
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
            // Threads>1の完了深さの通知順はスケジューリングに依存する。
            if threads == 1 {
                assert!(depths.windows(2).all(|pair| pair[0] < pair[1]), "{output}");
            }
        }
    }

    // [`UsiProtocol::should_resign`]の閾値判定は、投了を無効化した探索の公開出力を基準に検証する。
    // 評価値の定数を写さず、等号と隣接値の関係を確かめる。
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

    // [`UsiProtocol::should_resign`]の閾値と評価値を符号付きで比較する契約を検証する。
    #[test]
    fn resignation_mate_thresholds_and_winning_scores_use_signed_comparison() {
        for threshold in [29998, 29999, 99999] {
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

    // [`UsiProtocol::should_resign`]に従い、途中結果の下界が投了閾値を下回っても
    // 完了値で判定し、通常終了・保留後のstop・ponderhitのいずれでも着手を返す。
    #[test]
    fn partial_info_uses_lowerbound_and_resignation_uses_completed_score() {
        for (partial_score, score_text) in [(-1000, "cp -1000"), (-search::MATE + 1, "mate -0")] {
            for release in [None, Some("stop"), Some("ponderhit")] {
                let (mut engine, mut protocol) = resignation_engine(RESIGN_MATERIAL_SFEN);
                run(
                    &mut protocol,
                    &mut engine,
                    "setoption name ResignValue value 100\n",
                );
                let legal = moves_sets(&run(&mut protocol, &mut engine, "moves\n")).remove(0);
                let mut output = Vec::new();
                let limits = if release.is_some() {
                    vec!["ponder", "depth", "2"]
                } else {
                    vec!["depth", "2"]
                };
                let mut active = protocol.start_go(&engine, &limits, &mut output).unwrap();
                loop {
                    let Some(ActiveSearch::Running { handle, .. }) = active.as_ref() else {
                        panic!("search must be running");
                    };
                    let mut event = handle
                        .events()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                    let finished = if let SearchEvent::Finished {
                        score,
                        partial_score: lowerbound,
                        depth,
                        ..
                    } = &mut event
                    {
                        *score = 100;
                        *depth = 2;
                        *lowerbound = Some(partial_score);
                        true
                    } else {
                        false
                    };
                    protocol
                        .handle_search_event(&engine, &mut active, event, &mut output)
                        .unwrap();
                    if finished {
                        break;
                    }
                }
                if let Some(command) = release {
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
                let text = String::from_utf8(output).unwrap();
                assert_eq!(text.matches("info string partial\n").count(), 1, "{text}");
                assert!(
                    text.contains(&format!("info depth 3 score {score_text} lowerbound ")),
                    "{text}"
                );
                assert!(!text.contains("bestmove resign"), "{text}");
                assert_legal_bestmove(&text, &legal);
            }
        }
    }

    // RULES.md第31条R1の反復負けは1回の再現による見込みなので、実際の裁定より早く投了しない。
    // 実行中のstopが受信端から結果を回収する経路には評価値の差し替え口がないため、
    // その経路も使う共有判定を直接検査する。
    #[test]
    fn resignation_repetition_band_is_excluded_at_every_threshold() {
        let (_, mut protocol) = resignation_engine(RESIGN_MATERIAL_SFEN);
        for threshold in [0, 1, 20_000, 29_487, 29_743, 99_999] {
            protocol.resign_value = threshold;
            for score in [-29_487, -29_740, -29_743] {
                assert!(!protocol.should_resign(score, 2), "{score}, {threshold}");
                assert!(!protocol.should_resign(score, 0));
            }
        }
        protocol.resign_value = 20_000;
        for (score, expected) in [
            (-29_744, true),
            (-29_999, true),
            (-29_486, true),
            (-20_000, true),
            (-19_999, false),
            (29_487, false),
            (29_743, false),
        ] {
            assert_eq!(protocol.should_resign(score, 2), expected);
            assert!(!protocol.should_resign(score, 0));
        }
    }

    /// 探索イベントの評価値を規範値へ差し替え、既定のResignValueで最終出力を検査する。
    fn check_repetition_band_resignation(score: i32, release: Option<&str>, running_hit: bool) {
        let (mut engine, mut protocol) = resignation_engine(RESIGN_MATERIAL_SFEN);
        assert_eq!(protocol.resign_value, 20_000);
        let legal = moves_sets(&run(&mut protocol, &mut engine, "moves\n")).remove(0);
        let mut output = Vec::new();
        let limits = if release.is_some() || running_hit {
            vec!["ponder", "depth", "2"]
        } else {
            vec!["depth", "2"]
        };
        let mut active = protocol.start_go(&engine, &limits, &mut output).unwrap();
        if running_hit {
            assert!(matches!(active, Some(ActiveSearch::Running { .. })));
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
        }
        loop {
            let Some(ActiveSearch::Running { handle, .. }) = active.as_ref() else {
                panic!("search must be running");
            };
            let mut event = handle
                .events()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            let finished = match &mut event {
                SearchEvent::Progress {
                    score: reported, ..
                } => {
                    *reported = score;
                    false
                }
                SearchEvent::Finished { score: adopted, .. } => {
                    *adopted = score;
                    true
                }
            };
            protocol
                .handle_search_event(&engine, &mut active, event, &mut output)
                .unwrap();
            if finished {
                break;
            }
        }
        if let Some(command) = release {
            assert!(matches!(active, Some(ActiveSearch::AwaitingStop { .. })));
            assert!(bestmoves(std::str::from_utf8(&output).unwrap()).is_empty());
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
        let text = String::from_utf8(output).unwrap();
        if score == -29_744 {
            assert!(text.contains(" score mate -254 "), "{text}");
            assert_eq!(bestmoves(&text), ["bestmove resign"]);
        } else {
            assert!(text.contains(&format!(" score cp {score} ")), "{text}");
            assert_legal_bestmove(&text, &legal);
        }
    }

    // RULES.md第31条R1: 反復負けの見込みでは投了せず、詰み帯の境界では投了する。
    #[test]
    fn article_31_resignation_repetition_band_returns_a_legal_move_or_resigns() {
        for (release, running_hit, score) in [
            (None, false, -29_487),
            (None, false, -29_740),
            (None, false, -29_743),
            (None, false, -29_744),
            (Some("stop"), false, -29_487),
            (Some("stop"), false, -29_740),
            (Some("stop"), false, -29_743),
            (Some("stop"), false, -29_744),
            (Some("ponderhit"), false, -29_487),
            (Some("ponderhit"), false, -29_740),
            (Some("ponderhit"), false, -29_743),
            (Some("ponderhit"), false, -29_744),
            (None, true, -29_487),
            (None, true, -29_740),
            (None, true, -29_743),
            (None, true, -29_744),
        ] {
            let result = std::panic::catch_unwind(|| {
                check_repetition_band_resignation(score, release, running_hit);
            });
            assert!(
                result.is_ok(),
                "article 31: release={release:?}, running_hit={running_hit}, score={score}"
            );
        }
    }

    // [`UsiProtocol::should_resign`]の判定対象と投了の出力形式を検証する。
    // 探索イベント境界へ規範値を入力し、進捗と採用値が異なる場合を決定的に検証する。
    #[test]
    fn resignation_uses_adopted_score_without_repeating_info() {
        for (score, threshold, resign) in [(-100, 100, true), (0, 1, false), (100, 1, false)] {
            for release in [None, Some("stop")] {
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

    // [`UsiProtocol::should_resign`]の適用範囲を検査する。
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

    // [`UsiProtocol::should_resign`]と[`UsiProtocol::handle_state`]の契約を検査する。
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

    // [`UsiProtocol::should_resign`]の適用範囲と[`write_bestmove`]の停止理由を検査する。
    // 完了イベントをまだ消費していないRunningからstopを処理する。
    #[test]
    fn resignation_running_stop_returns_resign_for_ponder_search() {
        let (mut engine, mut protocol) = resignation_engine(RESIGN_MATERIAL_SFEN);
        run(
            &mut protocol,
            &mut engine,
            "setoption name ResignValue value 1\n",
        );
        let mut output = Vec::new();
        let mut active = protocol
            .start_go(&engine, &["ponder", "depth", "256"], &mut output)
            .unwrap();
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

    // [`UsiProtocol::should_resign`]の適用範囲と[`UsiProtocol::ponderhit`]の契約を検査する。
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

    // [`UsiProtocol::should_resign`]の入力と適用範囲を検査する。
    // Finishedを先に処理して保留状態を確実に作り、実行中の経路と区別する。
    #[test]
    fn resignation_held_depth_zero_does_not_resign_on_stop_or_ponderhit() {
        for command in ["stop", "ponderhit"] {
            let (mut engine, mut protocol) = resignation_engine(RESIGN_MATE_SFEN);
            run(
                &mut protocol,
                &mut engine,
                "setoption name ResignValue value 1\n",
            );
            let legal = moves_sets(&run(&mut protocol, &mut engine, "moves\n")).remove(0);
            let mut output = Vec::new();
            let mut active = protocol
                .start_go(&engine, &["ponder", "nodes", "1"], &mut output)
                .unwrap();
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
            assert!(released.starts_with("info string stop nodes\n"));
            assert_eq!(released.lines().count(), 2);
            assert_legal_bestmove(released, &legal);
        }
    }

    // [`UsiProtocol::handle_setoption`]と[`UsiProtocol::should_resign`]の契約。待機列は現在のbestmoveより後に適用する。
    #[test]
    fn resignation_option_changes_wait_until_running_result_is_released() {
        let (mut engine, mut protocol) = resignation_engine(RESIGN_MATERIAL_SFEN);
        run(
            &mut protocol,
            &mut engine,
            "setoption name ResignValue value 99999\n",
        );
        let legal = moves_sets(&run(&mut protocol, &mut engine, "moves\n")).remove(0);
        let mut output = Vec::new();
        let mut active = protocol
            .start_go(&engine, &["ponder", "depth", "256"], &mut output)
            .unwrap();
        resignation_wait_for_iteration(&mut protocol, &engine, &mut active, &mut output);
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

    #[test]
    fn isready_allocates_the_default_transposition_table_before_the_first_go() {
        // 対局開始直後の手が置換表の確保を待たないよう、isreadyで既定容量の
        // [`UsiProtocol::handle_idle_line`]のisready分岐に従い、探索前に置換表を作る。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);
        let before = run(&mut protocol, &mut engine, "position startpos\nstate\n");
        assert!(
            protocol
                .searcher
                .as_ref()
                .unwrap()
                .transposition_table()
                .is_none()
        );
        let output = run(&mut protocol, &mut engine, "isready\nstate\n");
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines[0], "readyok");
        // モジュールdocのコマンド表に従い、isreadyは対局状態を変えない。
        assert_eq!(before.lines().collect::<Vec<_>>(), [lines[1]]);
        assert!(
            protocol
                .searcher
                .as_ref()
                .unwrap()
                .transposition_table()
                .is_some()
        );
    }

    // RULES.md第33条とSetRulesの契約: 受信時に検証し、拒否時は直前の正当な次局規則を保つ。
    #[test]
    fn article_33_ruleset_values_are_validated_and_pending_survives_rejection() {
        for (name, settings, errors, expected) in [
            (
                "article 33 case and duplicate",
                "setoption name RuleSet value l1,p0,r2,e0\nsetoption name RuleSet value L1,l1,P0,R1,E0\n",
                1,
                "L1,P0,R2,E0",
            ),
            (
                "article 33 unknown and incomplete",
                "setoption name RuleSet value L1,P0,R2,E0\nsetoption name RuleSet value XX9\nsetoption name RuleSet value L1,E1\n",
                2,
                "L1,P0,R2,E0",
            ),
            (
                "article 33 standard",
                "setoption name RuleSet value lishogi\nsetoption name RuleSet value standard\n",
                1,
                "L1,L2,P0,P3,R1,E1,E3",
            ),
        ] {
            let output = session(
                &[RuleCode::R1],
                &format!("{settings}usinewgame\nposition startpos\nstate\n"),
            );
            assert_eq!(
                error_lines(&output).len(),
                errors,
                "{name}: {settings}: {output}"
            );
            let lines: Vec<_> = output.lines().collect();
            assert_eq!(lines.len(), errors + 1, "{name}: {output}");
            assert!(
                lines[..errors]
                    .iter()
                    .all(|line| line.starts_with("info string error: ")),
                "{name}: {output}"
            );
            assert_eq!(state_rules(lines[errors]), expected, "{name}: {settings}");
        }
    }

    #[test]
    fn article_33_presets_expand_to_code_lists() {
        // [`UsiProtocol::handle_setoption`]と[`EngineCommand::SetRules`]の契約、およびRULES.md第33条第5項と第6項に従う。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = UsiProtocol::new(&engine);
        let output = run(
            &mut protocol,
            &mut engine,
            "setoption name RuleSet value LISHOGI\nusinewgame\nposition startpos\nstate\n",
        );
        assert_eq!(
            output,
            format!("state rules L1,L2,P0,P3,R1,E1,E3 board {INITIAL_BOARD} status ongoing\n")
        );
        assert!(!output.contains("lishogi"));

        let output = run(
            &mut protocol,
            &mut engine,
            "gameover win\nsetoption name RuleSet value engine-default\nposition startpos\nstate\n",
        );
        assert_eq!(state_rules(state_lines(&output)[0]), "L0,P0,R1,E0");
    }

    #[test]
    fn ruleset_changes_latch_until_the_next_game() {
        // [`crate::protocol::engine`]の表に従い、対局中のSetPositionは現局の規則で再構成する。
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
        // [`UsiProtocol::handle_setoption`]と[`EngineCommand::SetRules`]の契約: USI_Variantは値chushogiだけを受理し、他はエラー通知。
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
        // モジュールdocのstartposの解釈とRULES.md第5条に従う。
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

    // EngineCommand::SetPositionの原子性: 構文エラーも途中の不合法手も状態を変えない。
    #[test]
    fn invalid_positions_are_rejected_atomically() {
        for input in [
            "position startpos moves 6i6h 1a1b".to_owned(),
            "position sfen broken-input".to_owned(),
            "position startpos moves 1a1b".to_owned(),
            "position unknown".to_owned(),
            "position startpos extra".to_owned(),
            format!("position sfen {INITIAL_BOARD} - 1 typo"),
            format!("position sfen {INITIAL_BOARD} - 1 - extra"),
        ] {
            let output = session(
                &[RuleCode::R1, RuleCode::E2],
                &format!("position startpos moves 6i6h\nstate\nmoves\n{input}\nstate\nmoves\n"),
            );
            assert_eq!(error_lines(&output).len(), 1, "{input}: {output}");
            let states = state_lines(&output);
            assert_eq!(states.len(), 2, "{input}: {output}");
            assert_eq!(states[0], states[1], "{input}");
            let moves = moves_sets(&output);
            assert_eq!(moves.len(), 2, "{input}: {output}");
            assert_eq!(moves[0], moves[1], "{input}");
        }
    }

    // handle_positionの契約: 差分適用、再送、短縮、置換は全再生と一致する。
    #[test]
    fn article_31_incremental_and_replaced_positions_match_full_replay() {
        let changed_board = INITIAL_BOARD.replace("LFCSGKEGSCFL b", "LFCSGKEGSCF1 b");
        let mut cases = vec![
            (
                "article 31 R1 incremental",
                vec![RuleCode::R1],
                vec![
                    "position startpos".to_owned(),
                    "position startpos moves 3i3h".to_owned(),
                    "position startpos moves 3i3h 5a4b".to_owned(),
                    "position startpos moves 3i3h 5a4b 8l9k".to_owned(),
                ],
                "position startpos moves 3i3h 5a4b 8l9k 8b9b".to_owned(),
            ),
            (
                "article 21 royal capture",
                vec![RuleCode::R1, RuleCode::E2],
                vec![format!("position sfen {ROYAL_SFEN}")],
                format!("position sfen {ROYAL_SFEN} moves 7g7d"),
            ),
        ];
        for command in [
            "position startpos moves 6i6h 1d1e".to_owned(),
            "position startpos moves 6i6h".to_owned(),
            "position startpos moves 3i3h 1d1e".to_owned(),
            format!("position sfen {changed_board} - 1 moves 6i6h 1d1e"),
        ] {
            cases.push((
                "article 31 R1 resend or replacement",
                vec![RuleCode::R1],
                vec!["position startpos moves 6i6h 1d1e".to_owned()],
                command,
            ));
        }
        let mut prelude = Vec::new();
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
            let command = format!("position startpos moves {moves}");
            cases.push((
                "article 31 R2 last move replacement",
                vec![RuleCode::R2, RuleCode::E2],
                prelude.clone(),
                command.clone(),
            ));
            prelude.push(command);
        }
        for (name, rules, prelude, command) in cases {
            let mut full_engine = make_engine(&rules);
            let mut full_protocol = UsiProtocol::new(&full_engine);
            let mut incremental_engine = make_engine(&rules);
            let mut incremental_protocol = UsiProtocol::new(&incremental_engine);
            for input in &prelude {
                let output = run(&mut incremental_protocol, &mut incremental_engine, input);
                assert!(output.is_empty(), "{name}: prelude={input}: {output}");
            }
            // 終局局面ではmovesは固定エラーとなるため、stateだけを照会する。
            let queries = if name == "article 21 royal capture" {
                "state\n"
            } else {
                "state\nmoves\n"
            };
            let input = format!("{command}\n{queries}");
            let expected = run(&mut full_protocol, &mut full_engine, &input);
            let actual = run(&mut incremental_protocol, &mut incremental_engine, &input);
            assert!(
                error_lines(&expected).is_empty(),
                "{name}: {input}: {expected}"
            );
            assert_eq!(
                actual, expected,
                "{name}: prelude={prelude:?}, command={command}"
            );
            assert_eq!(
                incremental_engine.game().position(),
                full_engine.game().position(),
                "{name}: {command}"
            );
            assert_eq!(
                incremental_engine.ply(),
                full_engine.ply(),
                "{name}: {command}"
            );
        }
    }

    #[test]
    fn failed_position_blocks_go_until_a_valid_position_recovers() {
        // [`UsiProtocol::start_go`]の同期の前提: 局面同期失敗状態では正当なposition受理までgoを拒否する。
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
        // 2件目のエラーが同期エラーであり、古い局面の探索が始まらないことを検査する。
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
    fn go_time_arguments_are_accepted_and_normalized_per_side() {
        // [`parse_go_config`]の時間引数の受理と手番側の時計選択を検査する。
        let go = "go btime 1000 wtime 2000 binc 10 winc 20 byoyomi 0 nodes 1";
        let black = session(&[RuleCode::R1], &format!("position startpos\n{go}\n"));

        assert_eq!(bestmoves(&black).len(), 1);

        // [`parse_go_config`]の入力解析と単位変換について、手番側の時計の選択を検査する。
        let tokens = [
            "btime", "1000", "wtime", "2000", "binc", "30", "winc", "40", "byoyomi", "500",
        ];
        assert_eq!(
            parse_go_config(&tokens, Color::Black, 37, 30)
                .unwrap()
                .clock(),
            Some(ClockLimits::new(1000, 30, 500, Some(2000), 37).unwrap())
        );
        assert_eq!(
            parse_go_config(&tokens, Color::White, 37, 30)
                .unwrap()
                .clock(),
            Some(ClockLimits::new(2000, 40, 500, Some(1000), 37).unwrap())
        );
    }

    #[test]
    fn go_clock_requires_opponent_time_only_when_time_advantage_applies() {
        let missing_opponent =
            "search clock without byoyomi requires the opponent's remaining time";
        for (args, side) in [
            ("btime 1000", Color::Black),
            ("wtime 1000", Color::White),
            ("ponder btime 1000", Color::Black),
        ] {
            assert_eq!(
                parse_go_config(&args.split_whitespace().collect::<Vec<_>>(), side, 0, 30,)
                    .unwrap_err(),
                missing_opponent,
                "{args}"
            );
        }

        let byoyomi = parse_go_config(&["btime", "1000", "byoyomi", "100"], Color::Black, 0, 30)
            .unwrap()
            .clock()
            .unwrap();
        assert_eq!(byoyomi.opponent_remaining_ms(), None);
    }

    #[test]
    fn position_tracks_absolute_ply_from_sfen_and_moves() {
        // [`Engine::ply`]と[`ClockLimits::ply`]に従い、SFENの基底手数と適用した手数の和を時計へ渡す。
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

    // parse_go_configとstart_goの契約: 不正な引数と非対局中の探索を拒否する。
    #[test]
    fn go_invalid_arguments_and_lifecycle_are_rejected_without_bestmove() {
        let finished = format!("position sfen {ROYAL_SFEN} moves 7g7d\n");
        for (prelude, input, errors) in [
            ("position startpos\n", "go", 1),
            ("position startpos\n", "go foobar 3", 1),
            ("position startpos\n", "go depth 0", 1),
            ("position startpos\n", "go depth 257", 1),
            ("position startpos\n", "go ponder", 1),
            ("position startpos\ngo ponder\n", "ponderhit", 2),
            ("", "go depth 1", 1),
            (finished.as_str(), "go depth 1", 1),
        ] {
            let output = session(
                &[RuleCode::R1, RuleCode::E2],
                &format!("{prelude}{input}\n"),
            );
            assert_eq!(
                error_lines(&output).len(),
                errors,
                "prelude={prelude:?}, input={input}: {output}"
            );
            assert!(
                bestmoves(&output).is_empty(),
                "prelude={prelude:?}, input={input}: {output}"
            );
        }
    }

    #[test]
    fn go_depth_returns_a_deterministic_legal_bestmove_without_applying_it() {
        // [`crate::protocol::engine`]の表に従い、usinewgameなしのpositionとgoを受理する。
        // 理由は`docs/adr/commit-pending-rules-on-first-position.md`にある。
        // [`UsiProtocol::start_go`]は複製で探索し、Engineの対局へ適用しない。
        // 理由は`docs/adr/keep-searched-moves-out-of-the-usi-game.md`にある。
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

    // モジュールdocのコマンド表に従い、固定応答、未知コマンド、無応答、quitを検査する。
    #[test]
    fn commands_return_the_documented_response() {
        for (input, expected) in [
            (
                "position startpos\ngo mate 10\ngo mate\n",
                "checkmate notimplemented\ncheckmate notimplemented\n".to_owned(),
            ),
            (
                "foobar baz\nposition startpos\nstate\nquit\nusi\n",
                format!(
                    "info string error: unknown command foobar\nstate rules L0,P0,R1,E0 board {INITIAL_BOARD} status ongoing\n"
                ),
            ),
            ("debug on\nregister later\n", String::new()),
        ] {
            assert_eq!(session(&[RuleCode::R1], input), expected, "input={input:?}");
        }
    }

    /// 出力行を1行ずつチャネルへ流す観測用ライター。
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
        // [`ActiveSearch`]に従い、go infiniteではstopまでbestmoveを保留する。
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
    fn commands_arriving_during_search_apply_after_bestmove() {
        // モジュールdocの探索中の入力に従い、待ち行列の入力をbestmoveの後に処理する。
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
        // 補助ワーカーの採用深さは非決定的なので数値を固定せず、
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

    // [`write_bestmove`]と[`write_info`]の形式を、1ワーカーと複数ワーカーで検査する。
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

    // 受信側harness::engine::usiの語彙とwrite_bestmoveの送信行を照合する。
    #[test]
    fn stop_reasons_have_distinct_protocol_words() {
        let position = Position::initial();
        let mv = usi::parse(&position, "6i6h").unwrap();
        let mut words = HashSet::new();
        for (reason, expected) in [
            (StopReason::DepthCompleted, "depth"),
            (StopReason::NodeLimit, "nodes"),
            (StopReason::SoftLimit, "soft"),
            (StopReason::HardLimit, "hard"),
            (StopReason::ExternalStop, "external"),
        ] {
            let mut output = Vec::new();
            write_bestmove(&mut output, &position, mv, None, reason, false).unwrap();
            let output = String::from_utf8(output).unwrap();
            assert_eq!(
                output,
                format!("info string stop {expected}\nbestmove 6i6h\n"),
                "reason={reason:?}"
            );
            assert!(
                words.insert(stop_reason_text(reason)),
                "duplicate word for {reason:?}"
            );
        }
    }

    #[test]
    fn hash_rejects_invalid_sizes_and_resizes_without_changing_the_game() {
        // [`UsiProtocol::handle_setoption`]による非探索中のリサイズと、極大の外部設定値によるオーバーフローの回帰を検証する。
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
        // モジュールdocの契約に従い、USIは自発的な終局裁定を出力しない。
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
        // USIには自発的な裁定通知がないため、裁定はrun終了後のGameStatusで検証する。
        assert_eq!(
            engine.status(),
            GameStatus::Finished(GameResult::Win {
                winner: Color::Black,
                reason: WinReason::RoyalCapture,
            })
        );

        // Finished局面への同一position全列の再送は終局通知や重複出力を生まない。
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

    /// [`UsiProtocol::handle_diagnostic`]の出力形式: d以外の成功は接頭辞と終端を持つ。
    #[test]
    fn diagnostics_frame_success_and_reject_uncommitted_positions() {
        for command in ["d", "eval", "tt"] {
            let mut engine = Engine::new(parse_rule_set("L0,P0,R1,E2").unwrap()).unwrap();
            let mut protocol = UsiProtocol::new(&engine);
            let error = run(&mut protocol, &mut engine, &format!("{command}\n"));
            assert_eq!(error.lines().count(), 1);
            assert!(error.starts_with("info string error: "));
            assert!(!error.contains("info string end"));
            assert_eq!(
                run(
                    &mut protocol,
                    &mut engine,
                    "setoption name USI_Hash value 1\nposition startpos\n"
                ),
                ""
            );
            let before = engine.game().position().clone();
            let output = run(&mut protocol, &mut engine, &format!("{command}\n"));
            if command == "d" {
                assert!(output.lines().all(|line| !line.starts_with("info string")));
                assert!(
                    output
                        .lines()
                        .last()
                        .unwrap()
                        .starts_with("rights-zobrist ")
                );
            } else {
                assert!(output.lines().all(|line| line.starts_with("info string ")));
                assert_eq!(output.lines().last(), Some("info string end"));
            }
            assert!(!output.contains("error"));
            assert_eq!(engine.game().position(), &before);
        }
        let output = session(&[RuleCode::R1], "position startpos\ntt\n");
        assert_eq!(output.lines().count(), 1);
        assert!(output.starts_with("info string error: "));
    }

    /// [`UsiProtocol::handle_diagnostic`]の受付順序: 探索中の3コマンドは探索終了後に処理する。
    #[test]
    fn diagnostics_queued_during_search_follow_bestmove() {
        let output = session(
            &[RuleCode::R1],
            concat!(
                "setoption name USI_Hash value 1\nposition startpos\n",
                "go ponder depth 1\nd\neval\ntt\nstop\n"
            ),
        );
        let bestmove = output.find("bestmove ").unwrap();
        for marker in ["\nsfen ", "info string evaluation ", "info string current "] {
            assert!(bestmove < output.find(marker).unwrap(), "{output}");
        }
        assert_eq!(
            output
                .lines()
                .filter(|line| *line == "info string end")
                .count(),
            2
        );
        assert!(!output.contains("error:"));
    }

    /// [`diagnostic_position`]の契約: 成り権保留と先獅子を含む拡張SFENがキーまで往復する。
    #[test]
    fn diagnostic_sfen_round_trips_all_position_state() {
        let mut engine = Engine::new(parse_rule_set("L1,L2,P1,R1,E2").unwrap()).unwrap();
        let mut protocol = UsiProtocol::new(&engine);
        let sfen = "k11/12/4P7/12/9g2/5+o3n2/12/9R2/12/12/12/11K b 7f 41 8c";
        let output = run(
            &mut protocol,
            &mut engine,
            &format!("position sfen {sfen}\nd\n"),
        );
        assert!(!output.contains("error:"), "{output}");
        let original = engine.game().position().clone();
        assert!(output.contains(&format!("zobrist {:016x}\n", original.zobrist())));
        assert!(output.contains(&format!(
            "rights-zobrist {:016x}\n",
            original.rights_zobrist()
        )));
        assert!(!original.promotion_deferred().is_empty());
        assert!(original.lion_capture_square().is_some());
        let emitted = output
            .lines()
            .find_map(|line| line.strip_prefix("sfen "))
            .unwrap();
        assert_eq!(emitted, sfen);
        assert!(output.contains("\nmove 41\n"));
        assert!(output.contains("\nlion 7f\n"));
        assert!(output.contains("\npromotion-deferred 8c\n"));
        assert!(output.contains("+o"));
        assert_eq!(
            run(
                &mut protocol,
                &mut engine,
                &format!("position sfen {emitted}\n")
            ),
            ""
        );
        let restored = engine.game().position();
        assert_eq!(original, *restored);
        assert_eq!(original.zobrist(), restored.zobrist());
        assert_eq!(original.rights_zobrist(), restored.rights_zobrist());
        let rows: Vec<_> = output.lines().skip(2).step_by(2).take(12).collect();
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.matches('|').count(), 13);
            assert!(row.ends_with(&format!("| {}", char::from(b'a' + index as u8))));
        }
    }

    /// debugging.mdの駒表記とRULES.md第5条の初期配置を検証する。
    /// 枠線の幅と空白の数は契約ではない。
    #[test]
    fn diagnostic_startpos_board_matches_rules() {
        let output = session(&[RuleCode::R1], "position startpos\nd\n");
        let expected = [
            "^香 ^猛 ^銅 ^銀 ^金 ^醉 ^玉 ^金 ^銀 ^銅 ^猛 ^香",
            "^反 ・ ^角 ・ ^盲 ^鳳 ^麒 ^盲 ・ ^角 ・ ^反",
            "^横 ^竪 ^飛 ^馬 ^龍 ^奔 ^獅 ^龍 ^馬 ^飛 ^竪 ^横",
            "^歩 ^歩 ^歩 ^歩 ^歩 ^歩 ^歩 ^歩 ^歩 ^歩 ^歩 ^歩",
            "・ ・ ・ ^仲 ・ ・ ・ ・ ^仲 ・ ・ ・",
            "・ ・ ・ ・ ・ ・ ・ ・ ・ ・ ・ ・",
            "・ ・ ・ ・ ・ ・ ・ ・ ・ ・ ・ ・",
            "・ ・ ・ 仲 ・ ・ ・ ・ 仲 ・ ・ ・",
            "歩 歩 歩 歩 歩 歩 歩 歩 歩 歩 歩 歩",
            "横 竪 飛 馬 龍 獅 奔 龍 馬 飛 竪 横",
            "反 ・ 角 ・ 盲 麒 鳳 盲 ・ 角 ・ 反",
            "香 猛 銅 銀 金 王 醉 金 銀 銅 猛 香",
        ];
        let rows: Vec<_> = output.lines().filter(|line| line.contains('|')).collect();
        assert_eq!(rows.len(), 12);
        for (row, expected) in rows.iter().zip(expected) {
            let cells: Vec<String> = row
                .split('|')
                .skip(1)
                .take(12)
                .map(|cell| cell.chars().filter(|c| !c.is_whitespace()).collect())
                .collect();
            assert_eq!(cells, expected.split_whitespace().collect::<Vec<_>>());
        }
        assert!(output.lines().any(|line| line.starts_with("sfen ")));
    }

    /// docs/guides/debugging.mdの「診断の道具」は後手に^、成駒に+を付ける。
    #[test]
    fn diagnostic_board_shows_promoted_kinds_and_owners() {
        use minase_core::test_util::{position_from_codes, sq};
        let position = position_from_codes(
            Color::Black,
            &[
                (
                    sq(0, 11),
                    PieceCode::new(Color::White, PieceKind::King).unwrap(),
                ),
                (
                    sq(11, 0),
                    PieceCode::new(Color::Black, PieceKind::King).unwrap(),
                ),
                (
                    sq(0, 6),
                    PieceCode::new_promoted(Color::Black, PieceKind::Rook).unwrap(),
                ),
                (
                    sq(1, 6),
                    PieceCode::new_promoted(Color::White, PieceKind::GoldGeneral).unwrap(),
                ),
                (
                    sq(2, 6),
                    PieceCode::new_promoted(Color::Black, PieceKind::CrownPrince).unwrap(),
                ),
                (
                    sq(3, 6),
                    PieceCode::new_promoted(Color::White, PieceKind::DrunkElephant).unwrap(),
                ),
            ],
        );
        let sfen = minase_core::notation::sfen::to_sfen(&position);
        let output = session(&[RuleCode::R1], &format!("position sfen {sfen} - 1\nd\n"));
        assert!(!output.contains("error"), "{output}");
        let cells: Vec<String> = output
            .lines()
            .filter(|line| line.contains('|'))
            .flat_map(|row| row.split('|').skip(1).take(12))
            .map(|cell| cell.chars().filter(|c| !c.is_whitespace()).collect())
            .collect();
        for piece in ["+飛", "^+金", "+太", "^+醉"] {
            assert_eq!(
                cells.iter().filter(|cell| cell.as_str() == piece).count(),
                1,
                "{output}"
            );
        }
    }

    /// [`diagnostic_evaluation`]の契約: 表示評価は全計算と一致し、盤面の各行と注記を表示する。
    #[test]
    fn diagnostic_evaluation_matches_full_evaluation_and_formats_rows() {
        let weights = nnue::weights().unwrap();
        let mut engine = Engine::new(parse_rule_set("L0,P0,R1,E2").unwrap()).unwrap();
        let mut protocol = UsiProtocol::new(&engine);
        let output = run(
            &mut protocol,
            &mut engine,
            &format!("position sfen {INITIAL_BOARD} - 1\neval\n"),
        );
        let score = nnue::evaluate(&weights, engine.game().position());
        assert!(
            output.contains(&format!("info string evaluation {score}\n")),
            "{output}"
        );
        assert!(output.contains("integer truncation"));
        assert!(output.contains("info string lion "));
        let rows: Vec<_> = output.lines().skip(3).take(12).collect();
        assert_eq!(rows.len(), 12);
        for row in rows {
            assert_eq!(row.split_whitespace().count(), 15);
        }
    }

    /// [`diagnostic_table`]の契約: 空項目と全合法手を列挙し、終局後は現局面だけを表示する。
    #[test]
    fn diagnostic_table_lists_legal_children_and_no_finished_children() {
        let mut engine = Engine::new(parse_rule_set("L0,P0,R1,E2").unwrap()).unwrap();
        let mut protocol = UsiProtocol::new(&engine);
        run(
            &mut protocol,
            &mut engine,
            "setoption name USI_Hash value 1\nposition startpos\n",
        );
        let expected: HashSet<_> = engine
            .game()
            .legal_moves()
            .iter()
            .map(|&mv| {
                format!(
                    "info string {} none",
                    usi::text_generated(engine.game().position(), mv)
                )
            })
            .collect();
        let output = run(&mut protocol, &mut engine, "tt\n");
        assert_eq!(output.lines().next(), Some("info string current none"));
        let actual: HashSet<_> = output
            .lines()
            .skip(1)
            .take(expected.len())
            .map(str::to_owned)
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(output.lines().count(), expected.len() + 3);
        assert_eq!(output, run(&mut protocol, &mut engine, "tt\n"));
        let output = run(
            &mut protocol,
            &mut engine,
            &format!("position sfen {ROYAL_SFEN} moves 7g7d\nd\neval\ntt\n"),
        );
        assert!(!output.contains("error:"));
        let tail = output.split("info string current none\n").nth(1).unwrap();
        assert_eq!(tail.lines().count(), 2);
    }

    /// [`diagnostic_table`]の契約: 探索値、静的評価、最善手、および項目なしを指定語彙で表示する。
    #[test]
    fn diagnostic_hit_formats_all_fields() {
        let position = Position::initial();
        let generator = MoveGenerator::standard();
        let mv = usi::parse(&position, "6i6h").unwrap();
        for (bound, text, best_move) in [
            (search::Bound::Exact, "exact", Some(mv)),
            (search::Bound::Lower, "lower", Some(mv)),
            (search::Bound::Upper, "upper", Some(mv)),
            (search::Bound::Exact, "exact", None),
        ] {
            let hit = search::Hit {
                best_move,
                static_eval: None,
                value: Some(search::TtValue {
                    score: 123,
                    depth: 7,
                    bound,
                }),
                is_pv: false,
            };
            assert_eq!(
                diagnostic_hit(&position, &generator, Some(hit)).unwrap(),
                format!(
                    "depth 7 bound {text} score 123 bestmove {} static-eval none",
                    if best_move.is_some() { "6i6h" } else { "none" }
                )
            );
        }
        let hit = search::Hit {
            best_move: None,
            static_eval: Some(-321),
            value: None,
            is_pv: false,
        };
        assert_eq!(
            diagnostic_hit(&position, &generator, Some(hit)).unwrap(),
            "search none bestmove none static-eval -321"
        );
        assert_eq!(diagnostic_hit(&position, &generator, None).unwrap(), "none");
    }

    #[test]
    fn moves_returns_the_legal_move_set_as_one_line() {
        // [`UsiProtocol::handle_moves`]の契約: Game::legal_moves()の全要素を既存USI表記で1行、順序は契約外。
        let output = session(
            &[RuleCode::R1],
            "position startpos\nmoves\nstate\nmoves\nstate\n",
        );
        let moves = moves_sets(&output);
        let states = state_lines(&output);

        // [`UsiProtocol::handle_moves`]の契約に従い、期待集合はGame::legal_moves()のUSI表記から作る。
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
        // [`UsiProtocol::handle_moves`]と[`UsiProtocol::handle_state`]の契約: AwaitingStartとFinishedのmovesは固定エラー行、stateはFinishedで応答する非対称。
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
    fn state_status_vocabulary_matches_the_contract() {
        // [`UsiProtocol::handle_state`]のstatus語彙に従い、勝因の語彙と勝者の色を検査する。
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
        // [`UsiProtocol::handle_moves`]と[`UsiProtocol::handle_state`]の契約: stateはInGameとFinishedで応答し、AwaitingStartではエラー。次局はactive規則を返す。
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

    // [`parse_go_config`]と[`UsiProtocol::write_handshake`]の先読みの契約を検査する。
    #[test]
    fn ponder_accepts_finite_limits_and_ignores_usi_ponder_option() {
        for (args, depth, nodes, movetime, clock) in [
            ("ponder depth 1", Some(1), None, None, None),
            ("ponder nodes 100", None, Some(100), None, None),
            (
                "ponder btime 1000 wtime 2000 binc 7 winc 9 byoyomi 50",
                None,
                None,
                None,
                Some((1000, 7, 50, Some(2000))),
            ),
            ("ponder movetime 20", None, None, Some(20), None),
            (
                "depth 2 ponder nodes 100 movetime 20 btime 1000",
                Some(2),
                Some(100),
                Some(20),
                Some((1000, 0, 0, None)),
            ),
        ] {
            let parsed = parse_go_config(
                &args.split_whitespace().collect::<Vec<_>>(),
                Color::Black,
                17,
                30,
            )
            .unwrap();
            assert!(!parsed.is_infinite(), "{args}");
            assert_eq!(parsed.depth(), depth, "{args}");
            assert_eq!(parsed.nodes(), nodes, "{args}");
            assert_eq!(parsed.movetime_ms(), movetime, "{args}");
            assert_eq!(
                parsed.clock().map(|c| {
                    (
                        c.remaining_ms(),
                        c.increment_ms(),
                        c.byoyomi_ms(),
                        c.opponent_remaining_ms(),
                    )
                }),
                clock,
                "{args}"
            );
            if let Some(clock) = parsed.clock() {
                assert_eq!(clock.ply(), 17);
                assert_eq!(clock.byoyomi_margin_ms(), 30);
            }
        }
        let output = session(&[RuleCode::R1], "position startpos\ngo ponder infinite\n");
        assert_eq!(error_lines(&output).len(), 1, "{output}");
        assert!(bestmoves(&output).is_empty());
        let output = session(
            &[RuleCode::R1],
            "usi\nsetoption name USI_Ponder value false\nposition startpos\ngo depth 2\n",
        );
        assert!(!output.contains("option name USI_Ponder"));
        assert!(error_lines(&output).is_empty());
        if let Some((best_move, ponder_move)) = assert_bestmove_matches_final_pv(&output) {
            assert_startpos_ponder_is_legal(best_move, ponder_move);
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

    // [`parse_go_config`]の時計とThreadsを実際に入力し、通常探索と先読み的中後の着手と予想手を検査する。
    #[test]
    fn four_thread_byoyomi_usi_returns_legal_bestmove_and_ponder_before_deadline() {
        for ponder in [false, true] {
            ponder_dialogue(|commands, lines| {
                for command in [
                    "setoption name Threads value 4",
                    "setoption name USI_Hash value 1",
                    "setoption name ByoyomiMargin value 30",
                    "position startpos",
                    "moves",
                ] {
                    commands.send(Ok(command.into())).unwrap();
                }
                let legal = moves_sets(&receive_line(lines)).remove(0);
                let go = if ponder {
                    "go ponder btime 0 wtime 0 byoyomi 300"
                } else {
                    "go btime 0 wtime 0 byoyomi 300"
                };
                let mut origin = std::time::Instant::now();
                commands.send(Ok(go.into())).unwrap();
                if ponder {
                    let line = receive_line(lines);
                    assert!(line.starts_with("info depth "), "{line}");
                    origin = std::time::Instant::now();
                    commands.send(Ok("ponderhit".into())).unwrap();
                }
                let best = loop {
                    let line = receive_line(lines);
                    assert!(!line.starts_with("info string error:"), "{line}");
                    if line.starts_with("bestmove ") {
                        break line;
                    }
                };
                // 締切270 msにノード周期の検査と実行遅延700 msを許容する。
                assert!(
                    origin.elapsed() <= Duration::from_millis(970),
                    "{:?}",
                    origin.elapsed()
                );
                assert_legal_bestmove(&best, &legal);
                // 完了深さは負荷に依存するので、予想手の有無は契約にしない。
                // 予想手を出したときは、着手後の局面で合法であることを検査する。
                let words: Vec<_> = best.split_whitespace().collect();
                if words.len() == 4 {
                    assert_eq!(words[2], "ponder");
                    commands
                        .send(Ok(format!("position startpos moves {}", words[1])))
                        .unwrap();
                    commands.send(Ok("moves".into())).unwrap();
                    let replies = moves_sets(&receive_line(lines)).remove(0);
                    assert!(replies.contains(words[3]), "{best}");
                } else {
                    assert_eq!(words.len(), 2, "{best}");
                }
                commands.send(Ok("quit".into())).unwrap();
            });
        }
    }

    // [`ActiveSearch`]の結果保留と[`write_bestmove`]の停止理由を検査する。
    #[test]
    fn ponder_channel_withholds_completed_result_until_hit() {
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
            commands.send(Ok("ponderhit".into())).unwrap();
            assert_eq!(receive_line(lines), "info string stop depth\n");
            let best = receive_line(lines);
            let result = format!("{final_pv}{best}");
            let ponder = assert_bestmove_matches_final_pv(&result);
            assert_eq!(receive_line(lines), "readyok\n");
            // 外形でも予想手の合法性を確認する。
            if let Some((best_move, ponder_move)) = ponder {
                commands
                    .send(Ok(format!("position startpos moves {best_move}")))
                    .unwrap();
                commands.send(Ok("moves".into())).unwrap();
                assert!(
                    receive_line(lines)
                        .split_whitespace()
                        .any(|word| word == ponder_move)
                );
            }
        });
    }

    // [`ActiveSearch`]とモジュールdocの探索中の入力の扱いを検査する。
    // [`UsiProtocol::should_resign`]の適用範囲: 保留結果の破棄では投了も出力しない。
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
            let held: String = lines.try_iter().collect();
            assert!(bestmoves(&held).is_empty(), "{held}");
            let mut pending = VecDeque::new();
            if command == "eof" {
                protocol.discard_search(&mut active).unwrap();
            } else {
                protocol
                    .handle_searching_line(&engine, &mut active, &mut pending, command, &mut output)
                    .unwrap();
            }
            assert!(active.is_none());
            let released: String = lines.try_iter().collect();
            if matches!(command, "ponderhit" | "stop") {
                assert_eq!(bestmoves(&released).len(), 1, "{released}");
                assert!(released.contains("info string stop depth\n"), "{released}");
                let result = format!("{held}{released}");
                if let Some((best_move, ponder_move)) = assert_bestmove_matches_final_pv(&result) {
                    assert_startpos_ponder_is_legal(best_move, ponder_move);
                }
            } else {
                assert!(bestmoves(&released).is_empty(), "{released}");
            }
        }
    }

    // モジュールdocの探索中の入力の扱いを検査する。
    // [`UsiProtocol::discard_search`]と[`UsiProtocol::should_resign`]に従う。
    // 探索結果の破棄ではbestmoveも投了も出力しない。
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
    }

    // [`Protocol::run`]の逐次経路の契約を検査する。
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

    // [`validated_ponder_move`]の出所と検査に従う。
    // 理由は`docs/adr/take-ponder-move-from-the-pv.md`にある。
    #[test]
    fn ponder_prediction_uses_adopted_pv_and_rejects_illegal_or_finishing_moves() {
        let output = session(
            &[RuleCode::R1],
            "setoption name Threads value 2\nposition startpos\ngo depth 2\n",
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
        use minase_core::PieceKind;
        use minase_core::test_util::{position, sq};
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

    // [`UsiProtocol::handle_position`]の末尾置換を検査する。
    // 理由は`docs/adr/apply-position-by-token-difference.md`にある。
    #[test]
    fn ponder_last_move_replacement_matches_replay_and_is_atomic() {
        let mut engine = make_engine(&[RuleCode::R2, RuleCode::E2]);
        let mut protocol = UsiProtocol::new(&engine);
        let setup = run(
            &mut protocol,
            &mut engine,
            "position startpos moves 5i5h 5d5e 6i6h\n",
        );
        assert!(setup.is_empty(), "{setup}");
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
}
