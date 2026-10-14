#![allow(dead_code)]
use super::{Outcome, Sample};
use minase::training::records::{
    Entry, Exclusion, GameRecord, Header, Origin, Outcome as GameOutcome, Policy, Value, Writer,
};
use minase_core::{DrawReason, GameResult, WinReason};
use std::io::Cursor;

pub fn outcome(sample: &Sample) -> GameOutcome {
    if sample.origin() == Origin::Synthetic {
        return GameOutcome::NotFromGame;
    }
    GameOutcome::Game(match sample.outcome() {
        Outcome::Win => GameResult::Win {
            winner: sample.side_to_move(),
            reason: WinReason::Resignation,
        },
        Outcome::Loss => GameResult::Win {
            winner: sample.side_to_move().opposite(),
            reason: WinReason::Resignation,
        },
        Outcome::Draw => GameResult::Draw {
            reason: DrawReason::Agreement,
        },
    })
}

pub fn bytes(header: Header, games: &[GameRecord]) -> Vec<u8> {
    let mut writer = Writer::new(Cursor::new(Vec::new()), header).unwrap();
    for game in games {
        writer.write_game(game).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// 独立した固定局面を1局面1対局で書く。番号は昇順にし、要求された訓練・検証区分を保つ。
/// 同一対局内の関係を検査する入力には使わず、GameRecordを直接構築する。
pub fn independent(header: Header, samples: &[Sample]) -> Vec<u8> {
    let mut next = 0_u32;
    let mut games = Vec::new();
    for sample in samples {
        let validation = is_validation(header.seed(), sample.game_number());
        let mut number = next.max(sample.game_number());
        while is_validation(header.seed(), number) != validation {
            number += 1;
        }
        next = number.checked_add(1).unwrap();
        games.push(
            GameRecord::new(
                number,
                sample.origin(),
                outcome(sample),
                sample.ply(),
                sample.to_position().unwrap(),
                vec![
                    Entry::new(
                        None,
                        if sample.origin() == Origin::Synthetic {
                            Value::None
                        } else {
                            Value::AlphaBeta(sample.score())
                        },
                        Exclusion::None,
                        Policy::None,
                        None,
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        );
    }
    bytes(header, &games)
}

// 検証分割の公開仕様であるSplitMix64(seed, game) % 20 == 0。
fn is_validation(seed: u64, game: u32) -> bool {
    let mut z = seed ^ u64::from(game).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (z ^ (z >> 31)).is_multiple_of(20)
}
