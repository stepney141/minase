//! movegen-speedup.md「静止探索の捕獲を段階的に生成する」の残存手順契約。

use super::*;
use crate::core::movegen::tests::{CAPTURE_EDGE_SFENS, capture_test_positions, capture_test_rules};

fn reference(
    position: &Position,
    generator: &MoveGenerator,
    pst: &Pst,
    tt_move: Option<Move>,
) -> Vec<(Move, MoveOrderKey)> {
    let mut moves = Vec::new();
    generator.generate_captures(position, &mut moves);
    let mut captures: Vec<_> = moves
        .into_iter()
        .map(|mv| (mv, move_order_key(position, pst, mv).unwrap()))
        .collect();
    order_captures(&mut captures);
    if let Some(index) = tt_move.and_then(|tt| captures.iter().position(|&(mv, _)| mv == tt)) {
        captures[..=index].rotate_right(1);
    }
    captures
}

// 同節と「捕獲対象を生成前に除外する」: 公開生成・安定整列・回転から
// 得た列を閾値で除いた結果に、段階生成の残存手順が一致する。
#[test]
fn staged_captures_match_reference_for_rules_tt_moves_and_thresholds() {
    let pst = crate::eval::weights().unwrap();
    let ranks = CaptureRanks::new(&pst);
    let positions = capture_test_positions();
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        let mut buffers = QsearchBuffers::default();
        for position in &positions {
            let ordered = reference(position, &generator, &pst, None);
            let mut thresholds = vec![-1, i32::MAX / 2];
            thresholds.extend(ordered.iter().map(|(_, key)| key.captured_value));
            thresholds.sort_unstable();
            thresholds.dedup();
            let mut all = Vec::new();
            generator.generate_moves(position, &mut all);
            let quiet = all.into_iter().find(|&mv| {
                position
                    .captured_squares(mv)
                    .into_iter()
                    .all(|s| s.is_none())
            });
            for threshold in thresholds {
                let tt_moves = [
                    None,
                    ordered.first().map(|&(mv, _)| mv),
                    ordered.get(ordered.len() / 2).map(|&(mv, _)| mv),
                    quiet,
                    ordered
                        .iter()
                        .find(|&&(mv, key)| {
                            key.captured_value <= threshold && !captures_last_royal(position, mv)
                        })
                        .map(|&(mv, _)| mv),
                ];
                for tt_move in tt_moves {
                    let expected: Vec<_> = reference(position, &generator, &pst, tt_move)
                        .into_iter()
                        .filter(|&(mv, key)| {
                            key.captured_value > threshold || captures_last_royal(position, mv)
                        })
                        .map(|(mv, _)| mv)
                        .collect();
                    buffers.reset(position, &generator, tt_move);
                    let mut actual = Vec::new();
                    while let Some(candidate) =
                        buffers.next(position, &generator, &pst, &ranks, threshold)
                    {
                        if candidate.captured_value > threshold
                            || captures_last_royal(position, candidate.capture.mv)
                        {
                            actual.push(candidate.capture.mv);
                        }
                    }
                    assert_eq!(
                        actual, expected,
                        "rules={rules:?}, tt={tt_move:?}, threshold={threshold}"
                    );
                }
            }
        }
    }
}

// 同節: 最初のグループ内で探索を止めれば、低価値の通常捕獲は未生成である。
#[test]
fn stopping_in_first_group_leaves_later_groups_ungenerated() {
    let pst = crate::eval::weights().unwrap();
    let ranks = CaptureRanks::new(&pst);
    let generator = MoveGenerator::standard();
    let position = crate::parse_sfen("12/2p1q7/2R1R7/12/12/12/12/12/12/12/12/11K b").unwrap();
    let mut buffers = QsearchBuffers::default();
    buffers.reset(&position, &generator, None);
    assert!(
        buffers
            .next(&position, &generator, &pst, &ranks, -1)
            .is_some()
    );
    let ordered = reference(&position, &generator, &pst, None);
    let first_value = ordered[0].1.captured_value;
    assert_ne!(buffers.present_ranks, 0);
    for rank in 0..ranks.values.len() {
        if buffers.present_ranks & (1_u64 << rank) != 0 {
            assert!(ranks.values[rank] < first_value);
        }
    }
    assert!(
        buffers
            .group
            .iter()
            .all(|c| c.captured_value == first_value)
    );
}

// 同節: 特殊捕獲は単独の被捕獲駒の価値で除外せず、2枚の合計で判定する。
#[test]
fn double_capture_survives_when_each_victim_is_at_the_threshold() {
    let pst = crate::eval::weights().unwrap();
    let ranks = CaptureRanks::new(&pst);
    let generator = MoveGenerator::standard();
    let position = crate::parse_sfen(CAPTURE_EDGE_SFENS[2]).unwrap();
    let threshold = pst.pawn_value();
    let mut buffers = QsearchBuffers::default();
    buffers.reset(&position, &generator, None);
    let mut surviving = Vec::new();
    while let Some(c) = buffers.next(&position, &generator, &pst, &ranks, threshold) {
        if c.captured_value > threshold {
            surviving.push(c);
        }
    }
    assert!(!surviving.is_empty());
    assert!(
        surviving
            .iter()
            .all(|c| c.capture.captured.into_iter().flatten().count() == 2)
    );
}

