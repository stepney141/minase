//! 静止探索と捕獲の段階生成。

use core::cmp::Reverse;

use crate::eval::Nnue;
use crate::eval::{PIECE_STATE_COUNT, piece_state_of};
use crate::search::snapshot::search_key;
use crate::search::{MATE, MAX_PLY};
use minase_core::MoveGenerator;
use minase_core::board::Bitboard;
use minase_core::movegen::{CaptureCache, CaptureCandidate, OrdinaryCapturer};
use minase_core::mv::Move;
use minase_core::piece::{PIECE_KIND_COUNT, PieceCode, PieceKind};
use minase_core::position::Position;

use super::capture_history::CaptureHistory;
use super::ordering::piece_at_for_ordering;
use super::params;
use super::pruning::capture_is_pruned_by_see;
use super::royal::captures_all_royals;
use super::searcher::Searcher;
use super::tt::Bound;

impl Searcher<'_> {
    /// stand-patと捕獲手だけを使う静止探索で局面を評価する。
    ///
    /// 王駒がすべて狙われていてもstand-patを使い、非捕獲の王手と成りは展開しない。
    /// この近似の理由は`docs/adr/use-stand-pat-when-royals-are-attacked.md`にある。
    /// 捕獲ごとに盤上の駒が減って同一局面へ戻らないため、反復は検出しない。
    /// 静止探索内の手は主変化へ含めず、中断された場合は`None`を返す。
    ///
    /// 最大ply、stand-patのβ以上、入口での候補なしの順に判定し、該当すれば置換表を照会も保存もせず返す。
    /// その後の置換表ヒットによる打ち切りと中断も保存しない。
    /// 通常出口は深さ0で保存し、最善値がβ以上なら下界、入力時のα以下なら上界、それ以外は正確な値とする。
    /// stand-patを上回る捕獲手がなければ記録手は持たない。
    /// 深さ0の記録は通常探索の深さ条件を満たさず、カットオフに使われない。
    /// 保存の理由は`docs/adr/store-quiescence-results-at-depth-zero.md`にある。
    ///
    /// 捕獲候補は最後の王駒の捕獲を先に判定し、それなら枝刈りせず`MATE - (ply + 1)`を使う。
    /// それ以外は[`Self::delta_margin`]のdelta pruning、負のSEEによる枝刈り、手数制限の順に判定する。
    /// delta pruningは成り益を加えず、置換表の捕獲手もSEEの例外にしない。
    /// 成り益を除く理由は`docs/adr/keep-delta-pruning-without-promotion-gain.md`にある。
    ///
    /// 手数制限は、置換表の手も含めて実際に読んだ捕獲手が`QsearchMoveLimit`手に達した後に働く。
    /// 最後の王駒を取る手、直前の相手の捕獲手の到達升で取り返す手、獅子が2枚を取る手は、この制限だけの例外とする。
    /// 例外として読んだ手も手数に含めるが、枝刈りした候補は含めない。
    /// 数え方の理由は`docs/adr/limit-searched-captures-in-qsearch.md`にある。
    pub(super) fn quiesce(
        &mut self,
        position: &mut Position,
        mut alpha: i32,
        beta: i32,
        ply: u32,
    ) -> Option<i32> {
        #[cfg(feature = "search-stats")]
        {
            self.stats.quiesce_nodes += 1;
        }
        self.pv[ply as usize].clear();

        if ply >= MAX_PLY {
            #[cfg(feature = "invariants")]
            self.nnue
                .assert_accumulator(position, &self.accumulators[ply as usize], ply);
            return Some(
                self.nnue.evaluate_accumulator(
                    &self.accumulators[ply as usize],
                    position.side_to_move(),
                ),
            );
        }

        #[cfg(feature = "invariants")]
        self.nnue
            .assert_accumulator(position, &self.accumulators[ply as usize], ply);
        let stand_pat = self
            .nnue
            .evaluate_accumulator(&self.accumulators[ply as usize], position.side_to_move());
        if stand_pat >= beta {
            return Some(stand_pat);
        }

        let original_alpha = alpha;
        alpha = alpha.max(stand_pat);
        let threshold = alpha - stand_pat - self.delta_margin;
        let buffers = &mut self.qsearch[ply as usize];
        buffers.reset(position);
        if !buffers.initialize(
            position,
            &self.generator,
            self.nnue,
            &self.capture_ranks,
            threshold,
        ) {
            return Some(stand_pat);
        }

        let key = search_key(position);
        let mut tt_move = None;
        #[cfg(feature = "search-stats")]
        {
            self.stats.quiesce_tt_probes += 1;
        }
        if let Some(hit) = self.tt.probe(key, ply) {
            #[cfg(feature = "search-stats")]
            {
                self.stats.quiesce_tt_hits += 1;
            }
            tt_move = hit.best_move;
            let cutoff = match hit.bound {
                Bound::Exact => true,
                Bound::Lower => hit.score >= beta,
                Bound::Upper => hit.score <= original_alpha,
            };
            if cutoff {
                #[cfg(feature = "search-stats")]
                {
                    self.stats.quiesce_tt_cutoffs += 1;
                }
                return Some(hit.score);
            }
        }

        let mut best = stand_pat;
        let mut best_move = None;
        let mut searched_captures = 0;
        self.qsearch[ply as usize].set_tt_move(position, &self.generator, tt_move);
        while let Some(candidate) = self.qsearch[ply as usize].next(
            position,
            &self.generator,
            self.nnue,
            &self.capture_ranks,
            self.capture_history,
        ) {
            let mv = candidate.capture.mv;
            let buffers = &self.qsearch[ply as usize];
            let is_last_royal_capture = captures_all_royals(
                buffers.royals,
                buffers.royal_count,
                candidate.capture.captured,
            );
            if !is_last_royal_capture
                && stand_pat + candidate.captured_value + self.delta_margin <= alpha
            {
                continue;
            }
            if !is_last_royal_capture
                && capture_is_pruned_by_see(position, self.rules, self.nnue, mv)
            {
                continue;
            }
            // 上限は実際に読んだ捕獲だけを数える。例外も手数に含める。
            if searched_captures >= params::qsearch_move_limit()
                && !is_last_royal_capture
                && !self.previous_capture[ply as usize]
                    .is_some_and(|square| candidate.capture.captured.contains(&Some(square)))
                && !(candidate.capture.piece.kind() == Some(PieceKind::Lion)
                    && candidate.capture.captured.iter().all(Option::is_some))
            {
                continue;
            }
            if !is_last_royal_capture {
                self.enter_node().then_some(())?;
            }
            searched_captures += 1;
            self.previous_capture[(ply + 1) as usize] = Some(mv.to);
            let score = if is_last_royal_capture {
                MATE - (ply + 1) as i32
            } else {
                let undo = position.make_move_with_captures_unchecked(
                    mv,
                    self.rules,
                    candidate.capture.captured,
                );
                let (parents, children) = self.accumulators.split_at_mut((ply + 1) as usize);
                self.nnue.update_accumulator_after_move(
                    &parents[ply as usize],
                    &mut children[0],
                    position,
                    &undo,
                );
                let score = self
                    .quiesce(position, -beta, -alpha, ply + 1)
                    .map(|value| -value);
                position.unmake_move(undo);
                score?
            };
            if score > best {
                best = score;
                best_move = Some(mv);
            }
            alpha = alpha.max(score);
            if alpha >= beta {
                break;
            }
        }
        let bound = if best >= beta {
            Bound::Lower
        } else if best <= original_alpha {
            Bound::Upper
        } else {
            Bound::Exact
        };
        self.tt.store(key, 0, best, bound, best_move, ply);
        Some(best)
    }
}

