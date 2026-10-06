//! 学習PSTの復号、差分更新および整数評価を検査する。

use sha2::{Digest, Sha256};

use super::accumulator::PstAccumulator;
use super::features::feature_index;
use super::features::{PIECE_STATE_COUNT, piece_state};
use super::format::HEADER_LENGTH;
use super::*;
use crate::eval::handcrafted::piece_value;
use minase_core::Move;
use minase_core::test_util::{position_from_codes, reflect_ranks_and_swap_colors, sq};
use minase_core::{Color, MoveRules, PieceCode, PieceKind, Square};

/// 検査用の正しいMNPTバイト列を返す。
fn valid_bytes() -> Vec<u8> {
    include_bytes!("../../../nets/pst-init.bin").to_vec()
}

/// 盤上に現れ得る駒状態を代表する先手の駒コードを返す。
fn reachable_piece_states() -> Vec<PieceCode> {
    let mut pieces = Vec::new();
    for kind in PieceKind::ALL {
        if kind.can_promote() {
            pieces.push(PieceCode::new(Color::Black, kind).unwrap());
            if kind.unpromoted().is_some() {
                pieces.push(PieceCode::new_promoted(Color::Black, kind).unwrap());
            }
        } else {
            pieces.push(
                PieceCode::new(Color::Black, kind)
                    .or_else(|| PieceCode::new_promoted(Color::Black, kind))
                    .unwrap(),
            );
        }
    }
    assert_eq!(pieces.len(), PIECE_STATE_COUNT - 10);
    pieces
}

/// MNPT本体の指定特徴の重みを書き換える。
fn set_weight(bytes: &mut [u8], endpoint: usize, feature: usize, weight: i16) {
    let offset = HEADER_LENGTH + (endpoint * FEATURE_COUNT + feature) * 2;
    bytes[offset..offset + 2].copy_from_slice(&weight.to_le_bytes());
}

