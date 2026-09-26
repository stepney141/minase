//! 対局時計とプロトコル別の思考要求。

use crate::Color;
use crate::harness::engine::{CECP_FIXED_TIME_CS, ThinkRequest};
use crate::harness::failure::EngineFailure;
use crate::harness::limit::{SearchLimit, TimeControl};
use std::time::Duration;

/// 1エンジンの現在の時計。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Clock {
    /// 残り時間。
    pub(super) remaining: Duration,
    /// 1手ごとの加算時間。
    increment: Duration,
    /// 1手ごとの秒読み。
    byoyomi: Duration,
}

impl Clock {
    /// 時間制御から時計を初期化する。
    pub(super) fn new(time: TimeControl) -> Self {
        Self {
            remaining: Duration::from_millis(time.base_ms),
            increment: Duration::from_millis(time.increment_ms),
            byoyomi: Duration::from_millis(time.byoyomi_ms),
        }
    }

    /// 実測思考時間を反映し、時間切れかどうかを返す。
    pub fn update(&mut self, elapsed: Duration) -> Result<(), EngineFailure> {
        if elapsed > self.remaining + self.byoyomi {
            return Err(EngineFailure::TimeForfeit);
        }
        self.remaining = self.remaining.saturating_sub(elapsed) + self.increment;
        Ok(())
    }

    /// USIへ送るミリ秒単位の残り時間を返す。
    fn remaining_ms(self) -> u128 {
        self.remaining.as_millis()
    }

    /// USIへ送るミリ秒単位の加算時間を返す。
    fn increment_ms(self) -> u128 {
        self.increment.as_millis()
    }

    /// USIへ送るミリ秒単位の秒読みを返す。
    fn byoyomi_ms(self) -> u128 {
        self.byoyomi.as_millis()
    }

    /// CECPへ送るセンチ秒単位の残り時間を返す。10 ms未満は切り捨てる。
    fn remaining_cs(self) -> u64 {
        u64::try_from(self.remaining.as_millis() / 10)
            .expect("a clock created from u64 milliseconds must fit in u64 centiseconds")
    }

    /// CECPへ送る残り時間と秒読みの合計をセンチ秒で返す。10 ms未満は切り捨てる。
    fn cecp_time_cs(self) -> u64 {
        Self {
            remaining: self.remaining + self.byoyomi,
            ..self
        }
        .remaining_cs()
    }
}

/// 1局で両色に割り当てた時計。
#[derive(Clone, Copy)]
pub struct GameClocks {
    /// 先手の時計。固定制限の側は`None`。
    black: Option<Clock>,
    /// 後手の時計。固定制限の側は`None`。
    white: Option<Clock>,
}

impl GameClocks {
    /// プレイヤーAとBの制限を対局時の色へ割り当てる。
    pub fn new(player_a_color: Color, player_a: SearchLimit, player_b: SearchLimit) -> Self {
        let (black, white) = if player_a_color == Color::Black {
            (player_a.clock(), player_b.clock())
        } else {
            (player_b.clock(), player_a.clock())
        };
        Self { black, white }
    }

    /// 指定色の時計を返す。
    pub(super) fn get(&self, color: Color) -> Option<Clock> {
        match color {
            Color::Black => self.black,
            Color::White => self.white,
        }
    }

    /// 指定色の時計を可変参照で返す。
    pub fn get_mut(&mut self, color: Color) -> Option<&mut Clock> {
        match color {
            Color::Black => self.black.as_mut(),
            Color::White => self.white.as_mut(),
        }
    }

    /// 現在の両時計から時間制御用のUSI `go`引数を返す。
    fn go_text(&self, side_to_move: Color) -> String {
        let black = self.black.unwrap_or_else(zero_clock);
        let white = self.white.unwrap_or_else(zero_clock);
        let byoyomi = self
            .get(side_to_move)
            .expect("a time-controlled player must have a clock");
        format!(
            "btime {} wtime {} binc {} winc {} byoyomi {}",
            black.remaining_ms(),
            white.remaining_ms(),
            black.increment_ms(),
            white.increment_ms(),
            byoyomi.byoyomi_ms()
        )
    }

    /// 現在の制限と両時計からプロトコル共通の思考要求を作る。
    pub(super) fn think_request(&self, side_to_move: Color, limit: SearchLimit) -> ThinkRequest {
        match limit {
            SearchLimit::Fixed { .. } => ThinkRequest {
                go_text: limit
                    .fixed_go_text()
                    .expect("a fixed limit must have USI go text"),
                own_cs: CECP_FIXED_TIME_CS,
                opponent_cs: CECP_FIXED_TIME_CS,
            },
            SearchLimit::Time(_) => ThinkRequest {
                go_text: self.go_text(side_to_move),
                own_cs: self
                    .get(side_to_move)
                    .expect("a time-controlled player must have a clock")
                    .cecp_time_cs(),
                opponent_cs: self
                    .get(side_to_move.opposite())
                    .map_or(0, Clock::cecp_time_cs),
            },
        }
    }
}

