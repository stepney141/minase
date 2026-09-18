use super::*;

/// 後手の歩兵と先手の金将を使う在庫境界の対象局面を作る。
fn pawn_stock_target(count: u8) -> Position {
    let mut pieces: Vec<_> = (0..count)
        .map(|file| (sq(file, 1), Color::White, PieceKind::Pawn))
        .collect();
    pieces.push((sq(5, 5), Color::Black, PieceKind::GoldGeneral));
    position(Color::White, &pieces)
}

/// 金将の直前局面を、到達升に復元する駒を指定して構築する。
fn restored_capture(target: &Position, restored: PieceCode) -> Position {
    let mut builder = PositionBuilder::new(Color::Black);
    for square in target.occupied().iter() {
        if square != sq(5, 5) {
            builder
                .put(square, target.piece_at(square).unwrap())
                .unwrap();
        }
    }
    builder
        .put(
            sq(5, 4),
            PieceCode::new(Color::Black, PieceKind::GoldGeneral).unwrap(),
        )
        .unwrap();
    builder.put(sq(5, 5), restored).unwrap();
    builder.finish().unwrap()
}

#[test]
fn full_pawn_stock_prevents_restoring_pawn_origin() {
    // 設計書「駒在庫」、第4・5・9条: 歩兵12枚が残ると歩兵も成金も復元できない。
    let target = pawn_stock_target(12);
    let result = checked(MoveRules::standard(), &target);
    for restored in [
        PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
        PieceCode::new_promoted(Color::White, PieceKind::GoldGeneral).unwrap(),
    ] {
        assert!(!result.contains(&restored_capture(&target, restored)));
    }
    let native_gold = PieceCode::new(Color::White, PieceKind::GoldGeneral).unwrap();
    let predecessor = restored_capture(&target, native_gold);
    assert!(result.contains(&predecessor));
    round_trip(
        MoveRules::standard(),
        &predecessor,
        mv(sq(5, 4), sq(5, 5), false),
    );
}

#[test]
fn missing_pawn_restores_both_unpromoted_and_promoted_forms() {
    // 設計書「捕獲駒の復元」、第9条: 不足する1枚の歩兵は不成と成金の両方を取り得る。
    let target = pawn_stock_target(11);
    let result = checked(MoveRules::standard(), &target);
    for restored in [
        PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
        PieceCode::new_promoted(Color::White, PieceKind::GoldGeneral).unwrap(),
    ] {
        let predecessor = restored_capture(&target, restored);
        assert!(result.contains(&predecessor));
        round_trip(
            MoveRules::standard(),
            &predecessor,
            mv(sq(5, 4), sq(5, 5), false),
        );
    }
    assert!(!result.is_empty());
    assert!(
        result
            .iter()
            .all(|p| p.occupied().popcount() <= target.occupied().popcount() + 1)
    );
}

#[test]
fn full_initial_stock_allows_no_capture_restoration() {
    // 設計書「駒在庫」、第4・5条: 46枚ずつ残る局面で捕獲駒を復元してはならない。
    let initial = Position::initial();
    let forward = MoveGenerator::standard();
    let mut moves = Vec::new();
    forward.generate_moves(&initial, &mut moves);
    let mv = moves
        .into_iter()
        .find(|mv| initial.piece_at(mv.from).unwrap().kind() == Some(PieceKind::Pawn))
        .unwrap();
    let result = round_trip(MoveRules::standard(), &initial, mv);
    assert!(result.iter().all(|p| p.occupied().popcount() == 92));
    let mut target = initial.clone();
    target.try_make_move(mv, &forward).unwrap();
    let mut builder = PositionBuilder::new(Color::Black);
    for square in initial.occupied().iter() {
        builder
            .put(square, initial.piece_at(square).unwrap())
            .unwrap();
    }
    builder
        .put(
            mv.to,
            PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
        )
        .unwrap();
    assert!(!result.contains(&builder.finish().unwrap()));
}

/// 同一由来の2枚復元は、不成と成駒の組でも不足在庫の範囲に限る。
#[test]
fn double_capture_respects_shared_origin_stock() {
    // 設計書「捕獲駒の復元」「駒在庫」: 歩兵と成金は同じ歩兵在庫から2枚引く。
    let from = sq(5, 5);
    let mid = sq(6, 5);
    let to = sq(7, 5);
    let pawn = piece(Color::White, PieceKind::Pawn);
    let promoted = PieceCode::new_promoted(Color::White, PieceKind::GoldGeneral).unwrap();
    let base = stocked_position(
        Color::Black,
        &[
            (from, piece(Color::Black, PieceKind::Lion)),
            (sq(11, 6), piece(Color::Black, PieceKind::Kirin)),
            (mid, pawn),
            (to, promoted),
        ],
    );
    let capture = Move {
        from,
        mid: Some(mid),
        to,
        promote: false,
    };
    let result = round_trip(MoveRules::standard(), &base, capture);
    for first in [pawn, promoted] {
        for second in [pawn, promoted] {
            let pieces: Vec<_> = base
                .occupied()
                .iter()
                .map(|s| {
                    (
                        s,
                        if s == mid {
                            first
                        } else if s == to {
                            second
                        } else {
                            base.piece_at(s).unwrap()
                        },
                    )
                })
                .collect();
            assert!(result.contains(&position_from_codes(Color::Black, &pieces)));
        }
    }
    let mut target = base.clone();
    target
        .try_make_move(capture, &MoveGenerator::standard())
        .unwrap();
    let extra = sq(11, 4);
    let mut pieces: Vec<_> = target
        .occupied()
        .iter()
        .map(|s| (s, target.piece_at(s).unwrap()))
        .collect();
    pieces.push((extra, pawn));
    let target = position_from_codes(Color::White, &pieces);
    let result = checked(MoveRules::standard(), &target);
    let mut invalid_pieces: Vec<_> = base
        .occupied()
        .iter()
        .map(|s| (s, base.piece_at(s).unwrap()))
        .collect();
    invalid_pieces.push((extra, pawn));
    assert!(!result.contains(&position_from_codes(Color::Black, &invalid_pieces)));
    for p in result {
        let count = p
            .occupied()
            .iter()
            .filter(|&s| {
                let pc = p.piece_at(s).unwrap();
                pc.color() == Some(Color::White) && (pc == pawn || pc == promoted)
            })
            .count();
        assert!(count <= 12);
    }
}
