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