// 同節: αが検査の途中で増加しても、入口以降の残存手順は参照と一致する。
#[test]
fn rising_threshold_preserves_the_surviving_sequence() {
    let pst = crate::eval::weights().unwrap();
    let ranks = CaptureRanks::new(&pst);
    let generator = MoveGenerator::standard();
    let mut buffers = QsearchBuffers::default();
    for position in capture_test_positions() {
        let ordered = reference(&position, &generator, &pst, None);
        let tt_move = ordered.get(ordered.len() / 2).map(|&(mv, _)| mv);
        let mut threshold = -1;
        let mut expected = Vec::new();
        for (mv, key) in reference(&position, &generator, &pst, tt_move) {
            if key.captured_value > threshold || captures_last_royal(&position, mv) {
                expected.push(mv);
                threshold = threshold.max(key.captured_value);
            }
        }
        buffers.reset(&position, &generator, tt_move);
        threshold = -1;
        let mut actual = Vec::new();
        while let Some(c) = buffers.next(&position, &generator, &pst, &ranks, -1) {
            if c.captured_value > threshold || captures_last_royal(&position, c.capture.mv) {
                actual.push(c.capture.mv);
                threshold = threshold.max(c.captured_value);
            }
        }
        assert_eq!(actual, expected);
    }
}

// 同節: 置換表の手で打ち切れば、価値グループも特殊捕獲も未生成である。
#[test]
fn tt_capture_is_returned_before_generating_groups() {
    let pst = crate::eval::weights().unwrap();
    let ranks = CaptureRanks::new(&pst);
    let generator = MoveGenerator::standard();
    let position = crate::parse_sfen(CAPTURE_EDGE_SFENS[2]).unwrap();
    let tt_move = reference(&position, &generator, &pst, None)[0].0;
    let mut buffers = QsearchBuffers::default();
    buffers.reset(&position, &generator, Some(tt_move));
    assert_eq!(
        buffers
            .next(&position, &generator, &pst, &ranks, -1)
            .unwrap()
            .capture
            .mv,
        tt_move
    );
    assert!(!buffers.initialized);
    assert_eq!(buffers.present_ranks, 0);
    assert!(buffers.special.is_empty());
}

// 「捕獲対象を生成前に除外する」: 同じ標本を再走査してもバッファを再確保しない。
#[test]
fn warmed_buffers_retain_capacity_across_nodes() {
    let pst = crate::eval::weights().unwrap();
    let ranks = CaptureRanks::new(&pst);
    let generator = MoveGenerator::standard();
    let positions = capture_test_positions();
    let mut buffers = QsearchBuffers::default();
    let capacities = |b: &QsearchBuffers| {
        [
            b.validation.capacity(),
            b.capturers.capacity(),
            b.special.capacity(),
            b.group.capacity(),
        ]
    };
    for position in &positions {
        buffers.reset(position, &generator, None);
        while buffers
            .next(position, &generator, &pst, &ranks, -1)
            .is_some()
        {}
    }
    let warmed = capacities(&buffers);
    for position in &positions {
        buffers.reset(position, &generator, None);
        while buffers
            .next(position, &generator, &pst, &ranks, -1)
            .is_some()
        {}
        assert_eq!(capacities(&buffers), warmed);
    }
}

// 単位B3「駒価値の順位を探索開始時に前計算する」:
// 全47状態の値を復元でき、値の大小・等しさが順位に対応する。
#[test]
fn capture_ranks_preserve_value_order_and_equality() {
    let pst = crate::eval::weights().unwrap();
    let ranks = CaptureRanks::new(&pst);
    assert!(ranks.values.windows(2).all(|pair| pair[0] > pair[1]));
    let mut used = vec![false; ranks.values.len()];
    for state in 0..PIECE_STATE_COUNT {
        let rank = usize::from(ranks.rank_of_state[state]);
        assert_eq!(ranks.values[rank], pst.piece_value_of_state(state));
        used[rank] = true;
        for other in 0..PIECE_STATE_COUNT {
            assert_eq!(
                ranks.rank_of_state[state].cmp(&ranks.rank_of_state[other]),
                pst.piece_value_of_state(other)
                    .cmp(&pst.piece_value_of_state(state)),
            );
        }
    }
    assert!(used.into_iter().all(|present| present));
}