/// 静止探索の捕獲価値、履歴値、および生成順を復元する整列キー。
/// 各フィールドの範囲に合わせて生成元と連番を小さい整数で保持する。
#[derive(Clone, Copy)]
pub(super) struct QsearchCapture {
    pub(super) capture: CaptureCandidate,
    pub(super) captured_value: i32,
    /// 群の生成時に候補ごとに1回引く捕獲履歴の生値。整列中に表を引き直さない。
    /// 群の順序を使わず先に返す置換表の手では0とする。
    history: i32,
    attacker_value: i32,
    // 駒種の添字は0..29、升の生値は最大187（16×11+11）なのでu8に収まる。
    origin_key: (u8, u8),
    // 1駒につき通常到達升143個と2段階移動8×8個、成否2通りを上界に取ると、
    // 144×(143+64)×2 = 59,616候補なので連番はu16に収まる。
    seq: u16,
}

impl QsearchCapture {
    fn new(nnue: &Nnue, capture: CaptureCandidate, captured_value: i32, history: i32) -> Self {
        let piece = capture.piece;
        Self {
            captured_value,
            history,
            attacker_value: nnue.piece_value(piece),
            origin_key: (
                piece.kind().expect("capture origin has a kind").index() as u8,
                capture.mv.from.raw(),
            ),
            capture,
            seq: 0,
        }
    }
}

