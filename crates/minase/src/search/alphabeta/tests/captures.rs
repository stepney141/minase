//! 静止探索の候補を捕獲価値の降順に群へ分け、同じ整列キーの手は生成順を保つ。

use super::*;
use crate::search::alphabeta::capture_history::CaptureHistory;
use minase_core::test_util::{CAPTURE_EDGE_SFENS, capture_test_positions, capture_test_rules};

fn public_reference(
    position: &Position,
    generator: &MoveGenerator,
    nnue: &Nnue,
    tt_move: Option<Move>,
) -> Vec<(Move, MoveOrderKey)> {
    let mut moves = Vec::new();
    generator.generate_captures(position, &mut moves);
    let mut captures: Vec<_> = moves
        .into_iter()
        .map(|mv| (mv, move_order_key(position, nnue, mv).unwrap()))
        .collect();
    order_captures(&mut captures);
    if let Some(index) = tt_move.and_then(|tt| captures.iter().position(|&(mv, _)| mv == tt)) {
        captures[..=index].rotate_right(1);
    }
    captures
}

// 静止探索だけに除外条件を適用する。
fn reference(
    position: &Position,
    generator: &MoveGenerator,
    nnue: &Nnue,
    tt_move: Option<Move>,
) -> Vec<(Move, MoveOrderKey)> {
    let mut captures = public_reference(position, generator, nnue, tt_move);
    captures.retain(|&(mv, _)| !MoveGenerator::is_excluded_lion_capture(position, mv));
    captures
}

