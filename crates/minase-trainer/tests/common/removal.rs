//! 書き出したネットを直接評価する、除去符号報告の独立した参照計算。
use minase::eval::{nnue::Network, piece_values::piece_value};
use minase_core::{Color, PieceKind, Position, PositionBuilder, Square};
use serde_json::{Value, json};

pub fn assert_report(report: &Value, network: &Network, samples: &[(Position, Color)]) {
    let mut total = 0;
    let mut terminal = 0;
    let mut reversed = Vec::new();
    for (position, side) in samples {
        let material: i32 = Square::all()
            .filter_map(|s| position.piece_at(s))
            .map(|p| piece_value(p) * if p.color() == Some(*side) { 1 } else { -1 })
            .sum();
        let before = network.evaluate_for(position, *side).unwrap();
        for square in Square::all() {
            let Some(piece) = position.piece_at(square) else {
                continue;
            };
            if piece.kind() == Some(PieceKind::King)
                || (matches!(network, Network::LayerStack(_))
                    && piece.kind() == Some(PieceKind::CrownPrince))
            {
                continue;
            }
            let mut builder = PositionBuilder::new(position.side_to_move());
            for s in Square::all().filter(|&s| s != square) {
                if let Some(p) = position.piece_at(s) {
                    builder.put(s, p).unwrap();
                }
            }
            let mut after = builder.finish().unwrap();
            after
                .set_lion_capture(position.lion_capture_square())
                .unwrap();
            if matches!(network, Network::Simple(_))
                && Color::ALL.into_iter().any(|color| {
                    !Square::all().filter_map(|s| after.piece_at(s)).any(|p| {
                        p.color() == Some(color)
                            && matches!(p.kind(), Some(PieceKind::King | PieceKind::CrownPrince))
                    })
                })
            {
                terminal += 1;
                continue;
            }
            let removed = piece_value(piece) * if piece.color() == Some(*side) { 1 } else { -1 };
            let reference =
                (material - removed).clamp(-28_999, 28_999) - material.clamp(-28_999, 28_999);
            if reference == 0 {
                continue;
            }
            total += 1;
            let delta = network.evaluate_for(&after, *side).unwrap() - before;
            if (delta < 0 && reference > 0) || (delta > 0 && reference < 0) {
                reversed.push(f64::from(delta).abs());
            }
        }
    }
    let expected = json!({
        "positions": samples.len(), "removals": total, "terminal_removal_count": terminal,
        "reversals": {
            "total": total, "count": reversed.len(),
            "fraction": (total != 0).then(|| reversed.len() as f64 / total as f64),
            "mean_absolute_cp": (!reversed.is_empty()).then(|| reversed.iter().sum::<f64>() / reversed.len() as f64),
            "max_absolute_cp": reversed.into_iter().reduce(f64::max),
        }
    });
    assert_eq!(report, &expected);
}
