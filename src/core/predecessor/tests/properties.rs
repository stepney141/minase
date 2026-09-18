use super::*;
use crate::test_util::{bench_positions, sampled_random_positions};

#[test]
fn ordinary_forward_edges_have_predecessors() {
    // 設計書「完全性の根拠」第2層: 通常移動の全合法辺に元の局面が含まれる。
    let rules = MoveRules::standard();
    let forward = MoveGenerator::new(rules);
    let reverse = PredecessorGenerator::new(rules);
    for (index, predecessor) in std::iter::once(Position::initial())
        .chain(bench_positions())
        .chain(sampled_random_positions(rules).into_iter().take(2))
        .enumerate()
    {
        let mut moves = Vec::new();
        forward.generate_moves(&predecessor, &mut moves);
        for mv in moves {
            let kind = predecessor.piece_at(mv.from).unwrap().kind().unwrap();
            if mv.mid.is_some()
                || matches!(
                    kind,
                    PieceKind::Lion | PieceKind::HornedFalcon | PieceKind::SoaringEagle
                )
            {
                continue;
            }
            let mut target = predecessor.clone();
            target.try_make_move(mv, &forward).unwrap();
            let result = reverse.generate_predecessors(&target).unwrap();
            assert!(result.contains(&predecessor), "corpus {index}, move {mv:?}");
        }
    }
}
