//! 移動の幾何と不足在庫から候補を構成し、順方向再適用で絞り込む。

use std::collections::HashSet;

use super::{material, membership};
use crate::core::attacks::{movement_profile, movement_profile_data};
use crate::core::bitboard::Bitboard;
use crate::core::movegen::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::{PieceCode, PieceKind};
use crate::core::position::{Position, PositionBuilder};
use crate::core::rules::{MoveRules, PromotionRule, in_promotion_zone};
use crate::core::square::Square;

/// 集合Aに属する対象局面へ到達する、通常移動の直前局面を返す。
pub(super) fn generate(forward: &MoveGenerator, target: &Position) -> Vec<Position> {
    let mover = target.side_to_move().opposite();
    let mut missing = material::missing(target);
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for to in target.pieces_of(mover).iter() {
        let arrived = target.piece_at(to).expect("mover square has a piece");
        let mut before_choices = vec![(arrived, false)];
        if arrived.is_promoted() {
            let origin = arrived
                .kind()
                .expect("board piece has a kind")
                .unpromoted()
                .expect("promoted piece has an origin");
            before_choices.push((
                PieceCode::new(mover, origin).expect("origin is unpromoted"),
                true,
            ));
        }
        for (before, promote) in before_choices {
            for from in origins(forward, to, before).iter() {
                if target.piece_at(from).is_some() {
                    continue;
                }
                let mv = Move {
                    from,
                    mid: None,
                    to,
                    promote,
                };
                let mut visit = |captured| {
                    for candidate in candidates(forward.rules(), target, mv, before, captured) {
                        if membership(forward.rules(), &candidate).is_err() {
                            continue;
                        }
                        let mut replayed = candidate.clone();
                        if replayed.try_make_move(mv, forward).is_ok()
                            && replayed == *target
                            && seen.insert(candidate.clone())
                        {
                            result.push(candidate);
                        }
                    }
                };
                visit(None);
                material::for_each_restoration(
                    &mut missing,
                    target.side_to_move(),
                    |piece, _remaining| {
                        visit(Some(piece));
                    },
                );
            }
        }
    }
    result
}

/// 固定移動・固定跳び・走りの幾何を反転させた移動元集合を返す。
fn origins(forward: &MoveGenerator, to: Square, before: PieceCode) -> Bitboard {
    let mover = before.color().expect("moving piece has an owner");
    let profile = movement_profile_data(movement_profile(
        before.kind().expect("moving piece has a kind"),
    ));
    let mut origins = Bitboard::EMPTY;
    for delta in profile.fixed_deltas {
        let (df, dr) = delta.for_color(mover);
        if let Some(from) = to.offset(-df, -dr) {
            origins.set(from);
        }
    }
    for slide in profile.slides {
        // 捕獲駒を復元した盤面での遮蔽判定を順方向生成器へ委ねる。
        origins |= forward.tables().sliding_control(
            to,
            slide.direction.for_color(mover).opposite(),
            Bitboard::EMPTY,
        );
    }
    // フェーズ4で実装する
    origins
}

/// 盤面を復元し、移動駒と捕獲駒の成り権保留状態を独立に列挙する。
fn candidates(
    rules: MoveRules,
    target: &Position,
    mv: Move,
    before: PieceCode,
    captured: Option<PieceCode>,
) -> Vec<Position> {
    let mut inherited = target.promotion_deferred();
    inherited.clear(mv.from);
    inherited.clear(mv.to);
    let mut optional = Vec::new();
    for (square, piece) in [(mv.from, Some(before)), (mv.to, captured)] {
        if let Some(piece) = piece
            && can_defer(rules, square, piece)
        {
            optional.push(square);
        }
    }
    // P2の着手側で非移動駒の待機も消失するため、全体列挙はフェーズ4で実装する。
    let mut result = Vec::new();
    for bits in 0..(1 << optional.len()) {
        let mut builder = PositionBuilder::new(target.side_to_move().opposite());
        for square in target.occupied().iter() {
            if square != mv.from && square != mv.to {
                builder
                    .put(square, target.piece_at(square).expect("occupied piece"))
                    .expect("unchanged squares are distinct");
            }
        }
        builder.put(mv.from, before).expect("origin was empty");
        if let Some(piece) = captured {
            builder.put(mv.to, piece).expect("destination was removed");
        }
        let mut deferred = inherited;
        for (index, square) in optional.iter().enumerate() {
            if bits & (1 << index) != 0 {
                deferred.set(*square);
            }
        }
        for square in deferred.iter() {
            builder
                .mark_promotion_deferred(square)
                .expect("deferred pieces are eligible");
        }
        let mut candidate = builder.finish().expect("reconstructed board is consistent");
        candidate
            .set_lion_capture(None)
            .expect("absent lion record is valid");
        result.push(candidate);
    }
    result
}

/// 採用規則の下で成り権保留状態を持てる配置かを返す(第30条)。
fn can_defer(rules: MoveRules, square: Square, piece: PieceCode) -> bool {
    let kind = piece.kind().expect("board piece has a kind");
    !piece.is_promoted()
        && kind.can_promote()
        && in_promotion_zone(piece.color().expect("board piece has an owner"), square)
        && (rules.promotion != PromotionRule::P0 || (rules.p5 && kind == PieceKind::Pawn))
}
