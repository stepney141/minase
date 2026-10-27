//! Chess Engine Communication Protocol（CECP、XBoard）のアダプター。
//!
//! CECPのWhiteは先手で内部の[`Color::Black`]、Blackは後手で[`Color::White`]に当たる。
//! 手番文字と結果の符号の読み替えは本モジュールに閉じる。
//! 待機中の入力は次の表に従う。
//!
//! | コマンド | 応答と処理 |
//! | --- | --- |
//! | `xboard`、`accepted` | 無応答で無視する。 |
//! | `protover` | `CecpProtocol::write_features`の宣言列を返す。 |
//! | `rejected` | `setboard`、`usermove`、`ping`なら`tellusererror minase requires the <feature> feature`を出して終了し、他は無視する。 |
//! | `variant` | `chu`だけを受理し、他は`Error (unsupported variant): <名前>`を返して継続する。 |
//! | `new` | `handle_new`で初期局面の対局を始める。 |
//! | `force`、`go` | `PlayMode`の担当手番と応手状態を変える。 |
//! | `setboard`、`usermove` | `handle_setboard`と`handle_usermove`で局面や着手を適用する。 |
//! | `ping N` | `pong N`を返す。 |
//! | `result` | 対局状態機械の`EndGame`で開始待ちへ戻り、裁定の食い違いに異議を唱えない。 |
//! | `option`、`memory`、`cores` | `handle_option`、`handle_memory`、`handle_cores`で設定を検査する。 |
//! | `time`、`otim`、`level`、`st`、`sd` | `TimeControl`の契約に従って両者の時計と探索制限を更新する。 |
//! | `?` | 待機中は無応答で無視する。 |
//! | `quit` | 無応答で終了する。 |
//! | `easy`、`hard`、`post`、`nopost`、`random`、`computer`、`name`、`hint`、`draw` | 対応する機能を持たず、状態を変えずに無視する。 |
//! | `undo`、`remove`、`analyze` | 無視すると盤面やモードが食い違うため、`Error (command not supported): <コマンド>`を返す。 |
//! | それ以外 | `Error (unknown command): <第1語>`を返す。 |
//!
//! 先読み、引き分け提案、思考情報の出力、およびXBoardのhighlightによる対話入力は実装しない。
//! 人間との対局は自作GUIが担う。

use std::collections::VecDeque;
use std::io::{self, BufRead, Write};
use std::num::NonZeroUsize;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use crate::search::{
    self, ClockLimits, SearchEvent, SearchHandle, SearchLimits, SearchOptionValue, SearchSnapshot,
    Searcher,
};
use minase_core::game::{DrawReason, GameResult, IllegalMoveCause, WinReason};
use minase_core::notation::cecp;
use minase_core::notation::sfen::{SetupPosition, parse_extended_sfen};
use minase_core::piece::Color;
use minase_core::position::Position;
use minase_core::rules::parse_rule_set;

use super::Protocol;
use super::engine::{
    Engine, EngineCommand, EngineLifecycle, EngineReply, RejectReason, canonical_rules_text,
};

/// CECPから設定できる置換表容量の上限(MiB)。
const MAX_MEMORY_MB: usize = 65_536;

/// 中将棋用のCECPアダプター。
pub struct CecpProtocol {
    /// feature宣言のRuleSet既定値に使う起動時規則の正準表記。
    startup_rules_text: String,
    /// CECPの着手応答モード。
    play_mode: PlayMode,
    /// 対局内で持ち越す探索器。探索スレッドへ貸し出している間は`None`。
    searcher: Option<Searcher>,
    /// 次に開始する探索へ割り当てる識別子。
    next_search_id: u64,
    /// CECPの時間コマンドをミリ秒へ正規化した値。
    time_control: TimeControl,
}

/// 担当手番と、自動で応手するかどうかを表す状態。
///
/// `new`は担当を後手として`Playing`へ移る。
/// `force`は`Force`へ移り、以後は両陣営の`usermove`を適用するだけで着手を出さない。
/// `go`は対局中だけ受理し、現在手番を担当とする`Playing`へ移って探索を始める。
/// 対局中以外の`go`は`Error (command not legal now): go`で拒否する。
/// `usermove`の後は、`Playing`で対局が続き、手番が担当と一致するときだけ探索する。
/// 探索の手が拒否された場合は`tellusererror engine move rejected: <理由>`を出して`Force`へ移る。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PlayMode {
    /// 両陣営の着手を受信するだけで、エンジンは応手しない。
    Force,
    /// 指定された手番をエンジンが担当する。
    Playing { engine_side: Color },
}

