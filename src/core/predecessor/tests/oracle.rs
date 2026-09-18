//! 設計書「完全性の根拠」第3層。順方向辺だけから直前局面表を作る。

use super::*;
use crate::{RuleCode::*, Rules};
use std::collections::HashMap;

/// この有限集合に対する集合Aの独立判定。
fn admissible(rules: MoveRules, p: &Position) -> bool {
    if p.validate().is_err() {
        return false;
    }
    // 第4・5・9条: 王各1枚、先手歩兵由来高々1枚、後手獅子高々1枚。
    // 構成時点でこの枚数に限るため在庫超過はなく、両側の麒麟在庫は必ず欠ける。
    // 第30条: 保留候補は敵陣内の不成歩兵1枚だけ。P2の高々1個も満たす。
    if rules.promotion == PromotionRule::P0 && !rules.p5 && !p.promotion_deferred().is_empty() {
        return false;
    }
    // 第15条と設計書「直前局面の定義」: 角鷹・飛鷲がないので空升記録は不可。
    // 麒麟由来の成獅子もないので、記録升は直前着手側の非獅子に限る。
    match p.lion_capture_square() {
        None => true,
        Some(square) => p.piece_at(square).is_some_and(|pc| {
            pc.color() == Some(p.side_to_move().opposite()) && pc.kind() != Some(PieceKind::Lion)
        }),
    }
}

fn states(rules: MoveRules) -> HashSet<Position> {
    // 6升の標準規則オラクルが30秒を超えたため、敵陣境界を含む4升へ縮小する。
    let squares: Vec<_> = (0..=1)
        .flat_map(|file| (7..=8).map(move |rank| sq(file, rank)))
        .collect();
    let mut result = HashSet::new();
    let pawn_choices: Vec<_> = std::iter::once(None)
        .chain(squares.iter().flat_map(|&s| {
            [
                Some((s, piece(Color::Black, PieceKind::Pawn))),
                Some((
                    s,
                    PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap(),
                )),
            ]
        }))
        .collect();
    for &black in &squares {
        for &white in &squares {
            for &pawn in &pawn_choices {
                for lion in std::iter::once(None).chain(squares.iter().copied().map(Some)) {
                    let mut pieces = vec![
                        (black, piece(Color::Black, PieceKind::King)),
                        (white, piece(Color::White, PieceKind::King)),
                    ];
                    pieces.extend(pawn);
                    if let Some(s) = lion {
                        pieces.push((s, piece(Color::White, PieceKind::Lion)));
                    }
                    if pieces.iter().map(|&(s, _)| s).collect::<HashSet<_>>().len() != pieces.len()
                    {
                        continue;
                    }
                    let eligible = pawn.filter(|&(s, pc)| s.rank() >= 8 && !pc.is_promoted());
                    for side in Color::ALL {
                        for deferred in [false, true] {
                            if deferred && eligible.is_none() {
                                continue;
                            }
                            let mut builder = PositionBuilder::new(side);
                            for &(s, pc) in &pieces {
                                builder.put(s, pc).unwrap();
                            }
                            if deferred {
                                builder
                                    .mark_promotion_deferred(eligible.unwrap().0)
                                    .unwrap();
                            }
                            let base = builder.finish().unwrap();
                            for record in
                                std::iter::once(None).chain(squares.iter().copied().map(Some))
                            {
                                let mut p = base.clone();
                                if p.set_lion_capture(record).is_ok() && admissible(rules, &p) {
                                    result.insert(p);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    result
}

fn compare_oracle(rules: MoveRules) {
    let states = states(rules);
    let forward = MoveGenerator::new(rules);
    let reverse = PredecessorGenerator::new(rules);
    let mut table: HashMap<Position, HashSet<Position>> = HashMap::new();
    let mut moves = Vec::new();
    let mut edges = 0;
    let mut staged = 0;
    let mut promotions = 0;
    for p in &states {
        moves.clear();
        forward.generate_moves(p, &mut moves);
        for &mv in &moves {
            let mut q = p.clone();
            q.try_make_move(mv, &forward).unwrap();
            if states.contains(&q) {
                table.entry(q).or_default().insert(p.clone());
                edges += 1;
                staged += usize::from(mv.mid.is_some());
                promotions += usize::from(mv.promote);
            }
        }
    }
    assert!(edges > 0 && staged > 0 && promotions > 0);
    let unique_edges: usize = table.values().map(HashSet::len).sum();
    println!(
        "predecessor_oracle {rules:?}: |S|={}, edges={edges}, unique_edges={unique_edges}, staged={staged}, promotions={promotions}",
        states.len()
    );
    let empty = HashSet::new();
    for q in &states {
        let actual: HashSet<_> = reverse
            .generate_predecessors(q)
            .unwrap()
            .into_iter()
            .filter(|p| states.contains(p))
            .collect();
        let expected = table.get(q).unwrap_or(&empty);
        assert_eq!(
            &actual,
            expected,
            "oracle mismatch for {}",
            crate::to_sfen(q)
        );
    }
}

#[test]
#[ignore = "標準規則の有限状態全列挙(約100秒、直前局面の集合が大きい)"]
fn predecessor_oracle_standard() {
    // 設計書「完全性の根拠」第3層: 標準規則の有限集合上で完全一致する。
    compare_oracle(MoveRules::standard());
}

#[test]
#[ignore = "ローカルルールの有限状態全列挙"]
fn predecessor_oracle_p1() {
    // 設計書「完全性の根拠」「一時状態の逆生成」: P1の保留状態も全列挙する。
    compare_oracle(Rules::from_codes(&[L0, P1, R1, E0]).unwrap().moves);
}

#[test]
#[ignore = "ローカルルールの有限状態全列挙"]
fn predecessor_oracle_p2_p6() {
    // 設計書「完全性の根拠」「一時状態の逆生成」: P2の待機とP6を併用する。
    compare_oracle(Rules::from_codes(&[L0, P2, P6, R1, E0]).unwrap().moves);
}

#[test]
#[ignore = "ローカルルールの有限状態全列挙"]
fn predecessor_oracle_p0_p5() {
    // 設計書「完全性の根拠」「一時状態の逆生成」: P5の歩兵保留を全列挙する。
    compare_oracle(Rules::from_codes(&[L0, P0, P5, R1, E0]).unwrap().moves);
}

#[test]
#[ignore = "ローカルルールの有限状態全列挙"]
fn predecessor_oracle_l1() {
    // 設計書「完全性の根拠」「ローカルルール」: 足条件なしの先獅子を検査する。
    compare_oracle(Rules::from_codes(&[L1, P0, R1, E0]).unwrap().moves);
}
