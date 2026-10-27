//! 駆動部と探索スレッドの試験に使う、学習済み重みを持たない局面と評価器。

use minase_core::test_util::{position, sq};
use minase_core::{
    Color, ExhaustionRule, Game, Move, MoveGenerator, PieceKind, Position, RepetitionRule, Rules,
};

use super::{Driver, EvaluationRequest};
use crate::nn::{Evaluator, NnError, Query};
use crate::search::{SearchLimits, SearchSnapshot};

pub(super) const SCALE: f32 = 1072.65;
pub(super) const CAPACITY: usize = 1024 * 1024;

pub(super) fn rules(repetition: RepetitionRule) -> Rules {
    Rules {
        repetition,
        exhaustion: ExhaustionRule::E2,
        ..Rules::ENGINE_DEFAULT
    }
}

pub(super) fn step(from: minase_core::Square, to: minase_core::Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: false,
    }
}

pub(super) fn legal_moves(position: &Position) -> Vec<Move> {
    let mut moves = Vec::new();
    MoveGenerator::new(Rules::ENGINE_DEFAULT.moves).generate_moves(position, &mut moves);
    moves
}

pub(super) fn snapshot(position: &Position) -> SearchSnapshot {
    SearchSnapshot::from_game(&Game::from_position(
        rules(RepetitionRule::R1),
        position.clone(),
    ))
    .unwrap()
}

pub(super) fn initial() -> SearchSnapshot {
    snapshot(&Position::initial())
}

pub(super) fn ambiguous_lion_game() -> Game {
    let codes = minase_core::rules::parse_rule_set("L0,P0,R1,E2").unwrap();
    let rules = Rules::from_codes(&codes).unwrap();
    let position =
        minase_core::parse_sfen("11k/12/2n9/12/12/12/8p3/4N3+O3/12/12/12/K11 b").unwrap();
    Game::from_position(rules, position)
}

pub(super) fn nodes(count: u64) -> SearchLimits {
    SearchLimits::new(None, Some(count), None, None).unwrap()
}

/// 4手で元へ戻る飛車の往復。各攻撃側は相手の歩兵を2升から照準する。
pub(super) fn rook_cycle(black_attacks: bool, white_attacks: bool) -> (Position, [Move; 4]) {
    let mut pieces = vec![
        (sq(0, 0), Color::Black, PieceKind::King),
        (sq(3, 3), Color::Black, PieceKind::Rook),
        (sq(8, 8), Color::White, PieceKind::Rook),
        (sq(11, 11), Color::White, PieceKind::King),
    ];
    if black_attacks {
        pieces.push((sq(3, 9), Color::White, PieceKind::Pawn));
    }
    if white_attacks {
        pieces.push((sq(8, 1), Color::Black, PieceKind::Pawn));
    }
    (
        position(Color::Black, &pieces),
        [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ],
    )
}

/// 指定した手順の手だけに高いlogitを与え、問い合わせた局面を記録する。
#[derive(Default)]
pub(super) struct ScriptedEvaluator {
    pub(super) script: Vec<(Position, Move)>,
    pub(super) seen: Vec<Position>,
}

impl ScriptedEvaluator {
    pub(super) fn line(position: &Position, moves: &[Move]) -> Self {
        let mut script = Vec::new();
        let mut position = position.clone();
        for &mv in moves {
            assert!(legal_moves(&position).contains(&mv));
            script.push((position.clone(), mv));
            position.make_move_unchecked(mv, Rules::ENGINE_DEFAULT.moves);
        }
        Self {
            script,
            seen: Vec::new(),
        }
    }
}

impl Evaluator for ScriptedEvaluator {
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        logits.clear();
        values.clear();
        for query in queries {
            self.seen.push(query.position.clone());
            let selected = self
                .script
                .iter()
                .find(|(position, _)| position == query.position)
                .map(|(_, mv)| *mv);
            for &mv in query.moves {
                logits.push(if Some(mv) == selected { 0.0 } else { -1000.0 });
            }
            values.push(0.5);
        }
        Ok(())
    }
}

pub(super) fn evaluate(
    driver: &mut Driver,
    evaluator: &mut dyn Evaluator,
    request: &EvaluationRequest,
) {
    let mut logits = Vec::new();
    let mut values = Vec::new();
    evaluator
        .evaluate(&[request.query()], &mut logits, &mut values)
        .unwrap();
    driver.evaluate(request, &logits, values[0]).unwrap();
}

pub(super) fn path(driver: &Driver, moves: &[Move]) -> Vec<usize> {
    let mut node = driver.tree().root();
    let mut path = Vec::new();
    for mv in moves {
        let index = node.edges.iter().position(|edge| edge.mv == *mv).unwrap();
        path.push(index);
        node = node.edges[index].child.as_deref().unwrap();
    }
    path
}