/// 探索用駒価値の指定状態を書き換える。
fn set_piece_value(bytes: &mut [u8], state: usize, value: i32) {
    let offset = HEADER_LENGTH + FEATURE_COUNT * 4 + state * 4;
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// 全特徴で端点が異なり、升と駒状態の差分も検出できる重みを作る。
fn distinct_pst() -> Pst {
    let mut bytes = valid_bytes();
    for feature in 0..FEATURE_COUNT {
        set_weight(&mut bytes, 1, feature, (feature % 997) as i16 - 498);
    }
    refresh_checksum(&mut bytes);
    Pst::decode(&bytes).unwrap()
}

/// 盤面全体を指定数の駒で埋め、最初の2枚を王にする。
fn position_with_count(count: usize, side: Color) -> Position {
    let pieces: Vec<_> = Square::all()
        .take(count)
        .enumerate()
        .map(|(index, square)| {
            let (color, kind) = match index {
                0 => (Color::Black, PieceKind::King),
                1 => (Color::White, PieceKind::King),
                _ => (Color::Black, PieceKind::Pawn),
            };
            (square, PieceCode::new(color, kind).unwrap())
        })
        .collect();
    position_from_codes(side, &pieces)
}

/// 公開評価と探索用累算評価を、仕様から求めた期待値と照合する。
fn assert_evaluation(pst: &Pst, position: &Position, expected: i32) {
    assert_eq!(evaluate(pst, position), expected);
    assert_eq!(
        pst.evaluate_accumulator(&pst.refresh_accumulator(position), position.side_to_move()),
        expected
    );
}

/// MNPT本体を変更した後のSHA-256をヘッダへ反映する。
fn refresh_checksum(bytes: &mut [u8]) {
    let checksum: [u8; 32] = Sha256::digest(&bytes[HEADER_LENGTH..]).into();
    bytes[48..80].copy_from_slice(&checksum);
}

/// 指定局面の手番側駒価値差を計算する。
fn material_score(position: &Position) -> i32 {
    let perspective = position.side_to_move();
    Square::all()
        .filter_map(|square| position.piece_at(square))
        .map(|piece| {
            let sign = if piece.color() == Some(perspective) {
                1
            } else {
                -1
            };
            sign * piece_value(piece.kind().unwrap())
        })
        .sum()
}

/// 着手前後の差分累算値が完全再計算と一致し、親累算値を変更しないことを検査する。
fn assert_move_accumulator(pst: &Pst, position: &mut Position, mv: Move) {
    let before = pst.refresh_accumulator(position);
    let undo = position.make_move_unchecked(mv, MoveRules::standard());
    let mut after = PstAccumulator::default();
    pst.update_accumulator_after_move(&before, &mut after, position, &undo);
    assert_eq!(after, pst.refresh_accumulator(position));
    assert_eq!(
        pst.evaluate_accumulator(&after, position.side_to_move()),
        evaluate(pst, position)
    );
    position.unmake_move(undo);
    assert_eq!(before, pst.refresh_accumulator(position));
}

/// 差分累算値が通常手、特殊移動、先獅子状態、およびnull moveで完全再計算と一致する。
#[test]
fn accumulator_updates_match_full_refresh_across_move_shapes() {
    let pst = &fm_tests::synthetic_fm_pst();
    let black_king = PieceCode::new(Color::Black, PieceKind::King).unwrap();
    let white_king = PieceCode::new(Color::White, PieceKind::King).unwrap();
    let black_pawn = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
    let white_pawn = PieceCode::new(Color::White, PieceKind::Pawn).unwrap();
    let black_lion = PieceCode::new(Color::Black, PieceKind::Lion).unwrap();

    let mut promotion = position_from_codes(
        Color::Black,
        &[
            (sq(0, 11), black_king),
            (sq(11, 0), white_king),
            (sq(5, 3), black_pawn),
            (sq(5, 2), white_pawn),
        ],
    );
    assert_move_accumulator(
        pst,
        &mut promotion,
        Move {
            from: sq(5, 3),
            mid: None,
            to: sq(5, 2),
            promote: true,
        },
    );

    let mut lion = position_from_codes(
        Color::Black,
        &[
            (sq(0, 11), black_king),
            (sq(11, 0), white_king),
            (sq(5, 5), black_lion),
            (sq(5, 4), white_pawn),
            (sq(5, 3), white_pawn),
        ],
    );
    assert_move_accumulator(
        pst,
        &mut lion,
        Move {
            from: sq(5, 5),
            mid: Some(sq(5, 4)),
            to: sq(5, 3),
            promote: false,
        },
    );

    let mut igui = position_from_codes(
        Color::Black,
        &[
            (sq(0, 11), black_king),
            (sq(11, 0), white_king),
            (sq(5, 5), black_lion),
            (sq(5, 4), white_pawn),
        ],
    );
    assert_move_accumulator(
        pst,
        &mut igui,
        Move {
            from: sq(5, 5),
            mid: Some(sq(5, 4)),
            to: sq(5, 5),
            promote: false,
        },
    );

    let mut jitto = position_from_codes(
        Color::Black,
        &[
            (sq(0, 11), black_king),
            (sq(11, 0), white_king),
            (sq(5, 5), black_lion),
        ],
    );
    assert_move_accumulator(
        pst,
        &mut jitto,
        Move {
            from: sq(5, 5),
            mid: Some(sq(5, 4)),
            to: sq(5, 5),
            promote: false,
        },
    );

    let black_rook = PieceCode::new(Color::Black, PieceKind::Rook).unwrap();
    let white_lion = PieceCode::new(Color::White, PieceKind::Lion).unwrap();
    let mut lion_capture = position_from_codes(
        Color::Black,
        &[
            (sq(0, 11), black_king),
            (sq(11, 0), white_king),
            (sq(5, 5), black_rook),
            (sq(5, 3), white_lion),
        ],
    );
    let before = pst.refresh_accumulator(&lion_capture);
    let undo = lion_capture.make_move_unchecked(
        Move {
            from: sq(5, 5),
            mid: None,
            to: sq(5, 3),
            promote: false,
        },
        MoveRules::standard(),
    );
    let mut after = PstAccumulator::default();
    pst.update_accumulator_after_move(&before, &mut after, &lion_capture, &undo);
    assert_eq!(after, pst.refresh_accumulator(&lion_capture));
    assert_eq!(after.piece_count, 3);
    assert!(lion_capture.lion_taken_by_non_lion().is_some());
    assert_eq!(
        pst.evaluate_accumulator(&after, lion_capture.side_to_move()),
        evaluate(pst, &lion_capture)
    );

    let mut normal_response = lion_capture.clone();
    assert_move_accumulator(
        pst,
        &mut normal_response,
        Move {
            from: sq(11, 0),
            mid: None,
            to: sq(10, 0),
            promote: false,
        },
    );

    let lion_before = lion_capture
        .lion_taken_by_non_lion()
        .map(|trigger| trigger.square);
    let null_undo = lion_capture.make_null_move();
    let mut after_null = PstAccumulator::default();
    pst.update_accumulator_after_null(&after, &mut after_null, lion_before);
    assert_eq!(after_null, pst.refresh_accumulator(&lion_capture));
    assert_eq!(after_null.piece_count, 3);
    assert_eq!(
        pst.evaluate_accumulator(&after_null, lion_capture.side_to_move()),
        evaluate(pst, &lion_capture)
    );
    lion_capture.unmake_null_move(null_undo);
    assert_eq!(after, pst.refresh_accumulator(&lion_capture));
    assert_eq!(after.piece_count, 3);
    assert!(lion_capture.lion_taken_by_non_lion().is_some());
    assert_eq!(
        pst.evaluate_accumulator(&after, lion_capture.side_to_move()),
        evaluate(pst, &lion_capture)
    );
    lion_capture.unmake_move(undo);
    assert_eq!(before, pst.refresh_accumulator(&lion_capture));
}

/// 固定長と異なるMNPTが拒否されることを検査する。
#[test]
fn decode_rejects_invalid_length() {
    for length in [0, 79, 54_987, 54_989] {
        let mut bytes = valid_bytes();
        bytes.resize(length, 0);
        assert!(
            matches!(Pst::decode(&bytes), Err(Error::InvalidLength { expected: 54_988, actual }) if actual == length)
        );
    }
}

/// MNPT識別子の不一致が拒否されることを検査する。
#[test]
fn decode_rejects_invalid_magic() {
    let mut bytes = valid_bytes();
    bytes[0] ^= 1;
    assert!(matches!(
        Pst::decode(&bytes),
        Err(Error::InvalidMagic { .. })
    ));
}

/// 未対応のMNPT版が拒否されることを検査する。
#[test]
fn decode_rejects_unsupported_version() {
    let mut bytes = valid_bytes();
    bytes[4..8].copy_from_slice(&1_u32.to_le_bytes());
    assert!(matches!(
        Pst::decode(&bytes),
        Err(Error::UnsupportedVersion { actual: 1 })
    ));
}

/// MNPT特徴数の不一致が拒否されることを検査する。
#[test]
fn decode_rejects_unexpected_feature_count() {
    let mut bytes = valid_bytes();
    bytes[8] ^= 1;
    assert!(matches!(
        Pst::decode(&bytes),
        Err(Error::UnexpectedFeatureCount { .. })
    ));
}

/// 規則セット名欄のNUL埋め違反が拒否されることを検査する。
#[test]
fn decode_rejects_invalid_rule_set_field() {
    let mut bytes = valid_bytes();
    bytes[47] = b'x';
    assert!(matches!(Pst::decode(&bytes), Err(Error::InvalidRuleSet)));
}

/// v0初期重みが先獅子のない局面の駒価値差と一致することを検査する。
#[test]
fn initialized_pst_matches_material_evaluation() {
    let pst = Pst::decode(include_bytes!("../../../nets/pst-init.bin")).unwrap();
    let promoted_gold = PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap();
    let promoted_lion = PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap();
    let positions = [
        Position::initial(),
        position_from_codes(
            Color::Black,
            &[
                (sq(1, 2), promoted_gold),
                (sq(7, 8), promoted_lion),
                (
                    sq(4, 6),
                    PieceCode::new(Color::White, PieceKind::King).unwrap(),
                ),
            ],
        ),
        position_from_codes(
            Color::White,
            &[
                (
                    sq(0, 0),
                    PieceCode::new(Color::Black, PieceKind::Pawn).unwrap(),
                ),
                (
                    sq(11, 11),
                    PieceCode::new(Color::White, PieceKind::FreeKing).unwrap(),
                ),
            ],
        ),
    ];
    for position in positions {
        assert_eq!(evaluate(&pst, &position), material_score(&position));
    }
}

/// 初期PSTに格納された全駒価値が固定した表と一致することを検査する。
#[test]
fn initialized_pst_derives_the_frozen_piece_values() {
    let pst = Pst::decode(include_bytes!("../../../nets/pst-init.bin")).unwrap();
    for piece in reachable_piece_states() {
        let kind = piece.kind().unwrap();
        if !matches!(kind, PieceKind::King | PieceKind::CrownPrince) {
            assert_eq!(pst.piece_value(piece), piece_value(kind), "piece={piece:?}");
        }
    }

    let king = PieceCode::new(Color::Black, PieceKind::King).unwrap();
    let prince = PieceCode::new_promoted(Color::Black, PieceKind::CrownPrince).unwrap();
    assert_eq!(pst.piece_value(king), 2_600);
    assert_eq!(pst.piece_value(prince), 2_600);
}

/// 分子が負でも0方向へ切り捨て、端点ごとの除算を行わない。
#[test]
fn interpolation_divides_once_and_truncates_toward_zero() {
    let position = position_with_count(3, Color::Black);
    let square = position.occupied().into_iter().next().unwrap();
    let feature = feature_index(Color::Black, position.piece_at(square).unwrap(), square);
    // 3枚ならq=1。分子はmg + 89*egであり、±719と±720を境界として検査する。
    for (mg, eg, expected) in [
        (-7, -8, 0),
        (-8, -8, -1),
        (7, 8, 0),
        (8, 8, 1),
        (-9, -7, 0),
        (9, 7, 0),
    ] {
        let mut bytes = valid_bytes();
        bytes[HEADER_LENGTH..HEADER_LENGTH + FEATURE_COUNT * 4].fill(0);
        set_weight(&mut bytes, 0, feature, mg);
        set_weight(&mut bytes, 1, feature, eg);
        refresh_checksum(&mut bytes);
        assert_evaluation(&Pst::decode(&bytes).unwrap(), &position, expected);
    }
}

/// 勝率尺度は正かつ有限の値だけを受け入れる。
#[test]
fn decode_rejects_invalid_k() {
    for k in [0.0_f32, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut bytes = valid_bytes();
        bytes[12..16].copy_from_slice(&k.to_le_bytes());
        assert!(matches!(Pst::decode(&bytes), Err(Error::InvalidK { .. })));
    }
}

/// 王と太子は最大非王駒価値と歩価値の和に厳密に一致する必要がある。
#[test]
fn decode_rejects_inconsistent_royal_values() {
    for kind in [PieceKind::King, PieceKind::CrownPrince] {
        for actual in [2_599, 2_601] {
            let mut bytes = valid_bytes();
            set_piece_value(&mut bytes, kind.index(), actual);
            refresh_checksum(&mut bytes);
            assert!(
                matches!(Pst::decode(&bytes), Err(Error::InconsistentRoyalValue { kind: found, expected: 2_600, actual: value }) if found == kind && value == actual)
            );
        }
    }
}

/// 未到達状態を含む全47値に探索上の数値範囲を適用する。
#[test]
fn decode_rejects_out_of_range_piece_values() {
    let cases = (0..PIECE_STATE_COUNT)
        .flat_map(|state| [-1, 29_000].map(|value| (state, value)))
        .chain([(0, i32::MIN), (0, i32::MAX)]);
    for (state, value) in cases {
        let mut bytes = valid_bytes();
        set_piece_value(&mut bytes, state, value);
        refresh_checksum(&mut bytes);
        assert!(
            matches!(Pst::decode(&bytes), Err(Error::PieceValueOutOfRange { state: found, value: actual }) if found == state && actual == value)
        );
    }
}

/// 検査和は両端点と探索用駒価値を含む本体全体を対象にする。
#[test]
fn checksum_covers_both_endpoints_and_piece_values() {
    let bytes = valid_bytes();
    assert_eq!(bytes.len(), 54_988);
    let pst = Pst::decode(&bytes).unwrap();
    let expected: [u8; 32] = Sha256::digest(&bytes[80..]).into();
    assert_eq!(pst.checksum(), &expected);
    assert_eq!(
        pst.k(),
        f32::from_le_bytes(bytes[12..16].try_into().unwrap())
    );
    for offset in [80, 80 + 13_680 * 2, 80 + 13_680 * 4, bytes.len() - 1] {
        let mut corrupt = bytes.clone();
        corrupt[offset] ^= 1;
        assert!(matches!(
            Pst::decode(&corrupt),
            Err(Error::ChecksumMismatch)
        ));
    }
}

/// 全到達可能な非王駒状態の0を拒否し、端点の変更では探索用駒価値を変えない。
#[test]
fn stored_piece_values_are_validated_independently_of_weights() {
    let initial = Pst::decode(&valid_bytes()).unwrap();
    let distinct = distinct_pst();
    assert_eq!(initial.piece_values, distinct.piece_values);
    for piece in reachable_piece_states() {
        if matches!(piece.kind(), Some(PieceKind::King | PieceKind::CrownPrince)) {
            continue;
        }
        let mut bytes = valid_bytes();
        set_piece_value(&mut bytes, piece_state(piece), 0);
        refresh_checksum(&mut bytes);
        assert!(matches!(
            Pst::decode(&bytes),
            Err(Error::NonPositivePieceValue { kind, promoted, value: 0 })
                if Some(kind) == piece.kind() && promoted == piece.is_promoted()
        ));
    }
}

/// 埋め込み重みが復号でき、初期局面評価がPython学習器と一致することを検査する。
#[test]
fn embedded_pst_matches_python_initial_position_evaluation() {
    // pst-longer-training-training の候補Lの学習ログ（Python整数参照評価）による値。
    assert_eq!(evaluate(&weights().unwrap(), &Position::initial()), 33);
}

/// debugging-tools.md「eval」: 特徴別の分子和、全計算評価、720での除算と上下限制限が一致する。
#[test]
fn diagnostic_numerators_match_independent_feature_sum_and_clamping() {
    for endpoints in [[8_i16, -24_i16], [i16::MAX; 2], [i16::MIN; 2]] {
        let mut pst = Pst::decode(&valid_bytes()).unwrap();
        pst.weights.fill(endpoints);
        // 先獅子を盤上の駒とは異なる寄与にして、欠落や二重計上を検出する。
        pst.weights[super::features::BOARD_FEATURE_COUNT..].fill([31, -57]);
        for count in [0, 1, 2, 3, 47, 92, 100, 144] {
            for side in Color::ALL {
                let mut position = position_with_count(count, side);
                let trigger = if count < 2 {
                    sq(11, 11)
                } else if side == Color::Black {
                    Square::from_dense(1).unwrap()
                } else {
                    Square::from_dense(0).unwrap()
                };
                position.set_lion_capture(Some(trigger)).unwrap();
                let detail = breakdown(&pst, &position);
                let q = (count as i64 - 2).clamp(0, 90);
                let per_piece = q * i64::from(endpoints[0]) + (90 - q) * i64::from(endpoints[1]);
                let lion = q * 31 + (90 - q) * -57;
                let numerator = count as i64 * per_piece + lion;
                assert_eq!(detail.q, q);
                assert_eq!(detail.board.iter().sum::<i64>() + detail.lion, numerator);
                assert_eq!(detail.lion, lion);
                for square in Square::all() {
                    assert_eq!(
                        detail.board[square.dense_index()],
                        if position.piece_at(square).is_some() {
                            per_piece
                        } else {
                            0
                        }
                    );
                }
                let expected = (numerator / 720).clamp(-28_999, 28_999) as i32;
                assert_evaluation(&pst, &position, expected);
                assert_eq!(pst.refresh_accumulator(&position).piece_count, count as u32);
                assert_eq!(detail.score, expected);
            }
        }
    }
}

/// debugging-tools.md「eval」: 升ごとの寄与は手番視点の特徴に対応し、先獅子と区別する。
#[test]
fn diagnostic_contributions_use_the_correct_square_and_perspective() {
    let pst = distinct_pst();
    for side in Color::ALL {
        let mut position = position_with_count(47, side);
        position.set_lion_capture(Some(sq(11, 11))).unwrap();
        let detail = breakdown(&pst, &position);
        let mut sums = [0_i64; 2];
        for square in Square::all() {
            let expected = if let Some(piece) = position.piece_at(square) {
                let weights = pst.weights[feature_index(side, piece, square)];
                sums[0] += i64::from(weights[0]);
                sums[1] += i64::from(weights[1]);
                45 * i64::from(weights[0]) + 45 * i64::from(weights[1])
            } else {
                0
            };
            assert_eq!(detail.board[square.dense_index()], expected);
        }
        let weights = pst.weights[super::features::lion_feature_index(side, sq(11, 11))];
        sums[0] += i64::from(weights[0]);
        sums[1] += i64::from(weights[1]);
        let numerator = 45 * sums[0] + 45 * sums[1];
        assert_eq!(detail.board.iter().sum::<i64>() + detail.lion, numerator);
        assert_eq!(detail.score, evaluate(&pst, &position));
        assert_eq!(
            detail.score,
            (numerator / 720).clamp(-28_999, 28_999) as i32
        );
    }
}

/// debugging-tools.md「適用範囲」「棄却した代案」: 段反転と陣営交換は手番側評価を保存する。
#[test]
fn random_legal_positions_preserve_evaluation_under_rank_and_color_reflection() {
    use minase_core::MoveGenerator;
    use minase_core::rng::{XorShift64, derive_seed};

    let learned = weights().unwrap();
    let distinct = distinct_pst();
    let rules = MoveRules::standard();
    let generator = MoveGenerator::new(rules);
    for game in 0..24 {
        let mut rng = XorShift64::new(derive_seed(0x5245_464c_4543_5431, game));
        let mut position = Position::initial();
        let mut moves = Vec::new();
        for ply in 1..=160 {
            if Color::ALL
                .into_iter()
                .any(|side| position.royal_pieces(side).is_empty())
            {
                break;
            }
            moves.clear();
            generator.generate_moves(&position, &mut moves);
            if moves.is_empty() {
                break;
            }
            let mv = moves[rng.next() as usize % moves.len()];
            position.make_move_unchecked(mv, rules);
            if ply % 17 == 0 {
                // 成り権の保留は評価特徴に含まれず、反転後は保留なしでよい。
                let reflected = reflect_ranks_and_swap_colors(&position);
                for pst in [&*learned, &distinct] {
                    assert_eq!(
                        evaluate(pst, &position),
                        evaluate(pst, &reflected),
                        "game={game}, ply={ply}, move={mv:?}"
                    );
                }
            }
        }
    }
}

/// debugging-tools.md「整合検査の内容」: 1項の不一致を検出し、局面と両累算値を診断する。
#[cfg(feature = "invariants")]
#[test]
fn accumulator_invariants_report_mismatch() {
    let pst = distinct_pst();
    let position = Position::initial();
    let recomputed = pst.refresh_accumulator(&position);
    // 手番ではない視点の1だけの差も、丸められた評価値ではなく中間値で検出する。
    let mut incremental = recomputed;
    incremental.sums[Color::White.index()][1] += 1;
    let panic = std::panic::catch_unwind(|| pst.assert_accumulator(&position, &incremental, 7))
        .expect_err("an inconsistent accumulator must panic");
    let message = panic.downcast_ref::<String>().unwrap();
    assert!(message.contains("PST accumulator mismatch"));
    assert!(message.contains(&format!("zobrist={:#018x}", position.zobrist())));
    assert!(message.contains("ply=7"));
    assert!(message.contains(&format!("incremental: {incremental:?}")));
    assert!(message.contains(&format!("recomputed: {recomputed:?}")));
    let setup = minase_core::notation::sfen::SetupPosition::new(position, None, 1).unwrap();
    assert!(message.contains(&minase_core::notation::sfen::to_extended_sfen(&setup)));
}

mod fm_tests {
    use super::*;
    /// v2の本体を保ち、補正0のFM節を加える。
    fn valid_fm_bytes() -> Vec<u8> {
        let mut bytes = valid_bytes();
        bytes[4..8].copy_from_slice(&3_u32.to_le_bytes());
        bytes.extend_from_slice(&(FM_RANK as u32).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&[1; FM_RANK]);
        bytes.resize(bytes.len() + FEATURE_COUNT * FM_RANK * 2, 0);
        refresh_checksum(&mut bytes);
        bytes
    }

    /// 未観測行も非零とし、全特徴の取り違えを検出できるv3重みを作る。
    pub(super) fn synthetic_fm_pst() -> Pst {
        let mut bytes = valid_fm_bytes();
        let offset = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
        bytes[offset + 4..offset + 8].copy_from_slice(&3_u32.to_le_bytes());
        for f in 0..FM_RANK {
            bytes[offset + 8 + f] = if f % 3 == 0 { 255 } else { 1 };
        }
        let mut state = 1_u32;
        for pair in bytes[offset + 8 + FM_RANK..].as_chunks_mut::<2>().0 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let value = ((state >> 16) % 201) as i16 - 100;
            pair.copy_from_slice(&value.to_le_bytes());
        }
        refresh_checksum(&mut bytes);
        Pst::decode(&bytes).unwrap()
    }

    /// 検査和が正しくても、次元・指数・各次元の符号が定義域外なら拒否する。
    #[test]
    fn decode_rejects_invalid_fm_parameters() {
        let offset = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
        for rank in [0, 1, FM_RANK as u32 + 1, u32::MAX] {
            let mut bytes = valid_fm_bytes();
            bytes[offset..offset + 4].copy_from_slice(&rank.to_le_bytes());
            refresh_checksum(&mut bytes);
            assert!(
                matches!(Pst::decode(&bytes), Err(Error::UnexpectedFmRank { actual }) if actual == rank)
            );
        }
        for exponent in [31, u32::MAX] {
            let mut bytes = valid_fm_bytes();
            bytes[offset + 4..offset + 8].copy_from_slice(&exponent.to_le_bytes());
            refresh_checksum(&mut bytes);
            assert!(
                matches!(Pst::decode(&bytes), Err(Error::InvalidFmExponent { actual }) if actual == exponent)
            );
        }
        for dimension in 0..FM_RANK {
            for sign in [0_i8, 2, -2, i8::MIN, i8::MAX] {
                let mut bytes = valid_fm_bytes();
                bytes[offset + 8 + dimension] = sign as u8;
                refresh_checksum(&mut bytes);
                assert!(
                    matches!(Pst::decode(&bytes), Err(Error::InvalidFmSign { dimension: found, actual }) if found == dimension && actual == sign)
                );
            }
        }
    }

    /// 全零FMはPSTの補間値を、駒数・手番・先獅子状態によらず保存する。
    #[test]
    fn zero_fm_preserves_pst_interpolation_exactly() {
        let baseline = distinct_pst();
        let mut bytes = valid_fm_bytes();
        let baseline_bytes = baseline.encode();
        bytes[HEADER_LENGTH..baseline_bytes.len()]
            .copy_from_slice(&baseline_bytes[HEADER_LENGTH..]);
        refresh_checksum(&mut bytes);
        let pst = Pst::decode(&bytes).unwrap();
        for count in [0, 1, 2, 3, 47, 92, 93, 144] {
            for side in Color::ALL {
                let mut position = position_with_count(count, side);
                for with_lion in [false, true] {
                    if with_lion {
                        let square = Square::all()
                            .find(|&sq| {
                                position
                                    .piece_at(sq)
                                    .is_none_or(|p| p.color() != Some(side))
                            })
                            .unwrap();
                        position.set_lion_capture(Some(square)).unwrap();
                    }
                    let accumulator = pst.refresh_accumulator(&position);
                    let expected = interpolate(accumulator.sums[side.index()], count as u32);
                    assert_eq!(evaluate_pst(&pst, &position), expected);
                    assert_evaluation(&pst, &position, expected);
                }
            }
        }
    }

    /// FMの大きな補正は64ビットで合算した後に、正負とも評価上限へ切り詰める。
    #[test]
    fn extreme_fm_clips_after_adding_to_clipped_pst() {
        let mut position = position_with_count(144, Color::Black);
        position.set_lion_capture(Some(sq(1, 0))).unwrap();
        for u in [i16::MIN, i16::MAX] {
            for sign in [-1_i8, 1] {
                let mut bytes = valid_fm_bytes();
                let offset = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
                bytes[offset + 8..offset + 8 + FM_RANK].fill(sign as u8);
                for pair in bytes[offset + 8 + FM_RANK..].as_chunks_mut::<2>().0 {
                    pair.copy_from_slice(&u.to_le_bytes());
                }
                refresh_checksum(&mut bytes);
                let pst = Pst::decode(&bytes).unwrap();
                assert_evaluation(&pst, &position, i32::from(sign) * 28_999);
            }
        }
        // PST自体を先に切り詰める契約。生PSTは上限を大きく超えるが、FMは−1cp。
        let mut bytes = valid_fm_bytes();
        for pair in bytes[HEADER_LENGTH..HEADER_LENGTH + FEATURE_COUNT * 4]
            .as_chunks_mut::<2>()
            .0
        {
            pair.copy_from_slice(&i16::MAX.to_le_bytes());
        }
        let offset = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4 + 8 + FM_RANK;
        let mut features = Vec::new();
        active_features(&position, |i| features.push(i));
        for (i, u) in [(features[0], 1_i16), (features[1], -1)] {
            bytes[offset + i * FM_RANK * 2..offset + i * FM_RANK * 2 + 2]
                .copy_from_slice(&u.to_le_bytes());
        }
        refresh_checksum(&mut bytes);
        assert_evaluation(&Pst::decode(&bytes).unwrap(), &position, 28_998);
    }

    /// 太子への成りと王・太子の捕獲でも、累算値と取消が全再計算に一致する。
    #[test]
    fn royal_promotion_and_captures_update_all_features() {
        let pst = synthetic_fm_pst();
        let king = PieceCode::new(Color::Black, PieceKind::King).unwrap();
        let enemy_king = PieceCode::new(Color::White, PieceKind::King).unwrap();
        let elephant = PieceCode::new(Color::Black, PieceKind::DrunkElephant).unwrap();
        let mut position = position_from_codes(
            Color::Black,
            &[
                (sq(0, 11), king),
                (sq(11, 0), enemy_king),
                (sq(5, 7), elephant),
            ],
        );
        assert_move_accumulator(
            &pst,
            &mut position,
            Move {
                from: sq(5, 7),
                mid: None,
                to: sq(5, 8),
                promote: true,
            },
        );
        for victim in [
            enemy_king,
            PieceCode::new_promoted(Color::White, PieceKind::CrownPrince).unwrap(),
        ] {
            let rook = PieceCode::new(Color::Black, PieceKind::Rook).unwrap();
            let mut position = position_from_codes(
                Color::Black,
                &[(sq(0, 11), king), (sq(5, 7), rook), (sq(5, 8), victim)],
            );
            assert_move_accumulator(
                &pst,
                &mut position,
                Move {
                    from: sq(5, 7),
                    mid: None,
                    to: sq(5, 8),
                    promote: false,
                },
            );
        }
    }

    /// 長い合法着手列の各段階と、その逆順の取消で両視点の累算値を保存する。
    #[test]
    fn long_move_sequence_and_undo_restore_accumulators() {
        let generator = minase_core::MoveGenerator::standard();
        for pst in [synthetic_fm_pst(), Pst::decode(EMBEDDED).unwrap()] {
            let mut position = Position::initial();
            let original = pst.refresh_accumulator(&position);
            let mut accumulator = original;
            // 探索と同様に子の格納先を再利用し、前回の値が残らないことも確かめる。
            let mut after = PstAccumulator::default();
            let mut after_null = PstAccumulator::default();
            let mut history = Vec::new();
            let mut state = 1_u32;
            for _ in 0..256 {
                let mut moves = Vec::new();
                generator.generate_moves(&position, &mut moves);
                moves.retain(|&mv| {
                    position
                        .captured_squares(mv)
                        .into_iter()
                        .flatten()
                        .all(|sq| {
                            !matches!(
                                position.piece_at(sq).unwrap().kind(),
                                Some(PieceKind::King | PieceKind::CrownPrince)
                            )
                        })
                });
                assert!(!moves.is_empty());
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let undo = position.make_move_unchecked(
                    moves[state as usize % moves.len()],
                    MoveRules::standard(),
                );
                pst.update_accumulator_after_move(&accumulator, &mut after, &position, &undo);
                history.push((undo, accumulator));
                accumulator = after;
                assert_eq!(accumulator, pst.refresh_accumulator(&position));
                assert_evaluation(
                    &pst,
                    &position,
                    pst.evaluate_accumulator(&accumulator, position.side_to_move()),
                );
                let lion_before = position
                    .lion_taken_by_non_lion()
                    .map(|trigger| trigger.square);
                let undo = position.make_null_move();
                pst.update_accumulator_after_null(&accumulator, &mut after_null, lion_before);
                assert_eq!(after_null, pst.refresh_accumulator(&position));
                assert_evaluation(
                    &pst,
                    &position,
                    pst.evaluate_accumulator(&after_null, position.side_to_move()),
                );
                position.unmake_null_move(undo);
                assert_eq!(accumulator, pst.refresh_accumulator(&position));
            }
            for (undo, parent) in history.into_iter().rev() {
                position.unmake_move(undo);
                accumulator = parent;
                assert_eq!(accumulator, pst.refresh_accumulator(&position));
                assert_evaluation(
                    &pst,
                    &position,
                    pst.evaluate_accumulator(&accumulator, position.side_to_move()),
                );
            }
            assert_eq!(position, Position::initial());
            assert_eq!(accumulator, original);
        }
    }
    #[test]
    fn diagnostic_perspectives_preserve_lion_feature_and_use_selected_fm_view() {
        // PSTは先獅子特徴だけが100/200cp、FMはその特徴と王の積6/20cp。
        // 視点変更で先獅子を消す、符号だけ反転する、FM視点を固定する実装を検出する。
        let mut bytes = valid_fm_bytes();
        bytes[HEADER_LENGTH..HEADER_LENGTH + FEATURE_COUNT * 4].fill(0);
        let black_king = PieceCode::new(Color::Black, PieceKind::King).unwrap();
        let white_king = PieceCode::new(Color::White, PieceKind::King).unwrap();
        let mut position = position_from_codes(
            Color::Black,
            &[(sq(0, 11), black_king), (sq(11, 0), white_king)],
        );
        position.set_lion_capture(Some(sq(5, 3))).unwrap();
        let original_hash = position.zobrist();
        let fm_start = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
        let embedding_start = fm_start + 8 + FM_RANK;
        for (perspective, pst_weight, lion_embedding, king_embedding) in [
            (Color::Black, 800, 2_i16, 3_i16),
            (Color::White, 1600, 4_i16, 5_i16),
        ] {
            let lion = crate::eval::pst::features::lion_feature_index(perspective, sq(5, 3));
            for endpoint in 0..2 {
                set_weight(&mut bytes, endpoint, lion, pst_weight);
            }
            let king = feature_index(perspective, black_king, sq(0, 11));
            for (feature, value) in [(lion, lion_embedding), (king, king_embedding)] {
                let offset = embedding_start + feature * FM_RANK * 2;
                bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
            }
        }
        refresh_checksum(&mut bytes);
        let pst = Pst::decode(&bytes).unwrap();
        assert_eq!(
            evaluate_for_diagnostics(&pst, &position, Color::Black),
            (106, 100)
        );
        assert_eq!(
            evaluate_for_diagnostics(&pst, &position, Color::White),
            (220, 200)
        );
        assert_eq!(breakdown(&pst, &position).score, 106);
        assert_eq!(breakdown(&pst, &position).fm, Some(6));
        assert_eq!(position.side_to_move(), Color::Black);
        assert_eq!(position.zobrist(), original_hash);
        assert_eq!(position.lion_taken_by_non_lion().unwrap().square, sq(5, 3));
    }

    /// v2とv3のヘッダ、PST、駒価値、FM節を再符号化で保存する。
    #[test]
    fn mnpt_versions_round_trip_without_changing_bytes() {
        for bytes in [valid_bytes(), valid_fm_bytes(), synthetic_fm_pst().encode()] {
            assert_eq!(Pst::decode(&bytes).unwrap().encode(), bytes);
        }
    }

    /// v3はFM節を必須とし、切断、余分なバイト、検査和の不一致を拒否する。
    #[test]
    fn v3_rejects_invalid_length_and_checksum() {
        let bytes = valid_fm_bytes();
        for length in [0, 4, 79, 80, valid_bytes().len(), bytes.len() - 1] {
            assert!(matches!(
                Pst::decode(&bytes[..length]),
                Err(Error::InvalidLength { .. })
            ));
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(matches!(
            Pst::decode(&extra),
            Err(Error::InvalidLength { .. })
        ));
        let mut corrupt = bytes;
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(matches!(
            Pst::decode(&corrupt),
            Err(Error::ChecksumMismatch)
        ));
    }
}
