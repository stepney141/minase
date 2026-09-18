use super::*;
use crate::test_util::{bench_positions, sampled_random_positions};

#[test]
fn ordinary_forward_edges_have_predecessors() {
    // 設計書「完全性の根拠」第2層: 通常移動の全合法辺に元の局面が含まれる。
    let rules = MoveRules::standard();
    let forward = MoveGenerator::new(rules);
    let reverse = PredecessorGenerator::new(rules);
    // 全移動形式の逆生成では2枚捕獲と記録升も毎回列挙するため、通常テストは先頭3局面に絞る。
    // 初期局面と固定シードの2局面を残し、全コーパスの網羅は設計書のフェーズ5で扱う。
    for (index, predecessor) in std::iter::once(Position::initial())
        .chain(bench_positions().into_iter().take(3))
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

/// 固定コーパスの獅子力の全合法辺が逆包含されることを検査する。
#[test]
fn lion_power_forward_edges_have_predecessors() {
    // 設計書「完全性の根拠」第2層: 経由升を持つ手も除外しない。
    // 捕獲後は復元候補が増えるため通常テストは先頭3局面に限定し、全形式は固定テストで補う。
    let rules = MoveRules::standard();
    let forward = MoveGenerator::new(rules);
    let reverse = PredecessorGenerator::new(rules);
    let mut edges = 0;
    let mut staged = 0;
    for (index, p) in bench_positions().into_iter().take(3).enumerate() {
        let mut moves = Vec::new();
        forward.generate_moves(&p, &mut moves);
        for mv in moves {
            if !matches!(
                p.piece_at(mv.from).unwrap().kind().unwrap(),
                PieceKind::Lion | PieceKind::HornedFalcon | PieceKind::SoaringEagle
            ) {
                continue;
            }
            edges += 1;
            staged += usize::from(mv.mid.is_some());
            let mut q = p.clone();
            q.try_make_move(mv, &forward).unwrap();
            let result = reverse.generate_predecessors(&q).unwrap();
            assert!(result.contains(&p), "corpus {index}, move {mv:?}");
        }
    }
    assert!(edges > 0);
    assert!(staged > 0);
}
