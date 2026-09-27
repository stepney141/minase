//! 局面の不変条件の検査の試験。

use crate::core::position::{Position, PositionError};
use crate::core::rules::MoveRules;
use crate::test_util::{bench_positions, sampled_random_positions};

#[test]
fn validate_rejects_corrupted_zobrist_values() {
    // 設計書predecessor-generator.md「Positionの公開面」
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

#[test]
fn bench_and_sampled_random_positions_validate() {
    // 設計書predecessor-generator.md「実装フェーズ > フェーズ1」
    for (index, position) in bench_positions()
        .into_iter()
        .chain(sampled_random_positions(MoveRules::standard()))
        .enumerate()
    {
        assert_eq!(position.validate(), Ok(()), "corpus position {index}");
    }
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
        reason: &str,
    ) {
        let message = panic.downcast_ref::<String>().unwrap();
        assert!(message.contains(reason), "{message}");
        assert!(message.contains(operation), "{message}");
        assert!(message.contains(&format!("move: {mv:?}")), "{message}");
        assert!(message.contains(&format!("side to move: {:?}", position.side_to_move())));
        assert!(message.contains(&format!(
            "zobrist: incremental={:#018x}, recomputed={:#018x}",
            position.zobrist(),
            position.recompute_zobrist()
        )));
        assert!(message.contains(&format!(
            "rights zobrist: incremental={:#018x}, recomputed={:#018x}",
            position.rights_zobrist(),
            position.recompute_rights_zobrist()
        )));
        let board = message
            .split_once("board raw codes (rank 0..11, file 0..11):\n")
            .unwrap()
            .1;
        let rows: Vec<_> = board.lines().collect();
        assert_eq!(rows.len(), 12);
        for (rank, row) in rows.into_iter().enumerate() {
            assert_eq!(
                row,
                format!("{:?}", &position.board[rank * 16..rank * 16 + 12])
            );
            assert_eq!(row.matches("PieceCode(").count(), 12);
        }
        // validateで拒否された局面はSFEN構築にも失敗するが、主診断は失わない。
        assert!(!message.contains("extended SFEN:"));
    }

    /// debugging-tools.md「整合検査の内容」: 通常着手後に両キーの破損を詳細診断付きで検出する。
    #[test]
    fn move_invariants_report_corrupted_keys() {
        let generator = MoveGenerator::standard();
        let mut moves = Vec::new();
        generator.generate_moves(&Position::initial(), &mut moves);
        let mv = moves[0];
        for rights in [false, true] {
            let mut position = Position::initial();
            let reason = if rights {
                position.rights_zobrist ^= 1;
                "RightsZobristMismatch"
            } else {
                position.zobrist ^= 1;
                "ZobristMismatch"
            };
            let panic = catch_unwind(AssertUnwindSafe(|| {
                position.make_move_unchecked(mv, MoveRules::standard())
            }))
            .expect_err("a corrupted position must panic after a move");
            assert_diagnostic(&position, panic, "make move", Some(mv), reason);
        }
    }

    /// debugging-tools.md「整合検査の内容」: ヌルムーブ後も両キーを検査し、ヌルムーブと明示する。
    #[test]
    fn null_move_invariants_report_corrupted_keys() {
        for rights in [false, true] {
            let mut position = Position::initial();
            let reason = if rights {
                position.rights_zobrist ^= 1;
                "RightsZobristMismatch"
            } else {
                position.zobrist ^= 1;
                "ZobristMismatch"
            };
            let panic = catch_unwind(AssertUnwindSafe(|| {
                position.make_null_move();
            }))
            .expect_err("a corrupted position must panic after a null move");
            assert_diagnostic(&position, panic, "make null move", None, reason);
        }
    }

    /// debugging-tools.md「整合検査の内容」: 通常着手の取消し後も復元したキーを検査する。
    #[test]
    fn unmake_invariants_report_corrupted_restored_key() {
        let generator = MoveGenerator::standard();
        let mut position = Position::initial();
        let mut moves = Vec::new();
        generator.generate_moves(&position, &mut moves);
        let mv = moves[0];
        let mut undo = position.make_move_unchecked(mv, MoveRules::standard());
        undo.previous_zobrist ^= 1;
        let panic = catch_unwind(AssertUnwindSafe(|| position.unmake_move(undo)))
            .expect_err("a corrupted restored key must panic");
        assert_diagnostic(&position, panic, "unmake move", Some(mv), "ZobristMismatch");
    }

    /// debugging-tools.md「整合検査の内容」: ヌルムーブの取消し後もキーの破損を検出する。
    #[test]
    fn unmake_null_invariants_report_corrupted_key() {
        let mut position = Position::initial();
        let undo = position.make_null_move();
        position.zobrist ^= 1;
        let panic = catch_unwind(AssertUnwindSafe(|| position.unmake_null_move(undo)))
            .expect_err("a corrupted key must panic after unmaking a null move");
        assert_diagnostic(
            &position,
            panic,
            "unmake null move",
            None,
            "ZobristMismatch",
        );
    }
}
