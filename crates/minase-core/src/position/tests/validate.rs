//! 局面の不変条件の検査の試験。

use crate::position::{Position, PositionError};
use crate::rules::MoveRules;

#[test]
fn validate_rejects_corrupted_zobrist_values() {
    // Position::validateの契約に従い、両Zobrist値と再計算値の不一致を拒否する。
    let position = Position::initial();
    assert_eq!(position.validate(), Ok(()));
    let mut corrupted = position.clone();
    corrupted.zobrist ^= 1;
    assert_eq!(corrupted.validate(), Err(PositionError::ZobristMismatch));
    let mut corrupted = position;
    corrupted.rights_zobrist ^= 1;
    assert_eq!(
        corrupted.validate(),
        Err(PositionError::RightsZobristMismatch)
    );
}

#[cfg(feature = "invariants")]
mod invariants {
    use super::*;
    use crate::{Move, MoveGenerator};
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn assert_diagnostic(
        position: &Position,
        panic: Box<dyn std::any::Any + Send>,
        operation: &str,
        mv: Option<Move>,
        reason: PositionError,
    ) {
        let message = panic.downcast_ref::<String>().unwrap();
        // docコメントが列挙する項目の存在だけを検査し、行順や見出し全体は固定しない。
        for value in [
            operation.to_owned(),
            reason.to_string(),
            format!("{mv:?}"),
            format!("{:?}", position.side_to_move()),
            format!("{:#018x}", position.zobrist()),
            format!("{:#018x}", position.recompute_zobrist()),
            format!("{:#018x}", position.rights_zobrist()),
            format!("{:#018x}", position.recompute_rights_zobrist()),
        ] {
            assert!(
                message.contains(&value),
                "{operation}: missing diagnostic value {value}"
            );
        }
        // 駒コードは公開アクセサから取り出し、番兵込み盤配列の配置には依存しない。
        for square in position.occupied() {
            let code = position.piece_at(square).unwrap();
            assert!(
                message.contains(&format!("{code:?}")),
                "{operation}: missing board code {code:?}"
            );
        }
    }

    /// [`Position::assert_invariants`]の契約に従い、着手と取消しの各操作がキー破損を診断する。
    #[test]
    fn move_and_unmake_invariants_report_corrupted_keys() {
        let generator = MoveGenerator::standard();
        let mut moves = Vec::new();
        generator.generate_moves(&Position::initial(), &mut moves);
        let mv = moves[0];
        type CorruptingOperation = Box<dyn Fn(&mut Position)>;
        let cases: [(&str, CorruptingOperation, Option<Move>, PositionError); 6] = [
            (
                "make move",
                Box::new(move |position| {
                    position.zobrist ^= 1;
                    position.make_move_unchecked(mv, MoveRules::standard());
                }),
                Some(mv),
                PositionError::ZobristMismatch,
            ),
            (
                "make move",
                Box::new(move |position| {
                    position.rights_zobrist ^= 1;
                    position.make_move_unchecked(mv, MoveRules::standard());
                }),
                Some(mv),
                PositionError::RightsZobristMismatch,
            ),
            (
                "make null move",
                Box::new(|position| {
                    position.zobrist ^= 1;
                    position.make_null_move();
                }),
                None,
                PositionError::ZobristMismatch,
            ),
            (
                "make null move",
                Box::new(|position| {
                    position.rights_zobrist ^= 1;
                    position.make_null_move();
                }),
                None,
                PositionError::RightsZobristMismatch,
            ),
            (
                "unmake move",
                Box::new(move |position| {
                    let mut undo = position.make_move_unchecked(mv, MoveRules::standard());
                    undo.previous_zobrist ^= 1;
                    position.unmake_move(undo);
                }),
                Some(mv),
                PositionError::ZobristMismatch,
            ),
            (
                "unmake null move",
                Box::new(|position| {
                    let undo = position.make_null_move();
                    position.zobrist ^= 1;
                    position.unmake_null_move(undo);
                }),
                None,
                PositionError::ZobristMismatch,
            ),
        ];
        for (operation, apply, mv, reason) in cases {
            let mut position = Position::initial();
            let panic = catch_unwind(AssertUnwindSafe(|| apply(&mut position))).expect_err(
                &format!("{operation}, move={mv:?}, reason={reason:?}: corrupted key must panic"),
            );
            assert_diagnostic(&position, panic, operation, mv, reason);
        }
    }
}