/// 捕獲価値の降順の順位。等しい値の駒状態は同じ順位を共有する。
pub(super) struct CaptureRanks {
    pub(super) rank_of_state: [u8; PIECE_STATE_COUNT],
    pub(super) ranks_of_kind: [[u8; 2]; PIECE_KIND_COUNT],
    pub(super) values: Vec<i32>,
}

impl CaptureRanks {
    pub(super) fn new(nnue: &Nnue) -> Self {
        let mut states: [_; PIECE_STATE_COUNT] = core::array::from_fn(|state| state);
        states.sort_unstable_by_key(|&state| Reverse(nnue.piece_value_of_state(state)));
        let mut ranks = Self {
            rank_of_state: [0; PIECE_STATE_COUNT],
            ranks_of_kind: [[0; 2]; PIECE_KIND_COUNT],
            values: Vec::with_capacity(PIECE_STATE_COUNT),
        };
        for state in states {
            let value = nnue.piece_value_of_state(state);
            if ranks.values.last() != Some(&value) {
                ranks.values.push(value);
            }
            ranks.rank_of_state[state] = (ranks.values.len() - 1) as u8;
        }
        for kind in PieceKind::ALL {
            let unpromoted = match PieceCode::new(minase_core::Color::Black, kind) {
                Some(piece) => piece_state_of(piece),
                None => kind.index(), // 成駒としてのみ存在する駒種。
            };
            let promoted = if kind.unpromoted().is_some() {
                kind.index()
            } else {
                unpromoted
            };
            ranks.ranks_of_kind[kind.index()] = [
                ranks.rank_of_state[unpromoted],
                ranks.rank_of_state[promoted],
            ];
        }
        ranks
    }
}

/// 静止探索の1深さ分の領域。対象升の配列は使用する順位だけを初期化する。
pub(super) struct QsearchBuffers {
    pub(super) validation: CaptureCache,
    pub(super) capturers: Vec<OrdinaryCapturer>,
    pub(super) special: Vec<QsearchCapture>,
    pub(super) group: Vec<QsearchCapture>,
    targets_by_rank: [Bitboard; PIECE_STATE_COUNT],
    pub(super) present_ranks: u64,
    royals: Bitboard,
    royal_count: u32,
    tt_move: Option<Move>,
    tt_pending: bool,
    special_cursor: usize,
    cursor: usize,
}

impl Default for QsearchBuffers {
    fn default() -> Self {
        Self {
            validation: CaptureCache::default(),
            capturers: Vec::new(),
            special: Vec::new(),
            group: Vec::new(),
            targets_by_rank: [Bitboard::EMPTY; PIECE_STATE_COUNT],
            present_ranks: 0,
            royals: Bitboard::EMPTY,
            royal_count: 0,
            tt_move: None,
            tt_pending: false,
            special_cursor: 0,
            cursor: 0,
        }
    }
}

