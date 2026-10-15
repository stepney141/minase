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
        let mut lines = message.lines();
        // 操作名と原因は同じ診断項目に含まれる。
        assert_eq!(
            lines.next(),
            Some(format!("position invariant failed after {operation}: {reason}").as_str()),
            "{operation}: reason"
        );
        assert_eq!(
            lines.next(),
            Some(format!("move: {mv:?}").as_str()),
            "{operation}: move"
        );
        assert_eq!(
            lines.next(),
            Some(format!("side to move: {:?}", position.side_to_move()).as_str()),
            "{operation}: side to move"
        );
        for (name, incremental, recomputed) in [
            ("zobrist", position.zobrist(), position.recompute_zobrist()),
            (
                "rights zobrist",
                position.rights_zobrist(),
                position.recompute_rights_zobrist(),
            ),
        ] {
            assert_eq!(
                lines.next(),
                Some(
                    format!(
                        "{name}: incremental={incremental:#018x}, recomputed={recomputed:#018x}"
                    )
                    .as_str()
                ),
                "{operation}: {name}"
            );
        }
        assert_eq!(
            lines.next(),
            Some("board raw codes (rank 0..11, file 0..11):"),
            "{operation}: board header"
        );
        for rank in 0..12 {
            let row: Vec<_> = (0..12)
                .map(|file| position.board[rank * 16 + file])
                .collect();
            assert_eq!(
                lines.next(),
                Some(format!("{row:?}").as_str()),
                "{operation}: rank={rank}"
            );
        }
        // 不正局面からは拡張SFENを構築できず、追加の診断行はない。
        assert_eq!(
            lines.next(),
            None,
            "{operation}: unexpected diagnostic item"
        );
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
