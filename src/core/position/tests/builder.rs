//! 駒の配置による局面の構築の試験。

use super::positions_with_distinct_temporary_states;
use crate::core::piece::Color;
use crate::core::position::{Position, PositionBuildError, PositionBuilder, PositionError};

#[test]
fn builder_finish_preserves_consistent_incremental_zobrist_values() {
    // 設計書predecessor-generator.md「実装フェーズ > フェーズ1」
    for position in positions_with_distinct_temporary_states()
        .into_iter()
        .chain([Position::initial()])
    {
        assert_eq!(position.zobrist(), position.recompute_zobrist());
        assert_eq!(
            position.rights_zobrist(),
            position.recompute_rights_zobrist()
        );
        assert_eq!(position.validate(), Ok(()));
    }
    for side in Color::ALL {
        assert_eq!(
            PositionBuilder::new(side).finish().unwrap().validate(),
            Ok(())
        );
    }
}

#[test]
fn builder_finish_rejects_corrupted_zobrist_values() {
    // 設計書predecessor-generator.md「Positionの公開面」
    let mut builder = PositionBuilder::new(Color::Black);
    builder.position.zobrist ^= 1;
    assert_eq!(
        builder.finish(),
        Err(PositionBuildError::InvalidPosition(
            PositionError::ZobristMismatch
        ))
    );
    let mut builder = PositionBuilder::new(Color::White);
    builder.position.rights_zobrist ^= 1;
    assert_eq!(
        builder.finish(),
        Err(PositionBuildError::InvalidPosition(
            PositionError::RightsZobristMismatch
        ))
    );
}
