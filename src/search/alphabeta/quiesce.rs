//! 静止探索と捕獲の段階生成。

use core::cmp::Reverse;

use crate::MoveGenerator;
use crate::core::bitboard::Bitboard;
use crate::core::movegen::{CaptureCache, CaptureCandidate, OrdinaryCapturer};
use crate::core::mv::Move;
use crate::core::piece::{PIECE_KIND_COUNT, PieceCode, PieceKind};
use crate::core::position::Position;
use crate::eval::Pst;
use crate::eval::pst::{PIECE_STATE_COUNT, piece_state_of};
use crate::search::snapshot::search_key;
use crate::search::{MATE, MAX_PLY};

use super::ordering::piece_at_for_ordering;
use super::pruning::capture_is_pruned_by_see;
use super::royal::captures_all_royals;
use super::searcher::Searcher;
use super::tt::Bound;

impl Searcher<'_> {
    /// stand-patと捕獲手だけを使う静止探索で局面を評価する。
    ///
    /// 設計書movegen-speedup-2.md「段階7」に従い、静的評価で打ち切るときは置換表に触れない。
    /// 静止探索内の手は主変化へ含めず、反復検出は行わない。
    /// 中断された場合は`None`を返す。
    pub(super) fn quiesce(
        &mut self,
        position: &mut Position,
        mut alpha: i32,
        beta: i32,
        ply: u32,
    ) -> Option<i32> {
        self.pv[ply as usize].clear();

        if ply >= MAX_PLY {
            return Some(
                self.pst
                    .evaluate_accumulator(self.accumulators[ply as usize], position.side_to_move()),
            );
        }

        let stand_pat = self
            .pst
            .evaluate_accumulator(self.accumulators[ply as usize], position.side_to_move());
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
            self.pst,
            &self.capture_ranks,
            threshold,
        ) {
            return Some(stand_pat);
        }

        let key = search_key(position);
        let mut tt_move = None;
        if let Some(hit) = self.tt.probe(key, ply) {
            tt_move = hit.best_move;
            let cutoff = match hit.bound {
                Bound::Exact => true,
                Bound::Lower => hit.score >= beta,
                Bound::Upper => hit.score <= original_alpha,
            };
            if cutoff {
                return Some(hit.score);
            }
        }

        let mut best = stand_pat;
        let mut best_move = None;
        self.qsearch[ply as usize].set_tt_move(position, &self.generator, tt_move);
        while let Some(candidate) = self.qsearch[ply as usize].next(
            position,
            &self.generator,
            self.pst,
            &self.capture_ranks,
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
                && capture_is_pruned_by_see(position, self.rules, self.pst, mv)
            {
                continue;
            }
            let score = if is_last_royal_capture {
                MATE - (ply + 1) as i32
            } else {
                self.enter_node().then_some(())?;
                let undo = position.make_move_with_captures_unchecked(
                    mv,
                    self.rules,
                    candidate.capture.captured,
                );
                self.accumulators[(ply + 1) as usize] = self.pst.update_accumulator_after_move(
                    self.accumulators[ply as usize],
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

/// 静止探索の捕獲価値と、同順位で生成順を保つ連番。
/// 設計書movegen-speedup-2.md「段階5」に従い、順序キーを圧縮する。
#[derive(Clone, Copy)]
pub(super) struct QsearchCapture {
    pub(super) capture: CaptureCandidate,
    pub(super) captured_value: i32,
    attacker_value: i32,
    // 駒種の添字は0..29、升の生値は最大187（16×11+11）なのでu8に収まる。
    origin_key: (u8, u8),
    // 1駒につき通常到達升143個と2段階移動8×8個、成否2通りを上界に取ると、
    // 144×(143+64)×2 = 59,616候補なので連番はu16に収まる。
    seq: u16,
}

impl QsearchCapture {
    fn new(pst: &Pst, capture: CaptureCandidate, captured_value: i32) -> Self {
        let piece = capture.piece;
        Self {
            captured_value,
            attacker_value: pst.piece_value(piece),
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
    pub(super) fn new(pst: &Pst) -> Self {
        let mut states: [_; PIECE_STATE_COUNT] = core::array::from_fn(|state| state);
        states.sort_unstable_by_key(|&state| Reverse(pst.piece_value_of_state(state)));
        let mut ranks = Self {
            rank_of_state: [0; PIECE_STATE_COUNT],
            ranks_of_kind: [[0; 2]; PIECE_KIND_COUNT],
            values: Vec::with_capacity(PIECE_STATE_COUNT),
        };
        for state in states {
            let value = pst.piece_value_of_state(state);
            if ranks.values.last() != Some(&value) {
                ranks.values.push(value);
            }
            ranks.rank_of_state[state] = (ranks.values.len() - 1) as u8;
        }
        for kind in PieceKind::ALL {
            let unpromoted = match PieceCode::new(crate::Color::Black, kind) {
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

    /// 設計書movegen-speedup-2.md「段階9」に従い、初期化後に置換表の手を設定する。
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

    /// 入口で残す対象と価値グループを決め、通常駒の利きを保存する。
    /// 設計書movegen-speedup-2.md「段階9」に従い、入口の枝刈り後に合法な候補があるかを返す。
    pub(super) fn initialize(
        &mut self,
        position: &Position,
        generator: &MoveGenerator,
        pst: &Pst,
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
                .map(|square| pst.piece_value(piece_at_for_ordering(position, square)))
                .sum();
            if captured_value <= threshold
                && !captures_all_royals(royals, royal_count, capture.captured)
            {
                return;
            }
            let mut candidate = QsearchCapture::new(pst, capture, captured_value);
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
                    let value = pst.piece_value(piece_at_for_ordering(position, capture.mv.to));
                    present |= value > threshold
                        || captures_all_royals(royals, royal_count, capture.captured);
                });
                present
            })
    }

    /// 設計書movegen-speedup-2.md「段階5」に従い、同価値の対象升を集合のまま登録する。
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

    /// 価値順の2列から次の値を選び、その通常捕獲と特殊捕獲を生成順でマージする。
    fn generate_group(
        &mut self,
        position: &Position,
        generator: &MoveGenerator,
        pst: &Pst,
        ranks: &CaptureRanks,
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
            let mut candidate = QsearchCapture::new(pst, capture, value);
            candidate.seq = self.group.len() as u16;
            self.group.push(candidate);
        });
        if special_start == self.special_cursor {
            self.group
                .sort_unstable_by_key(|c| (c.attacker_value, c.seq));
        } else {
            self.group
                .extend_from_slice(&self.special[special_start..self.special_cursor]);
            // 駒種と移動元は通常駒・特殊駒で重ならない。各列の連番と合わせると、
            // 公開生成順で合流してから安定整列した順序に一致する。
            self.group
                .sort_unstable_by_key(|c| (c.attacker_value, c.origin_key, c.seq));
        }
        Some(())
    }

    /// 置換表の捕獲を先頭に返し、要求されたグループまでだけを生成する。
    pub(super) fn next(
        &mut self,
        position: &Position,
        generator: &MoveGenerator,
        pst: &Pst,
        ranks: &CaptureRanks,
    ) -> Option<QsearchCapture> {
        if self.tt_pending {
            self.tt_pending = false;
            let mv = self.tt_move.expect("pending TT capture exists");
            let captured = position.captured_squares(mv);
            let captured_value = captured
                .into_iter()
                .flatten()
                .map(|s| pst.piece_value(piece_at_for_ordering(position, s)))
                .sum();
            return Some(QsearchCapture::new(
                pst,
                CaptureCandidate {
                    mv,
                    piece: piece_at_for_ordering(position, mv.from),
                    captured,
                },
                captured_value,
            ));
        }
        loop {
            while let Some(&candidate) = self.group.get(self.cursor) {
                self.cursor += 1;
                if Some(candidate.capture.mv) != self.tt_move {
                    return Some(candidate);
                }
            }
            self.generate_group(position, generator, pst, ranks)?;
        }
    }
}