impl QsearchBuffers {
    pub(super) fn reset(&mut self, position: &Position) {
        self.validation.clear();
        self.special.clear();
        self.group.clear();
        self.present_ranks = 0;
        self.royals = position.royal_pieces(position.side_to_move().opposite());
        self.royal_count = self.royals.popcount();
        self.capturers.clear();
        self.tt_move = None;
        self.tt_pending = false;
        self.special_cursor = 0;
        self.cursor = 0;
    }

    /// 初期化後に置換表の記録手を検証し、探索用の合法捕獲手なら先頭候補にする。
    pub(super) fn set_tt_move(
        &mut self,
        position: &Position,
        generator: &MoveGenerator,
        tt_move: Option<Move>,
    ) {
        self.tt_move =
            tt_move.filter(|&mv| generator.is_legal_capture(position, mv, &mut self.validation));
        self.tt_pending = self.tt_move.is_some();
    }

    /// 入口のdelta pruning後に残る対象と捕獲価値の群を決め、通常駒の利きを保存する。
    /// 合計捕獲価値が`threshold`を超える合法捕獲か、最後の王駒を取る合法捕獲がある場合だけ真を返す。
    /// 獅子が経由升で取り、移動元と異なる空升へ進む手は探索候補から除く。
    pub(super) fn initialize(
        &mut self,
        position: &Position,
        generator: &MoveGenerator,
        nnue: &Nnue,
        ranks: &CaptureRanks,
        threshold: i32,
    ) -> bool {
        let opponent = position.side_to_move().opposite();
        let royals = self.royals;
        let royal_count = self.royal_count;
        generator.generate_special_captures(position, &mut |capture| {
            if MoveGenerator::is_excluded_lion_capture(position, capture.mv) {
                return;
            }
            let captured_value = capture
                .captured
                .into_iter()
                .flatten()
                .map(|square| nnue.piece_value(piece_at_for_ordering(position, square)))
                .sum();
            if captured_value <= threshold
                && !captures_all_royals(royals, royal_count, capture.captured)
            {
                return;
            }
            let mut candidate = QsearchCapture::new(nnue, capture, captured_value, 0);
            candidate.seq = self.special.len() as u16;
            self.special.push(candidate);
        });
        self.special
            .sort_unstable_by_key(|c| (Reverse(c.captured_value), c.seq));
        let mut allowed = Bitboard::EMPTY;
        for kind in PieceKind::ALL {
            let pieces = position.pieces_of_kind(opponent, kind);
            let [unpromoted, promoted] = ranks.ranks_of_kind[kind.index()];
            if unpromoted == promoted {
                let targets = if ranks.values[unpromoted as usize] > threshold {
                    pieces
                } else {
                    pieces & royals
                };
                self.add_targets(targets, unpromoted, &mut allowed);
            } else {
                for square in pieces {
                    let rank = if piece_at_for_ordering(position, square).is_promoted() {
                        promoted
                    } else {
                        unpromoted
                    };
                    if ranks.values[rank as usize] > threshold || royals.contains(square) {
                        self.add_targets(Bitboard::from_squares([square]), rank, &mut allowed);
                    }
                }
            }
        }
        generator.collect_ordinary_capturers(position, allowed, &mut self.capturers);
        !self.special.is_empty()
            || self.capturers.iter().any(|capturer| {
                let mut present = false;
                generator.emit_ordinary_captures(position, &[*capturer], allowed, &mut |capture| {
                    let value = nnue.piece_value(piece_at_for_ordering(position, capture.mv.to));
                    present |= value > threshold
                        || captures_all_royals(royals, royal_count, capture.captured);
                });
                present
            })
    }

    /// 同じ捕獲価値の対象升を順位別の集合へ加え、存在する順位と全対象升の集合を更新する。
    fn add_targets(&mut self, targets: Bitboard, rank: u8, allowed: &mut Bitboard) {
        if targets.is_empty() {
            return;
        }
        let bit = 1_u64 << rank;
        if self.present_ranks & bit == 0 {
            self.targets_by_rank[rank as usize] = targets;
            self.present_ranks |= bit;
        } else {
            self.targets_by_rank[rank as usize] |= targets;
        }
        *allowed |= targets;
    }