/// 時間制御を使わない側をUSI時間引数へ表す0値の時計を返す。
fn zero_clock() -> Clock {
    Clock {
        remaining: Duration::ZERO,
        increment: Duration::ZERO,
        byoyomi: Duration::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::limit::parse_search_limit;

    // D8-HARN-10(search.md実施状況): 実測思考時間が`残り時間 + byoyomi`を
    // 「超えた」場合だけ時間切れ失権となる。同値は超過ではない。
    #[test]
    fn time_forfeit_requires_exceeding_remaining_plus_byoyomi() {
        let mut clock = Clock::new(TimeControl {
            base_ms: 1_000,
            increment_ms: 0,
            byoyomi_ms: 50,
        });
        // ちょうど残り+秒読みの消費は失権ではない
        assert_eq!(clock.update(Duration::from_millis(1_050)), Ok(()));
        // 残りは消費に応じて減少する(現在値契約。加算時間0で会計の曖昧さを避ける)
        assert_eq!(clock.remaining_ms(), 0);
        assert_eq!(clock.update(Duration::from_millis(50)), Ok(()));
        assert_eq!(
            clock.update(Duration::from_millis(51)),
            Err(EngineFailure::TimeForfeit)
        );
    }

    // D8-HARN-10(search.md実施状況＋sprt.mdペア対局): 毎手のgoは両者の時計の
    // 現在値をbtime/wtime/binc/winc/byoyomiで送る。ペア内の先後入替で同一
    // エンジンの時計がbtime側とwtime側を交差する。
    #[test]
    fn go_time_arguments_reflect_current_clocks_and_cross_colors() {
        let player_a = SearchLimit::Time(TimeControl {
            base_ms: 10_000,
            increment_ms: 100,
            byoyomi_ms: 1_000,
        });
        let player_b = SearchLimit::Time(TimeControl {
            base_ms: 20_000,
            increment_ms: 200,
            byoyomi_ms: 2_000,
        });
        // プレイヤーAが後手の局では、Aの時計がwtime/winc側に載る
        let clocks = GameClocks::new(Color::White, player_a, player_b);
        assert_eq!(
            clocks.go_text(Color::Black),
            "btime 20000 wtime 10000 binc 200 winc 100 byoyomi 2000"
        );
        // byoyomiは手番側の値を送る
        assert_eq!(
            clocks.go_text(Color::White),
            "btime 20000 wtime 10000 binc 200 winc 100 byoyomi 1000"
        );

        // 現在値契約: 消費後のgoは消費250 msと加算100 msを残り時間へ反映する
        let simple = SearchLimit::Time(TimeControl {
            base_ms: 1_000,
            increment_ms: 100,
            byoyomi_ms: 0,
        });
        let mut clocks = GameClocks::new(Color::Black, simple, simple);
        clocks
            .get_mut(Color::Black)
            .expect("a time-controlled player must have a clock")
            .update(Duration::from_millis(250))
            .expect("a small consumption must not forfeit");
        assert_eq!(
            clocks.go_text(Color::White),
            "btime 850 wtime 1000 binc 100 winc 100 byoyomi 0"
        );

        // 固定制限側のgo引数(sprt.mdの`--each depth=4`等に対応)
        assert_eq!(
            SearchLimit::Fixed {
                depth: Some(3),
                nodes: Some(400),
            }
            .fixed_go_text(),
            Some("depth 3 nodes 400".to_owned())
        );

        // D8-HARN-20（match-harness.md「CECPセッション管理」）: CECPへは固定制限で
        // 十分大きい固定時計を、時間制御で現在値を10 ms単位へ切り捨てて送る。
        let fixed = clocks.think_request(
            Color::Black,
            SearchLimit::Fixed {
                depth: Some(1),
                nodes: None,
            },
        );
        assert_eq!(fixed.own_cs, 3_000_000);
        assert_eq!(fixed.opponent_cs, 3_000_000);
        let timed = clocks.think_request(Color::Black, simple);
        assert_eq!(timed.own_cs, 85);
        assert_eq!(timed.opponent_cs, 100);
    }

    // 1手固定時間の仕様: CECPの両時計は残り時間と秒読みの合計を送り、
    // 時計を持たない相手には0を送る。USIには秒読みを独立して渡す。
    #[test]
    fn cecp_time_requests_include_byoyomi() {
        let fixed_time = parse_search_limit("time=0+0,byoyomi=2000").unwrap();
        let opponent = parse_search_limit("time=1007+0,byoyomi=1007").unwrap();
        for color in [Color::Black, Color::White] {
            let clocks = GameClocks::new(color, fixed_time, opponent);
            let request = clocks.think_request(color, fixed_time);
            assert_eq!(request.own_cs, 200);
            // 合計2014 msを切り捨てる。個別の切り捨てでは200になってしまう。
            assert_eq!(request.opponent_cs, 201);
            let request = clocks.think_request(color.opposite(), opponent);
            assert_eq!(request.own_cs, 201);
            assert_eq!(request.opponent_cs, 200);
        }

        let clocks = GameClocks::new(
            Color::Black,
            fixed_time,
            parse_search_limit("depth=1").unwrap(),
        );
        let request = clocks.think_request(Color::Black, fixed_time);
        assert_eq!(request.own_cs, 200);
        assert_eq!(request.opponent_cs, 0);
        assert_eq!(
            request.go_text,
            "btime 0 wtime 0 binc 0 winc 0 byoyomi 2000"
        );
    }
}