// 同節と「捕獲対象を生成前に除外する」: 公開生成・安定整列・回転から
// 得た列を閾値で除いた結果に、段階生成の残存手順が一致する。
#[test]
fn staged_captures_match_reference_for_rules_tt_moves_and_thresholds() {
    let nnue = test_nnue();
    let ranks = CaptureRanks::new(nnue);
    let capture_history = CaptureHistory::new();
    let positions = capture_test_positions();
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        let mut buffers = QsearchBuffers::default();
        for position in &positions {
            let ordered = reference(position, &generator, nnue, None);
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
                    let expected: Vec<_> = reference(position, &generator, nnue, tt_move)
                        .into_iter()
                        .filter(|&(mv, key)| {
                            key.captured_value > threshold || captures_last_royal(position, mv)
                        })
                        .map(|(mv, _)| mv)
                        .collect();
                    buffers.reset(position);
                    buffers.initialize(position, &generator, nnue, &ranks, threshold);
                    buffers.set_tt_move(position, &generator, tt_move);
                    let mut actual = Vec::new();
                    while let Some(candidate) =
                        buffers.next(position, &generator, nnue, &ranks, &capture_history)
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

// 同節: 特殊捕獲は単独の被捕獲駒の価値で除外せず、2枚の合計で判定する。
#[test]
fn double_capture_survives_when_each_victim_is_at_the_threshold() {
    let nnue = test_nnue();
    let ranks = CaptureRanks::new(nnue);
    let capture_history = CaptureHistory::new();
    let generator = MoveGenerator::standard();
    let position = minase_core::parse_sfen(CAPTURE_EDGE_SFENS[2]).unwrap();
    let threshold = nnue.pawn_value();
    let mut buffers = QsearchBuffers::default();
    buffers.reset(&position);
    buffers.initialize(&position, &generator, nnue, &ranks, threshold);
    buffers.set_tt_move(&position, &generator, None);
    let mut surviving = Vec::new();
    while let Some(c) = buffers.next(&position, &generator, nnue, &ranks, &capture_history) {
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
    let nnue = test_nnue();
    let ranks = CaptureRanks::new(nnue);
    let capture_history = CaptureHistory::new();
    let generator = MoveGenerator::standard();
    let mut buffers = QsearchBuffers::default();
    for position in capture_test_positions() {
        let ordered = reference(&position, &generator, nnue, None);
        let tt_move = ordered.get(ordered.len() / 2).map(|&(mv, _)| mv);
        let mut threshold = -1;
        let mut expected = Vec::new();
        for (mv, key) in reference(&position, &generator, nnue, tt_move) {
            if key.captured_value > threshold || captures_last_royal(&position, mv) {
                expected.push(mv);
                threshold = threshold.max(key.captured_value);
            }
        }
        buffers.reset(&position);
        buffers.initialize(&position, &generator, nnue, &ranks, -1);
        buffers.set_tt_move(&position, &generator, tt_move);
        threshold = -1;
        let mut actual = Vec::new();
        while let Some(c) = buffers.next(&position, &generator, nnue, &ranks, &capture_history) {
            if c.captured_value > threshold || captures_last_royal(&position, c.capture.mv) {
                actual.push(c.capture.mv);
                threshold = threshold.max(c.captured_value);
            }
        }
        assert_eq!(actual, expected);
    }
}

// 単位B3「ノードごとのグループ構築を順位のビット集合で行う」:
// 前ノードを途中で打ち切っても、同じ順位の対象升を次ノードへ持ち越さない。
#[test]
fn reset_discards_targets_from_partially_consumed_node() {
    let nnue = test_nnue();
    let ranks = CaptureRanks::new(nnue);
    let capture_history = CaptureHistory::new();
    let generator = MoveGenerator::standard();
    let positions = [
        position(
            Color::Black,
            &[
                (sq(2, 10), Color::White, PieceKind::Pawn),
                (sq(4, 10), Color::White, PieceKind::FreeKing),
                (sq(2, 9), Color::Black, PieceKind::Rook),
                (sq(4, 9), Color::Black, PieceKind::Rook),
                (sq(11, 0), Color::Black, PieceKind::King),
            ],
        ),
        position(
            Color::Black,
            &[
                (sq(2, 10), Color::White, PieceKind::FreeKing),
                (sq(4, 10), Color::White, PieceKind::Pawn),
                (sq(2, 9), Color::Black, PieceKind::Rook),
                (sq(4, 9), Color::Black, PieceKind::Rook),
                (sq(11, 0), Color::Black, PieceKind::King),
            ],
        ),
    ];
    let mut buffers = QsearchBuffers::default();
    for first in &positions {
        for second in &positions {
            buffers.reset(first);
            buffers.initialize(first, &generator, nnue, &ranks, -1);
            buffers.set_tt_move(first, &generator, None);
            assert!(
                buffers
                    .next(first, &generator, nnue, &ranks, &capture_history)
                    .is_some()
            );
            buffers.reset(second);
            buffers.initialize(second, &generator, nnue, &ranks, -1);
            buffers.set_tt_move(second, &generator, None);
            let mut actual = Vec::new();
            while let Some(c) = buffers.next(second, &generator, nnue, &ranks, &capture_history) {
                actual.push(c.capture.mv);
            }
            let expected: Vec<_> = reference(second, &generator, nnue, None)
                .into_iter()
                .map(|(mv, _)| mv)
                .collect();
            assert_eq!(actual, expected);
        }
    }
}

// 除外した手が置換表にあっても復活せず、
// 居喰いを含む残存候補は参照列の順序で返る。
#[test]
fn lion_capture_filter_preserves_remaining_tt_variants() {
    let nnue = test_nnue();
    let ranks = CaptureRanks::new(nnue);
    let capture_history = CaptureHistory::new();
    let position = minase_core::parse_sfen(CAPTURE_EDGE_SFENS[2]).unwrap();
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        let ordered = reference(&position, &generator, nnue, None);
        let single_mid: Vec<_> = ordered
            .iter()
            .map(|&(mv, _)| mv)
            .filter(|&mv| matches!(position.captured_squares(mv), [Some(_), None]))
            .collect();
        assert!(!single_mid.is_empty());
        assert!(single_mid.iter().all(|mv| mv.from == mv.to));
        let mut buffers = QsearchBuffers::default();
        let mut public = Vec::new();
        generator.generate_captures(&position, &mut public);
        assert!(
            public
                .iter()
                .any(|&mv| MoveGenerator::is_excluded_lion_capture(&position, mv))
        );
        let invalid = Move {
            promote: true,
            ..single_mid[0]
        };
        for tt_move in public
            .iter()
            .copied()
            .map(Some)
            .chain([None, Some(invalid)])
        {
            for threshold in [-1, nnue.pawn_value(), i32::MAX / 2] {
                buffers.reset(&position);
                buffers.initialize(&position, &generator, nnue, &ranks, threshold);
                buffers.set_tt_move(&position, &generator, tt_move);
                let mut actual = Vec::new();
                while let Some(candidate) =
                    buffers.next(&position, &generator, nnue, &ranks, &capture_history)
                {
                    if candidate.captured_value > threshold
                        || captures_last_royal(&position, candidate.capture.mv)
                    {
                        actual.push(candidate.capture.mv);
                    }
                }
                let expected: Vec<_> = reference(&position, &generator, nnue, tt_move)
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

// 主探索のキー前計算は、同点時の
// 安定性と置換表の手の扱いを含めて、公開捕獲列の安定整列と一致する。
#[test]
fn main_picker_captures_match_stable_reference_for_all_rules() {
    let capture_history = crate::search::alphabeta::capture_history::CaptureHistory::new();
    let nnue = test_nnue();
    let history = Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        for position in capture_test_positions() {
            let ordered = public_reference(&position, &generator, nnue, None);
            for tt_move in [None, ordered.get(ordered.len() / 2).map(|&(mv, _)| mv)] {
                let mut picker = MovePicker::new(tt_move, [None; KILLER_COUNT]);
                let mut actual = Vec::new();
                while let Some((mv, is_capture)) =
                    picker.next(&position, nnue, &generator, &history, &capture_history)
                {
                    if !is_capture {
                        break;
                    }
                    actual.push(mv);
                }
                let expected: Vec<_> = public_reference(&position, &generator, nnue, tt_move)
                    .into_iter()
                    .map(|(mv, _)| mv)
                    .collect();
                assert_eq!(actual, expected, "rules={rules:?}, tt={tt_move:?}");
            }
        }
    }
}

// 候補の有無は、合法な捕獲の参照列に
// 入口の閾値と最後の王駒の例外を適用した結果と一致する。
#[test]
fn initialized_candidate_presence_matches_filtered_reference() {
    let nnue = test_nnue();
    let ranks = CaptureRanks::new(nnue);
    let mut buffers = QsearchBuffers::default();
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        for position in capture_test_positions() {
            let ordered = reference(&position, &generator, nnue, None);
            let mut thresholds = vec![-1, i32::MAX / 2];
            thresholds.extend(ordered.iter().map(|(_, key)| key.captured_value));
            thresholds.sort_unstable();
            thresholds.dedup();
            for threshold in thresholds {
                let expected = ordered.iter().any(|&(mv, key)| {
                    key.captured_value > threshold || captures_last_royal(&position, mv)
                });
                buffers.reset(&position);
                assert_eq!(
                    buffers.initialize(&position, &generator, nnue, &ranks, threshold),
                    expected,
                    "rules={rules:?}, threshold={threshold}, position={position:?}",
                );
            }
        }
    }
}

/// 捕獲履歴の尺度が0なら、捕獲価値の降順、攻撃駒価値の昇順、生成順に整列する。
#[cfg(feature = "tuning")]
pub(super) fn zero_capture_scale_preserves_reference_order() {
    let mut capture_history = crate::search::alphabeta::capture_history::CaptureHistory::new();
    let nnue = test_nnue();
    let history = Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]);
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        for position in capture_test_positions() {
            let ordered = public_reference(&position, &generator, nnue, None);
            // 同順位も含む基準列に、正負の非ゼロ履歴を付けてから比較する。
            for pair in ordered.chunks(2) {
                let earlier: Vec<_> = pair.iter().skip(1).map(|&(mv, _)| mv).collect();
                capture_history.record_cutoff(&position, nnue, pair[0].0, &earlier, 256);
                assert_ne!(capture_history.read(&position, nnue, pair[0].0), 0);
            }
            for tt_move in [None, ordered.get(ordered.len() / 2).map(|&(mv, _)| mv)] {
                let mut picker = MovePicker::new(tt_move, [None; KILLER_COUNT]);
                let mut actual = Vec::new();
                while let Some((mv, is_capture)) =
                    picker.next(&position, nnue, &generator, &history, &capture_history)
                {
                    if !is_capture {
                        break;
                    }
                    actual.push(mv);
                }
                let expected: Vec<_> = public_reference(&position, &generator, nnue, tt_move)
                    .into_iter()
                    .map(|(mv, _)| mv)
                    .collect();
                assert_eq!(actual, expected, "rules={rules:?}, tt={tt_move:?}");
            }
        }
    }
}