    /// 通常捕獲と特殊捕獲の残りから最大の捕獲価値を選び、同価値の候補を1群にまとめる。
    ///
    /// 群内は捕獲履歴の生値の降順、攻撃駒の価値の昇順、全手生成での生成順に並べる。
    /// 履歴の表引きは群へ加える候補ごとに1回だけ行い、整列中は保存した値を使う。
    /// 通常駒と特殊駒が混在する場合は、生成元の駒種と升、各列の連番で生成順を復元する。
    fn generate_group(
        &mut self,
        position: &Position,
        generator: &MoveGenerator,
        nnue: &Nnue,
        ranks: &CaptureRanks,
        capture_history: &CaptureHistory,
    ) -> Option<()> {
        let rank = self.present_ranks.trailing_zeros() as usize;
        let ordinary_value = (self.present_ranks != 0).then(|| ranks.values[rank]);
        let special_value = self
            .special
            .get(self.special_cursor)
            .map(|c| c.captured_value);
        let value = ordinary_value.into_iter().chain(special_value).max()?;
        let targets = if ordinary_value == Some(value) {
            self.present_ranks &= !(1_u64 << rank);
            self.targets_by_rank[rank]
        } else {
            Bitboard::EMPTY
        };
        let special_start = self.special_cursor;
        while self
            .special
            .get(self.special_cursor)
            .is_some_and(|c| c.captured_value == value)
        {
            self.special_cursor += 1;
        }
        self.group.clear();
        self.cursor = 0;
        generator.emit_ordinary_captures(position, &self.capturers, targets, &mut |capture| {
            let history = capture_history.read(position, nnue, capture.mv);
            let mut candidate = QsearchCapture::new(nnue, capture, value, history);
            candidate.seq = self.group.len() as u16;
            self.group.push(candidate);
        });
        if special_start == self.special_cursor {
            self.group
                .sort_unstable_by_key(|c| (Reverse(c.history), c.attacker_value, c.seq));
        } else {
            self.group
                .extend(self.special[special_start..self.special_cursor].iter().map(
                    |&candidate| QsearchCapture {
                        history: capture_history.read(position, nnue, candidate.capture.mv),
                        ..candidate
                    },
                ));
            // 駒種と移動元は通常駒・特殊駒で重ならない。各列の連番と合わせると、
            // 公開生成順で合流してから安定整列した順序に一致する。
            self.group.sort_unstable_by_key(|c| {
                (Reverse(c.history), c.attacker_value, c.origin_key, c.seq)
            });
        }
        Some(())
    }

    /// 置換表の捕獲を先頭に返し、要求されたグループまでだけを生成する。
    pub(super) fn next(
        &mut self,
        position: &Position,
        generator: &MoveGenerator,
        nnue: &Nnue,
        ranks: &CaptureRanks,
        capture_history: &CaptureHistory,
    ) -> Option<QsearchCapture> {
        if self.tt_pending {
            self.tt_pending = false;
            let mv = self.tt_move.expect("pending TT capture exists");
            let captured = position.captured_squares(mv);
            let captured_value = captured
                .into_iter()
                .flatten()
                .map(|s| nnue.piece_value(piece_at_for_ordering(position, s)))
                .sum();
            return Some(QsearchCapture::new(
                nnue,
                CaptureCandidate {
                    mv,
                    piece: piece_at_for_ordering(position, mv.from),
                    captured,
                },
                captured_value,
                0,
            ));
        }
        loop {
            while let Some(&candidate) = self.group.get(self.cursor) {
                self.cursor += 1;
                if Some(candidate.capture.mv) != self.tt_move {
                    return Some(candidate);
                }
            }
            self.generate_group(position, generator, nnue, ranks, capture_history)?;
        }
    }
}