/// CECPの時間と深さを、探索層の単位に正規化して保持する。
///
/// `time`と`otim`は1/100秒を10倍してミリ秒にする。
/// `level MPS BASE INC`のBASEは分または`分:秒`で、秒は60未満とする。
/// INCは秒として小数第3位までをミリ秒へ写し、第4位以下は切り捨てる。
/// エンジン側の残り時間をBASE、加算をINCにし、保持していた相手の残り時間は消す。
/// MPSは区切り手数として構文だけ検査し、探索制限に残り手数の欄がないため使わない。
/// `st`は小数を許す1手の固定秒数、`sd`は1〜256の深さ上限である。
/// 不正な値は`Error (invalid command): <コマンド>`を返す。
/// `otim`は相手の残り時間として保持し、`time`との受信順にかかわらず探索へ渡す。
/// 新規対局では保持した相手の残り時間を消す。探索への変換は[`Self::search_limits`]に従う。
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
            play_mode: PlayMode::Force,
            searcher: Some(Searcher::alpha_beta()),
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
                self.play_mode = PlayMode::Force;
            }
            "go" => {
                if engine.lifecycle() != EngineLifecycle::InGame {
                    writeln!(output, "Error (command not legal now): go")?;
                } else {
                    self.play_mode = PlayMode::Playing {
                        engine_side: engine.game().position().side_to_move(),
                    };
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
            "cores" => self.handle_cores(&tokens[1..], output)?,
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
        let limits = match self.time_control.search_limits(engine.ply()) {
            Ok(Some(limits)) => limits,
            Ok(None) => {
                writeln!(output, "tellusererror search limits are not set")?;
                return Ok(None);
            }
            Err(error) => {
                writeln!(output, "tellusererror {error}")?;
                return Ok(None);
            }
        };
        let game = engine.game();
        let snapshot = match SearchSnapshot::from_game(game) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                writeln!(output, "tellusererror {error}")?;
                return Ok(None);
            }
        };
        let search_id = self.next_search_id;
        self.next_search_id = self.next_search_id.wrapping_add(1);
        let searcher = self
            .searcher
            .take()
            .expect("idle protocol must own its searcher");
        let handle = match searcher.start(snapshot, limits, search_id, false) {
            Ok(handle) => handle,
            Err(failure) => {
                self.searcher = Some(failure.searcher);
                writeln!(output, "tellusererror {}", failure.error)?;
                return Ok(None);
            }
        };
        Ok(Some(ActiveSearch {
            context: SearchContext { id: search_id },
            handle,
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

    /// 探索中の`?`で停止し、その時点の最善手を通常の着手処理へ渡す。
    ///
    /// `force`、`result`、`new`、`quit`は探索を止めて結果を捨て、`move`を出さずに通常の処理へ戻す。
    /// 他のコマンドは`ping`も含めて待ち行列に積み、探索解放後に受信順で処理する。
    /// 探索結果を着手として返す場合、探索中の`ping`への`pong`は`move`より後になる。
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
        self.searcher = Some(join_search(search.handle)?);
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
                self.searcher = Some(join_search(search.handle)?);
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
        self.searcher = Some(join_search(search.handle)?);
        self.apply_engine_move(engine, best_move, output)
    }

    /// 探索結果を出力せずに停止し、置換表だけを回収する。
    fn discard_search(&mut self, active: &mut Option<ActiveSearch>) -> io::Result<()> {
        let Some(search) = active.take() else {
            return Ok(());
        };
        search.handle.request_stop();
        self.searcher = Some(join_search(search.handle)?);
        Ok(())
    }

    /// 探索の手を現局へ適用してから、着手、終局結果の順に出す。
    ///
    /// `ApplyMove`を受理した後、[`cecp::legs`]に従う`move <レグ>`を送る。
    /// 2レグなら非最終レグの末尾にコンマを付け、じっとは`@@@@`、成りの`+`は最終レグだけに付ける。
    /// `newly_finished`があれば最後に結果行を1回送る。
    /// 審判層が確定したルート合法手から選ぶ契約なので、拒否は探索とプロトコルの契約違反である。
    /// 拒否時の応答と状態遷移は[`PlayMode`]に従う。
    fn apply_engine_move(
        &mut self,
        engine: &mut Engine,
        best_move: minase_core::Move,
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
                self.play_mode = PlayMode::Force;
            }
        }
        output.flush()
    }

    /// `protover`へ次の順でfeature宣言を返す。
    ///
    /// ```text
    /// feature myname="minase <CARGO_PKG_VERSION>"
    /// feature variants="chu"
    /// feature setboard=1
    /// feature usermove=1
    /// feature ping=1
    /// feature colors=0
    /// feature sigint=0
    /// feature sigterm=0
    /// feature analyze=0
    /// feature time=1
    /// feature memory=1
    /// feature draw=0
    /// feature option="RuleSet -string <起動時の--rulesの正準表記>"
    /// feature smp=1
    /// feature done=1
    /// ```
    ///
    /// 時間と容量のコマンドを受けるため`time=1`と`memory=1`を宣言し、引き分け提案を扱わないため`draw=0`とする。
    /// 必須の`setboard`、`usermove`、`ping`が拒否された場合はセッションを続けない。
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
        writeln!(output, "feature smp=1")?;
        writeln!(output, "feature done=1")
    }

    /// 次局の規則を確定し、初期局面から対局を開始する。
    ///
    /// `NewGame`に続けて初形の`SetPosition`を適用し、`InGame`へ入る。
    /// CECPでは`new`の後に`setboard`なしで着手を受けるためである。
    /// 担当手番を後手にし、置換表とhistory表を空にする。
    /// 状態機械が拒否した場合は`Error (command not legal now): new`を返す。
    fn handle_new(&mut self, engine: &mut Engine, output: &mut dyn Write) -> io::Result<()> {
        if !matches!(
            engine.handle(EngineCommand::NewGame),
            EngineReply::Accepted { .. }
        ) {
            return writeln!(output, "Error (command not legal now): new");
        }

        let setup = SetupPosition::new(Position::initial(), None, 1)
            .expect("the standard initial setup is valid");
        if matches!(
            engine.handle(EngineCommand::SetPosition {
                setup,
                moves: Vec::new(),
            }),
            EngineReply::Accepted { .. }
        ) {
            self.play_mode = PlayMode::Playing {
                engine_side: Color::White,
            };
            self.time_control.opponent_remaining_ms = None;
            self.searcher
                .as_mut()
                .expect("idle protocol must own its searcher")
                .clear();
            Ok(())
        } else {
            writeln!(output, "Error (command not legal now): new")
        }
    }

    /// 盤面とCECP手番の2欄を必須として局面を設定する。
    ///
    /// 盤面はSFENと同じパーサへ渡し、手番の`w`は先手、`b`は後手として読み替える。
    /// XBoardのFENにはキャスリングなどの欄もあるため、第3欄以降は無視する。
    /// `<盤面> <SFENの手番> - 1`として解析し、先獅子の状態と成り権の保留を持たず、
    /// 次の手番号1、空の反復履歴から始める。CECPが運ばない状態の独自拡張はしない。
    /// 制約は`docs/research/protocols/hachu.md`第12節にある。
    ///
    /// 解析や局面検証の失敗は`tellusererror Illegal position`で拒否する。
    /// 解析できても終局後なら`Error (command not legal now): setboard`で拒否する。
    /// 対局中は現局を置き換え、担当手番を変えない。
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
        let setup = match parse_extended_sfen(&text, engine.position_rules().moves) {
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

    /// 受信した着手を適用し、終局時だけ結果行を1回出す。
    ///
    /// 通常の着手は[`cecp::parse`]、`@@@@`は[`representative_jitto`]で解析する。
    /// 解析失敗または動きの規則違反は`Illegal move: <表記>`、
    /// 反復規則R2とR3の禁止は`Illegal move (repetition): <表記>`とする。
    /// 解析できても開始前や終局後なら`Error (command not legal now): usermove <表記>`を返す。
    /// 着手で終局したら探索を始めず、継続時の自動応手は[`PlayMode`]に従う。
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
            EngineReply::Accepted { .. } => Ok(matches!(
                self.play_mode,
                PlayMode::Playing { engine_side }
                    if engine.lifecycle() == EngineLifecycle::InGame
                        && engine_side == engine.game().position().side_to_move()
            )),
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

    /// `option RuleSet=<値>`だけを処理し、他のオプション名は無視する。
    ///
    /// 値はUSIの規則オプションと同じ[`parse_rule_set`]の文法で読む。
    /// 正当なら次局の規則だけを更新し、置換表とhistory表を空にする。
    /// 不正なら`Error (invalid option value): <受信行>`を返す。
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
            self.searcher
                .as_mut()
                .expect("idle protocol must own its searcher")
                .clear();
            Ok(())
        } else {
            writeln!(output, "Error (invalid option value): {line}")
        }
    }

    /// `memory MB`を1〜65,536 MiBとして受理し、置換表を確保または作り直す。
    ///
    /// 成功時はhistory表を空にする。
    /// 不正な値は`Error (invalid command): memory`、確保失敗は`Error (cannot allocate memory): <MB>`とする。
    /// このコマンドを受けていなければ、探索開始時に既定の256 MiBで確保する。
    fn handle_memory(&mut self, tokens: &[&str], output: &mut dyn Write) -> io::Result<()> {
        let Some(size_mb) = tokens
            .first()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|&size| (1..=MAX_MEMORY_MB).contains(&size))
        else {
            return writeln!(output, "Error (invalid command): memory");
        };
        let result = self
            .searcher
            .as_mut()
            .expect("idle protocol must own its searcher")
            .set_option("USI_Hash", SearchOptionValue::Spin(size_mb));
        if result.is_err() {
            return writeln!(output, "Error (cannot allocate memory): {size_mb}");
        }
        Ok(())
    }

    /// `cores N`を1〜256の探索ワーカー数として受理する。
    /// 変更時は置換表を空にし、history表をその数で作り直す。
    /// 不正な値は`Error (invalid command): cores`で拒否する。
    fn handle_cores(&mut self, tokens: &[&str], output: &mut dyn Write) -> io::Result<()> {
        let Some(threads) = tokens.first().and_then(|value| parse_cores(value)) else {
            return writeln!(output, "Error (invalid command): cores");
        };
        let searcher = self
            .searcher
            .as_mut()
            .expect("idle protocol must own its searcher");
        if searcher.threads() != threads {
            if searcher
                .set_option("Threads", SearchOptionValue::Spin(threads.get()))
                .is_err()
            {
                return writeln!(output, "Error (invalid command): cores");
            }
            searcher.clear();
        }
        Ok(())
    }

    /// [`TimeControl`]の契約に従い、`time`または`otim`の1/100秒値をミリ秒へ正規化する。
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

    /// [`TimeControl`]の`level`文法に従い、エンジン側の残り時間と加算をミリ秒で更新する。
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
        self.time_control.opponent_remaining_ms = None;
        self.time_control.increment_ms = increment_ms;
        Ok(())
    }

    /// [`TimeControl`]の`st`文法に従い、1手の固定時間をミリ秒で保持する。
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

    /// [`SearchLimits::new`]と同じ1〜256の深さ上限を検査して保持する。
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
    /// 自分と相手の残り時間、加算、秒読み0、および[`Engine::ply`]から探索の時計を作る。
    ///
    /// 残り時間がなければ時計は作らず、保持した`st`と`sd`を併せて制限にする。
    /// 制限が1つもなければ`None`を返し、呼び出し側は`tellusererror search limits are not set`を出す。
    /// 時計があり、相手の残り時間も`st`による固定時間もなければ、探索を始める時点で
    /// [`SearchLimits::new`]が拒否する。
    /// `level`の初期持ち時間を相手の残り時間として補わない。
    fn search_limits(&self, ply: u32) -> Result<Option<SearchLimits>, search::SearchError> {
        let clock = self
            .engine_remaining_ms
            .map(|remaining_ms| {
                ClockLimits::new(
                    remaining_ms,
                    self.increment_ms,
                    0,
                    self.opponent_remaining_ms,
                    ply,
                )
            })
            .transpose()?;
        if self.depth.is_none() && self.movetime_ms.is_none() && clock.is_none() {
            return Ok(None);
        }
        SearchLimits::new(self.depth, None, self.movetime_ms, clock).map(Some)
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

/// 探索スレッドの終了を待ち、貸し出した探索器を回収する。
fn join_search(handle: SearchHandle) -> io::Result<Searcher> {
    handle
        .join()
        .map_err(|error| io::Error::other(error.to_string()))
}

/// 1以上256以下のワーカー数を解析する。
fn parse_cores(value: &str) -> Option<NonZeroUsize> {
    value
        .parse::<usize>()
        .ok()
        .and_then(NonZeroUsize::new)
        .filter(|threads| threads.get() <= 256)
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

/// `@@@@`へ割り当てる正準じっとを、移動元の内部密番号が最小の合法手から選ぶ。
///
/// 候補は`mid`なし、移動元と移動先が同じ、不成の手である。
/// 居喰いは駒を取って局面を変えるため含めない。
/// どの候補も同じ後続局面と反復キーを生むが、R1の攻撃的着手の判定は動かした駒の到達升を見るため、
/// 代表の選び方が裁定に影響し得る。これは移動元と経由升を運ばないCECP表記に由来する、R1対局の既知の制限である。
fn representative_jitto(engine: &Engine) -> Option<minase_core::Move> {
    engine
        .game()
        .legal_moves()
        .into_iter()
        .filter(|mv| mv.mid.is_none() && mv.to == mv.from && !mv.promote)
        .min_by_key(|mv| mv.from.dense_index())
}

/// `<符号> {<理由>}`の1行で確定した結果を出力する。
///
/// 先手勝ちは`1-0`、後手勝ちは`0-1`、引き分けは`1/2-1/2`とする。
/// 勝ちの理由は[`win_reason_text`]、引き分けは[`draw_reason_text`]で固定語へ写す。
/// 呼び出し側は`EngineReply::Accepted`の`newly_finished`がある場合だけ送信し、
/// 終局後の後続コマンドから結果を作り直さない。
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

/// 勝利理由を結果行の固定語へ1対1に写す。
/// 王駒捕獲は`royal capture`、詰みは`checkmate`、合法手なしは`no legal moves`、
/// 反復は`repetition`、駒枯れは`piece exhaustion`、裸玉は`bare king`、投了は`resignation`である。
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

/// 引き分け理由を結果行の固定語へ1対1に写す。
/// 反復は`repetition`、駒枯れは`piece exhaustion`、裸玉は`bare kings`、合意は`agreement`である。
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
    use crate::protocol::UsiProtocol;
    use minase_core::rules::RuleCode;
    use minase_core::{Game, Rules};

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

    fn run(protocol: &mut CecpProtocol, engine: &mut Engine, input: &str) -> String {
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();
        protocol.run(engine, &mut input, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn time_and_otim_are_stored_in_either_order_and_normalized() {
        // [`TimeControl`]の換算は出力に現れないため、内部時計を直接観測して係数10を検査する。
        for input in ["time 6000\notim 123\n", "otim 123\ntime 6000\n"] {
            let mut engine = make_engine(&[RuleCode::R1]);
            let mut protocol = CecpProtocol::new(&engine);
            let output = run(&mut protocol, &mut engine, input);

            assert!(output.is_empty());
            let clock = protocol
                .time_control
                .search_limits(37)
                .unwrap()
                .unwrap()
                .clock()
                .unwrap();
            assert_eq!(clock.remaining_ms(), 60_000);
            assert_eq!(clock.opponent_remaining_ms(), Some(1_230));
            assert_eq!(clock.ply(), 37);
        }
    }

    #[test]
    fn time_without_otim_is_rejected_when_search_starts() {
        let output = session(&[RuleCode::R1], "new\ntime 6000\ngo\n");

        assert_eq!(
            output,
            "tellusererror search clock without byoyomi requires the opponent's remaining time\n"
        );
        assert!(move_lines(&output).is_empty());
    }

    #[test]
    fn new_and_level_clear_the_stored_opponent_time() {
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);
        let output = run(&mut protocol, &mut engine, "time 6000\notim 123\nnew\n");

        assert!(output.is_empty());
        assert_eq!(protocol.time_control.engine_remaining_ms, Some(60_000));
        assert_eq!(protocol.time_control.opponent_remaining_ms, None);
        assert!(matches!(
            protocol.time_control.search_limits(0),
            Err(search::SearchError::MissingOpponentClock)
        ));

        let output = run(&mut protocol, &mut engine, "otim 456\nlevel 40 5 0\n");
        assert!(output.is_empty());
        assert_eq!(protocol.time_control.engine_remaining_ms, Some(300_000));
        assert_eq!(protocol.time_control.opponent_remaining_ms, None);
        assert!(matches!(
            protocol.time_control.search_limits(0),
            Err(search::SearchError::MissingOpponentClock)
        ));
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
        // [`CecpProtocol::write_features`]の宣言値を検査する。
        // 宣言の行順は仕様が任意とするため（`docs/research/protocols/cecp.md`第4章）、done=1の終端以外は集合として検証する。
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
            "feature option=\"RuleSet -string L1,P0,R1,E2\"",
            "feature smp=1",
        ] {
            assert!(lines.contains(&expected), "missing: {expected}");
        }
        // 文字列値は二重引用符で囲む（[`CecpProtocol::write_features`]の契約）。
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("feature myname=\"minase ") && line.ends_with('"'))
        );
        // debug・highlight・san・reuseは宣言しない（[`CecpProtocol::write_features`]）。
        for absent in ["debug=", "highlight=", "san=", "reuse="] {
            assert!(!output.contains(absent), "unexpected: {absent}");
        }
    }

    #[test]
    fn cores_accepts_boundaries_and_rejects_invalid_values() {
        // [`CecpProtocol::handle_cores`]に従い、coresは1..=256だけを受理し、不正値は固定CECPエラーを
        // 返す。正当値は無応答である。
        assert_eq!(parse_cores("1").map(NonZeroUsize::get), Some(1));
        assert_eq!(parse_cores("256").map(NonZeroUsize::get), Some(256));
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);
        // 探索開始へ渡す値を受信後に観測し、設定の無視や丸めを検出する。
        for threads in [1, 2] {
            assert_eq!(
                run(&mut protocol, &mut engine, &format!("cores {threads}\n")),
                ""
            );
            assert_eq!(protocol.searcher.as_ref().unwrap().threads().get(), threads);
        }
        let output = run(
            &mut protocol,
            &mut engine,
            "cores\ncores nope\ncores 0\ncores 257\n",
        );

        assert_eq!(output, "Error (invalid command): cores\n".repeat(4));
        assert_eq!(protocol.searcher.as_ref().unwrap().threads().get(), 2);
    }

    #[test]
    fn required_feature_rejections_terminate_the_session() {
        // モジュールdocのコマンド表: 必須feature（setboard・usermove・ping）の拒否は
        // tellusererrorを出して終了し、他の拒否は無視する。
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
        // モジュールdocのコマンド表: variantはchuだけを受理する。
        // エラー本文の完全形は規範文書に明記されていないため、規定された先頭部だけを検証する。
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
        // モジュールdocの色の読み替えと[`CecpProtocol::handle_setboard`]の契約を検査する。
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
    }

    #[test]
    fn result_lines_take_the_white_viewpoint_and_appear_once() {
        // モジュールdocの色の読み替えと[`write_result`]の理由表、
        // [`CecpProtocol::handle_usermove`]の結果通知順を検査する。
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

        // R1の4回目の同一局面は引き分け裁定でありIllegal moveではない（RULES.md第31条R1）。
        // force相当（起動直後）の棋譜再生中でも裁定は生きている。
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
        // [`minase_core::notation::cecp::parse`]の契約: 受信はコンマ区切り単一文字列、第2レグの始点不一致は拒否。
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
        // 居喰い（RULES.md第12条）は@@@@ではなく明示レグで送る。
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
        // [`minase_core::notation::cecp::parse`]の契約: +は最終レグの末尾だけ、=は不成として受理。
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
        // 成れない着手への+も拒否する（RULES.md第26条）。
        assert_eq!(
            session(&[RuleCode::R1], "new\nforce\nusermove g4g5+\n"),
            "Illegal move: g4g5+\n"
        );
    }

    #[test]
    fn at_at_plays_a_canonical_jitto_or_is_rejected() {
        // [`representative_jitto`]の契約に従い、受理と拒否の応答を検証する。
        // 代表選択が局面遷移へ影響しないことは、指し手表記のテストで検証する。
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
        // [`CecpProtocol::apply_engine_move`]と[`minase_core::notation::cecp::legs`]の契約を検査する。
        // 探索の具体的な着手は評価依存のため固定せず、レグ連結をparseへ往復して合法手集合への所属だけを検証する。
        let output = session(&[RuleCode::R1], "sd 1\nnew\ngo\n");
        let lines = move_lines(&output);

        assert_eq!(output.lines().count(), lines.len());
        assert_eq!(logical_move_count(&output), 1);
        let payload: String = lines
            .iter()
            .map(|line| line.strip_prefix("move ").unwrap())
            .collect();

        let game = Game::new(Rules::ENGINE_DEFAULT);
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
        // [`CecpProtocol::handle_usermove`]の拒否形式: Movementは省略形、Repetitionは注記付き
        // である。RULES.md第27条第4項により、R2の禁止手は不合法な着手として扱う。
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
        // [`PlayMode`]とモジュールdocのコマンド表: usermove後、¬force∧継続中∧手番=担当なら探索する。
        // [`CecpProtocol::handle_new`]の契約: newは初期局面でInGameへ入り、エンジンは着手を自発しない。
        let output = session(&[RuleCode::R1], "sd 1\nnew\nusermove g4g5\n");

        // new直後に自発着手はなく、usermoveへの応手だけが出る。
        assert!(!output.is_empty());
        assert!(output.lines().all(|line| line.starts_with("move ")));
        // 自分が指した直後（手番が相手側）には探索しないため、応手はちょうど1手。
        assert_eq!(logical_move_count(&output), 1);
    }

    #[test]
    fn force_applies_both_sides_without_replies() {
        // [`PlayMode`]とモジュールdocのコマンド表: forceは着手を出さずusermoveを両陣営分適用する。
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
        // [`PlayMode`]とモジュールdocのコマンド表: goは継続中に限りforceを解除し現在手番を担当して探索する。
        let output = session(&[RuleCode::R1], "sd 1\nnew\nforce\nusermove g4g5\ngo\n");
        assert!(!output.is_empty());
        assert!(output.lines().all(|line| line.starts_with("move ")));

        // AwaitingStartのgoは探索せずエラーを返し、規範文書に明文のない本文は検証しない。
        let output = session(&[RuleCode::R1], "sd 1\ngo\n");
        assert_eq!(output.lines().count(), 1);
        assert!(!output.contains("move "));
        assert!(output.starts_with("Error") || output.starts_with("tellusererror"));
        // Finished後のgo拒否はan_engine_finishing_move_emits_move_then_result_onceが検証する。
    }

    #[test]
    fn go_without_search_limits_is_a_tellusererror() {
        // [`TimeControl::search_limits`]の契約: 探索制限のないgoは暗黙の既定値でフォールバック
        // せずtellusererrorを返す（暗黙フォールバックは禁止）。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);

        let output = run(&mut protocol, &mut engine, "new\ngo\n");
        assert_eq!(output.lines().count(), 1);
        assert!(output.starts_with("tellusererror"));
        assert!(!output.contains("move "));
    }

    #[test]
    fn an_engine_finishing_move_emits_move_then_result_once() {
        // [`CecpProtocol::apply_engine_move`]の契約: ApplyMove→move行→newly_finished時のみRESULT1回。
        // RESULT後のgoは拒否される。
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
        // [`PlayMode`]とモジュールdocのコマンド表: resultは外部要因を含む確定通知であり、エンジンは異議を唱えず
        // AwaitingStartへ戻る。
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
    fn stop_class_commands_discard_the_running_search() {
        // [`CecpProtocol::handle_searching_line`]の契約: 探索中のforce・result・new・quitは停止・破棄で、遅延した
        // move行が漏れない。
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
    fn move_now_and_queued_commands_complete_in_order() {
        // [`CecpProtocol::handle_searching_line`]の停止とpingの順序、および探索中のcoresを次の探索に適用する契約を検査する。
        let mut engine = make_engine(&[RuleCode::R1]);
        let mut protocol = CecpProtocol::new(&engine);
        assert_eq!(run(&mut protocol, &mut engine, "?\nping 42\n"), "pong 42\n");
        let output = run_queued(
            &mut protocol,
            &mut engine,
            "sd 256\nnew\ngo\ncores 2\nping 7\n?\nsd 1\nnew\ngo\n",
        );
        assert!(!output.contains("Error"));
        assert_eq!(logical_move_count(&output), 2);
        let lines: Vec<_> = output.lines().collect();
        let first_move = lines
            .iter()
            .position(|line| line.starts_with("move ") && !line.ends_with(','))
            .unwrap();
        let pong = lines.iter().position(|line| *line == "pong 7").unwrap();
        let last_move = lines
            .iter()
            .rposition(|line| line.starts_with("move "))
            .unwrap();
        assert!(first_move < pong);
        assert!(pong < last_move);
    }

    #[test]
    fn time_commands_are_accepted_and_normalized_to_milliseconds() {
        // [`TimeControl`]の単位: time/otimは1/100秒、levelは分・分:秒・加算秒、
        // stは秒、sdは深さで、正規化先はミリ秒のSearchLimits。
        let output = session(
            &[RuleCode::R1],
            "level 40 5 0\nlevel 0 0:30 1\nst 5\nsd 3\n",
        );
        assert!(output.is_empty());

        // CECPは受理した時間値を応答に含めない。探索の所要時間からも厳密な
        // 換算値は復元できないため、単位の解析は純粋関数の戻り値で検証する。
        assert_eq!(parse_level_base_milliseconds("5"), Some(300_000));
        assert_eq!(parse_level_base_milliseconds("0:30"), Some(30_000));
        assert_eq!(parse_level_base_milliseconds("5:30"), Some(330_000));
        assert_eq!(parse_seconds_milliseconds("1"), Some(1_000));
        assert_eq!(parse_seconds_milliseconds("2.5"), Some(2_500));
        assert_eq!(parse_seconds_milliseconds("3.25"), Some(3_250));
    }

    #[test]
    fn oversized_memory_is_rejected_and_the_session_continues() {
        // 監査「置換表サイズのオーバーフロー」: 極大の外部設定値は
        // CECPエラーとなり、後続の有効な設定と探索を妨げない。
        let output = session(
            &[RuleCode::R1],
            &format!("memory {}\nmemory 1\nmemory 2\nsd 1\nnew\ngo\n", usize::MAX),
        );

        assert_eq!(
            output
                .lines()
                .filter(|line| line.starts_with("Error "))
                .count(),
            1
        );
        assert_eq!(move_lines(&output).len(), 1);
    }

    #[test]
    fn setboard_takes_two_fields_ignores_extras_and_fails_atomically() {
        // [`CecpProtocol::handle_setboard`]の契約: 2欄必須・3欄目以降は無視・解析失敗はtellusererror
        // Illegal positionで状態不変。
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
        // [`CecpProtocol::handle_option`]の契約: option NAME=VALUEで検証し、正当ならpendingのみ更新、不正なら
        // Error (invalid option value): <受信行>。commitはnew。
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
                    "option RuleSet=L0,P0,R2,E2\n",
                    "{}", // 変更は対局中に反映されない（3回目の出現も合法のまま）
                    "option RuleSet=XX9\n",
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
                "Error (invalid option value): option RuleSet=L1,E1\n",
                "Illegal move (repetition): i8i9\n",
            )
        );
    }

    // モジュールdocのコマンド表: 無視、未対応、未知コマンド、quitの応答。
    #[test]
    fn commands_return_the_documented_response() {
        for (input, expected) in [
            (
                "ping a\neasy\nhard\npost\nnopost\nrandom\ncomputer\nname foo\nhint\ndraw\nping b\n",
                "pong a\npong b\n",
            ),
            (
                "new\nforce\nundo\nremove\nanalyze\nusermove g4g5\n",
                "Error (command not supported): undo\nError (command not supported): remove\nError (command not supported): analyze\n",
            ),
            (
                "foobar baz qux\nnew\nforce\nusermove g4g5\n",
                "Error (unknown command): foobar\n",
            ),
            ("xboard\nquit\nping 1\n", ""),
        ] {
            assert_eq!(session(&[RuleCode::R1], input), expected, "input={input:?}");
        }
    }

    #[test]
    fn result_reason_table_matches_the_documented_mapping() {
        // [`write_result`]と[`draw_reason_text`]の固定語に従い、
        // 裸玉の引き分けはbare kingsと表す。
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
    }
}
