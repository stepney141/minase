//! 学習PSTの復号、差分更新および整数評価を検査する。

use sha2::{Digest, Sha256};

use super::features::feature_index;
use super::features::{PIECE_STATE_COUNT, piece_state};
use super::format::HEADER_LENGTH;
use super::*;
use crate::Move;
use crate::eval::handcrafted::piece_value;
use crate::test_util::{position_from_codes, reflect_ranks_and_swap_colors, sq};
use crate::{Color, MoveRules, PieceCode, PieceKind, Square};

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
        pst.evaluate_accumulator(pst.refresh_accumulator(position), position.side_to_move()),
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
    let after = pst.update_accumulator_after_move(before, position, &undo);
    assert_eq!(after, pst.refresh_accumulator(position));
    assert_eq!(
        pst.evaluate_accumulator(after, position.side_to_move()),
        evaluate(pst, position)
    );
    position.unmake_move(undo);
    assert_eq!(before, pst.refresh_accumulator(position));
}

/// 差分累算値が通常手、特殊移動、先獅子状態、およびnull moveで完全再計算と一致する。
#[test]
fn accumulator_updates_match_full_refresh_across_move_shapes() {
    let pst = &distinct_pst();
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
    let after = pst.update_accumulator_after_move(before, &lion_capture, &undo);
    assert_eq!(after, pst.refresh_accumulator(&lion_capture));
    assert_eq!(after.piece_count, 3);
    assert!(lion_capture.lion_taken_by_non_lion().is_some());
    assert_eq!(
        pst.evaluate_accumulator(after, lion_capture.side_to_move()),
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
    let after_null = pst.update_accumulator_after_null(after, lion_before);
    assert_eq!(after_null, pst.refresh_accumulator(&lion_capture));
    assert_eq!(after_null.piece_count, 3);
    assert_eq!(
        pst.evaluate_accumulator(after_null, lion_capture.side_to_move()),
        evaluate(pst, &lion_capture)
    );
    lion_capture.unmake_null_move(null_undo);
    assert_eq!(after, pst.refresh_accumulator(&lion_capture));
    assert_eq!(after.piece_count, 3);
    assert!(lion_capture.lion_taken_by_non_lion().is_some());
    assert_eq!(
        pst.evaluate_accumulator(after, lion_capture.side_to_move()),
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
    // pst-gen3-training の候補G23の学習ログ（Python整数参照評価）による値。
    assert_eq!(evaluate(&weights().unwrap(), &Position::initial()), 27);
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
    use crate::MoveGenerator;
    use crate::rng::{XorShift64, derive_seed};

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
    let panic = std::panic::catch_unwind(|| pst.assert_accumulator(&position, incremental, 7))
        .expect_err("an inconsistent accumulator must panic");
    let message = panic.downcast_ref::<String>().unwrap();
    assert!(message.contains("PST accumulator mismatch"));
    assert!(message.contains(&format!("zobrist={:#018x}", position.zobrist())));
    assert!(message.contains("ply=7"));
    assert!(message.contains(&format!("incremental: {incremental:?}")));
    assert!(message.contains(&format!("recomputed: {recomputed:?}")));
    let setup = crate::notation::sfen::SetupPosition::new(position, None, 1).unwrap();
    assert!(message.contains(&crate::notation::sfen::to_extended_sfen(&setup)));
}