// 単位B3「ノードごとのグループ構築を順位のビット集合で行う」:
// 前ノードを途中で打ち切っても、同じ順位の対象升を次ノードへ持ち越さない。
#[test]
fn reset_discards_targets_from_partially_consumed_node() {
    let pst = crate::eval::weights().unwrap();
    let ranks = CaptureRanks::new(&pst);
    let generator = MoveGenerator::standard();
    let positions = [
        crate::parse_sfen("12/2p1q7/2R1R7/12/12/12/12/12/12/12/12/11K b").unwrap(),
        crate::parse_sfen("12/2q1p7/2R1R7/12/12/12/12/12/12/12/12/11K b").unwrap(),
    ];
    let mut buffers = QsearchBuffers::default();
    for first in &positions {
        for second in &positions {
            buffers.reset(first, &generator, None);
            assert!(buffers.next(first, &generator, &pst, &ranks, -1).is_some());
            assert_ne!(buffers.present_ranks, 0);
            buffers.reset(second, &generator, None);
            let mut actual = Vec::new();
            while let Some(c) = buffers.next(second, &generator, &pst, &ranks, -1) {
                actual.push(c.capture.mv);
            }
            let expected: Vec<_> = reference(second, &generator, &pst, None)
                .into_iter()
                .map(|(mv, _)| mv)
                .collect();
            assert_eq!(actual, expected);
        }
    }
}

// 設計書movegen-speedup-2.md「段階5」: 居喰いと複数の空升への移動を含む
// 圧縮候補を展開し、どの展開手が置換表にあっても公開生成の安定整列に一致する。
#[test]
fn compressed_lion_captures_preserve_every_tt_variant() {
    let pst = crate::eval::weights().unwrap();
    let ranks = CaptureRanks::new(&pst);
    let position = crate::parse_sfen(CAPTURE_EDGE_SFENS[2]).unwrap();
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        let ordered = reference(&position, &generator, &pst, None);
        let single_mid: Vec<_> = ordered
            .iter()
            .map(|&(mv, _)| mv)
            .filter(|&mv| matches!(position.captured_squares(mv), [Some(_), None]))
            .collect();
        assert!(single_mid.len() > 2);
        assert!(single_mid.iter().any(|mv| mv.from == mv.to));
        let mut buffers = QsearchBuffers::default();
        buffers.reset(&position, &generator, None);
        buffers.initialize(&position, &generator, &pst, &ranks, -1);
        assert_eq!(
            buffers
                .special
                .iter()
                .filter(|c| c.lion_destinations != 0)
                .count(),
            1
        );
        assert_eq!(
            buffers
                .special
                .iter()
                .map(|c| c.lion_destinations.count_ones() as usize)
                .sum::<usize>(),
            single_mid.len()
        );
        let invalid = Move {
            promote: true,
            ..single_mid[0]
        };
        for tt_move in ordered
            .iter()
            .map(|&(mv, _)| Some(mv))
            .chain([None, Some(invalid)])
        {
            for threshold in [-1, pst.pawn_value(), i32::MAX / 2] {
                buffers.reset(&position, &generator, tt_move);
                let mut actual = Vec::new();
                while let Some(candidate) =
                    buffers.next(&position, &generator, &pst, &ranks, threshold)
                {
                    if candidate.captured_value > threshold
                        || captures_last_royal(&position, candidate.capture.mv)
                    {
                        actual.push(candidate.capture.mv);
                    }
                }
                let expected: Vec<_> = reference(&position, &generator, &pst, tt_move)
                    .into_iter()
                    .filter(|&(mv, key)| {
                        key.captured_value > threshold || captures_last_royal(&position, mv)
                    })
                    .map(|(mv, _)| mv)
                    .collect();
                assert_eq!(
                    actual, expected,
                    "rules={rules:?}, tt={tt_move:?}, threshold={threshold}"
                );
            }
        }
    }
}

// 設計書movegen-speedup-2.md「段階5」: 駒種別の成否2通りの順位が、
// 全ての有効な駒コードについて従来の47状態の順位に一致する。
#[test]
fn kind_capture_ranks_match_all_piece_codes() {
    let pst = crate::eval::weights().unwrap();
    let ranks = CaptureRanks::new(&pst);
    for color in crate::Color::ALL {
        for kind in PieceKind::ALL {
            for piece in [
                PieceCode::new(color, kind),
                PieceCode::new_promoted(color, kind),
            ]
            .into_iter()
            .flatten()
            {
                assert_eq!(
                    ranks.ranks_of_kind[kind.index()][usize::from(piece.is_promoted())],
                    ranks.rank_of_state[piece_state_of(piece)]
                );
            }
        }
    }
}

// 設計書movegen-speedup-2.md「段階5」: 主探索のキー前計算は、同点時の
// 安定性と置換表の手の扱いを含めて、公開捕獲列の安定整列と一致する。
#[test]
fn main_picker_captures_match_stable_reference_for_all_rules() {
    let pst = crate::eval::weights().unwrap();
    let history = Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        for position in capture_test_positions() {
            let ordered = reference(&position, &generator, &pst, None);
            for tt_move in [None, ordered.get(ordered.len() / 2).map(|&(mv, _)| mv)] {
                let mut picker = MovePicker::new(tt_move, [None; KILLER_COUNT]);
                let mut actual = Vec::new();
                while let Some((mv, is_capture)) =
                    picker.next(&position, &pst, &generator, &history)
                {
                    if !is_capture {
                        break;
                    }
                    actual.push(mv);
                }
                let expected: Vec<_> = reference(&position, &generator, &pst, tt_move)
                    .into_iter()
                    .map(|(mv, _)| mv)
                    .collect();
                assert_eq!(actual, expected, "rules={rules:?}, tt={tt_move:?}");
            }
        }
    }
}
