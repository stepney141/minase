//! 静止探索と捕獲の段階生成。

use core::cmp::Reverse;

use crate::eval::Nnue;
use crate::eval::{PIECE_STATE_COUNT, piece_state_of};
use crate::search::snapshot::{SearchRepetition, search_key};
use crate::search::{MATE, MAX_PLY, in_repetition_band};
use minase_core::MoveGenerator;
use minase_core::board::Bitboard;
use minase_core::movegen::{CaptureCache, CaptureCandidate, OrdinaryCapturer};
use minase_core::mv::Move;
use minase_core::piece::{PIECE_KIND_COUNT, PieceCode, PieceKind};
use minase_core::position::Position;
use minase_core::repetition_key;

use super::capture_history::CaptureHistory;
use super::correction::{
    CorrectionPieceKeys, correction_from_weighted_sum, correction_piece_keys_after_move,
    key_after_move,
};
#[cfg(feature = "invariants")]
use super::correction::{correction_piece_keys, material_key};
#[cfg(feature = "invariants")]
use super::history::pawn_key;
use super::history::pawn_key_after_move;
use super::ordering::piece_at_for_ordering;
use super::params;
use super::pruning::{corrected_static_eval, pruning_eval, should_raise_futility_best_score};
use super::royal::captures_all_royals;
use super::searcher::{MoveRecord, PathMove, Searcher};
use super::see::see_prunes;
use super::tt::Bound;

impl Searcher<'_> {
    /// 補正後の静的評価をstand-patとし、捕獲手だけを使う静止探索で局面を評価する。
    ///
    /// 王駒がすべて狙われていてもstand-patを使い、非捕獲の王手と成りは展開しない。
    /// この近似の理由は`docs/adr/use-stand-pat-when-royals-are-attacked.md`にある。
    /// 捕獲ごとに盤上の駒が減って同一局面へ戻らないため、反復は検出しない。
    /// 捕獲を指して子を読む間は、補正の4つの鍵と探索経路の着手記録を通常探索と同じ契約で更新する。
    /// 静止探索内の手は主変化へ含めず、中断された場合は`None`を返す。
    ///
    /// 最大plyでは補正前の静的評価を返す。その後、静的評価を求める前に置換表を照会し、
    /// 窓の幅が1なら、正確な値、β以上の下界、またはα以下の上界で打ち切る
    /// （Stockfish src/search.cpp:1734-1746）。専用の帯の値は対局履歴と探索経路に依存するため
    /// 打ち切りには使わず、詰み帯の値は使う。窓の幅が1でなければ、同じ条件を満たしても打ち切らない。
    ///
    /// 照合した記録に補正前の静的評価があれば評価関数の値に代えて使い、現在の補正を加える。
    /// 置換表の探索値が通常域にあり、正確な値であるか、下界でstand-patより大きいか、
    /// 上界でstand-pat以下なら、その値を最善値の初期値にする（Stockfish src/search.cpp:1756-1770）。
    /// この初期値がβ以上なら返し、照会に当たっていなければ補正前の静的評価だけを持つ記録を書く
    /// （Stockfish src/search.cpp:1785-1787）。delta pruningとSEEの閾値の基準は、
    /// 置換表の値へ置き換える前のstand-patから求める（Stockfish src/search.cpp:1794）。
    ///
    /// 置換表ヒットによる打ち切りと中断は保存しない。通常出口は補正前の静的評価と探索値を
    /// 深さ0で保存し、最善値がβ以上なら下界、それ以外は上界とする。正確な値は保存しない
    /// （Stockfish src/search.cpp:1908-1909）。補正後のstand-patは静的評価として保存しない。
    /// stand-patを上回る捕獲手がなければ記録手は持たない。
    /// 深さ0の記録は通常探索の深さ条件を満たさず、カットオフに使われない。
    /// 保存の理由は`docs/adr/store-quiescence-results-at-depth-zero.md`にある。
    ///
    /// 捕獲候補は最後の王駒の捕獲を先に判定し、それなら枝刈りせず`MATE - (ply + 1)`を使う。
    /// それ以外は、取り返しと成る手なら手数制限、[`Self::delta_margin`]のdelta pruning、
    /// 補正後のstand-patと余裕値を基準にしたSEEの閾値を適用しない。
    /// どちらでもない候補は、この3つの条件に当たったときだけ王手になるかを判定する。
    /// 王手になる手も3つの枝刈りを適用せず、同じ候補では王手を高々1回判定する。
    /// 最後に、すべての候補へ[`Self::qsearch_see_floor`]のSEEの下限を適用する。
    /// delta pruningで読まない手はstand-pat、余裕値、および捕獲価値の和で最善値を引き上げる。
    /// SEEの閾値で読まない手は、現在のαとstand-patに余裕値を足した値の小さい方で引き上げる。
    /// どちらも現在の最善値と見積りが通常域にある場合に限り、αと最善手は変えない
    /// （Stockfish src/search.cpp:1834, 1841）。
    /// 手数制限とSEEの下限で読まない手は最善値を引き上げない。
    /// 置換表の捕獲手はSEEの例外にしない。
    ///
    /// 手数制限は、候補の列の`next`が返した合法で重複しない候補について、現在の候補より前に返した候補の数`n`を
    /// 0から数え、`n >= QsearchMoveLimit`なら候補を読まない（Stockfish src/search.cpp:1816, 1825）。
    /// 置換表の手、最後の王駒を取る手、免除と例外の手、および各枝刈りで読まない手も候補に数える。
    /// 取り返しは、候補が取る駒の升の集合に直前の手の到達升を含むかで判定する。
    /// 取り返し、成る手、および王手になる手は3つの枝刈りを免れ、獅子が2枚を取る手は手数制限だけを免れる。
    /// 角鷹と飛鷲の2枚取りは、獅子の例外に含めない。
    /// 2段階移動も1候補と数え、最後の王駒を取る手は候補に数えても探索ノードには数えない。
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

        #[cfg(feature = "invariants")]
        {
            assert_eq!(self.path_moves.len(), ply as usize);
            assert_eq!(self.path_moves.len() + 1, self.path_keys.len());
            self.nnue
                .assert_accumulator(position, &self.accumulators[ply as usize], ply);
            assert_eq!(self.material_keys[ply as usize], material_key(position));
            assert_eq!(self.pawn_keys[ply as usize], pawn_key(position));
            let correction_keys = correction_piece_keys(position);
            assert_eq!(self.minor_keys[ply as usize], correction_keys.minor);
            assert_eq!(self.non_pawn_keys[ply as usize], correction_keys.non_pawn);
        }

        if ply >= MAX_PLY {
            return Some(
                self.nnue.evaluate_accumulator(
                    &self.accumulators[ply as usize],
                    position.side_to_move(),
                ),
            );
        }

        let key = search_key(position);
        #[cfg(feature = "search-stats")]
        {
            self.stats.quiesce_tt_probes += 1;
        }
        let tt_hit = self.tt.probe(key, ply);
        // Stockfish src/search.cpp:1741, 1908。静止探索は照合した印だけを引き継ぐ。
        let pv_hit = tt_hit.is_some_and(|hit| hit.is_pv);
        let mut tt_move = None;
        if let Some(hit) = tt_hit {
            #[cfg(feature = "search-stats")]
            {
                self.stats.quiesce_tt_hits += 1;
            }
            tt_move = hit.best_move;
            if let Some(value) = hit.value {
                let cutoff = !in_repetition_band(value.score)
                    && match value.bound {
                        Bound::Exact => true,
                        Bound::Lower => value.score >= beta,
                        Bound::Upper => value.score <= alpha,
                    };
                if cutoff {
                    if beta == alpha + 1 {
                        #[cfg(feature = "search-stats")]
                        {
                            self.stats.quiesce_tt_cutoffs += 1;
                        }
                        return Some(value.score);
                    }
                    #[cfg(feature = "search-stats")]
                    {
                        self.stats.quiesce_tt_wide_window_cutoff_skips += 1;
                    }
                }
            }
        }

        let side = position.side_to_move();
        let v = if let Some(static_eval) = tt_hit.and_then(|hit| hit.static_eval) {
            #[cfg(feature = "search-stats")]
            {
                self.stats.quiesce_tt_static_eval_uses += 1;
            }
            static_eval
        } else {
            self.nnue
                .evaluate_accumulator(&self.accumulators[ply as usize], side)
        };
        let s = corrected_static_eval(
            v,
            correction_from_weighted_sum(
                self.correction_weighted_sum_at(side, ply),
                self.nnue.pawn_value(),
            ),
        );
        let best = pruning_eval(
            s,
            tt_hit
                .and_then(|hit| hit.value)
                .map(|value| (value.score, value.bound)),
        );
        #[cfg(feature = "search-stats")]
        if best != s {
            self.stats.quiesce_tt_value_replacements += 1;
        }
        if best >= beta {
            if tt_hit.is_none() && self.tt.store_static_eval(key, v, false) {
                #[cfg(feature = "search-stats")]
                {
                    self.stats.quiesce_static_eval_stores += 1;
                }
            }
            return Some(best);
        }

        alpha = alpha.max(best);
        let buffers = &mut self.qsearch[ply as usize];
        buffers.reset(position);
        buffers.initialize(position, &self.generator, self.nnue, &self.capture_ranks);
        let mut best = best;
        let mut best_move = None;
        let mut candidate_count = 0;
        let futility_base = s + self.delta_margin;
        #[cfg(feature = "search-stats")]
        let low_value_threshold = alpha - futility_base;
        #[cfg(feature = "search-stats")]
        let mut returned_low_value_group = false;
        self.qsearch[ply as usize].set_tt_move(position, &self.generator, tt_move);
        'candidates: while let Some(candidate) = self.qsearch[ply as usize].next(
            position,
            &self.generator,
            self.nnue,
            &self.capture_ranks,
            self.capture_history,
        ) {
            let candidate_index = candidate_count;
            candidate_count += 1;
            let mv = candidate.capture.mv;
            #[cfg(feature = "search-stats")]
            if !returned_low_value_group
                && Some(mv) != tt_move
                && candidate.captured_value <= low_value_threshold
            {
                self.stats.quiesce_low_value_group_nodes += 1;
                returned_low_value_group = true;
            }
            let buffers = &self.qsearch[ply as usize];
            let is_last_royal_capture = captures_all_royals(
                buffers.royals,
                buffers.royal_count,
                candidate.capture.captured,
            );
            let score = if is_last_royal_capture {
                MATE - (ply + 1) as i32
            } else {
                let futility_value = futility_base + candidate.captured_value;
                let is_recapture = self.previous_move_to[ply as usize]
                    .is_some_and(|square| candidate.capture.captured.contains(&Some(square)));
                let is_double_lion = candidate.capture.piece.kind() == Some(PieceKind::Lion)
                    && candidate.capture.captured.iter().all(Option::is_some);
                'pruning: {
                    if is_recapture || mv.promote {
                        #[cfg(feature = "search-stats")]
                        {
                            let would_prune = (candidate_index
                                >= params::qsearch_move_limit() as usize
                                && !is_double_lion)
                                || futility_value <= alpha
                                || see_prunes(
                                    position,
                                    self.rules,
                                    self.nnue,
                                    mv,
                                    alpha - futility_base,
                                );
                            if would_prune {
                                if is_recapture {
                                    self.stats.quiesce_recapture_exemptions += 1;
                                } else {
                                    self.stats.quiesce_promotion_exemptions += 1;
                                }
                            }
                        }
                        break 'pruning;
                    }

                    if candidate_index >= params::qsearch_move_limit() as usize {
                        if is_double_lion {
                            #[cfg(feature = "search-stats")]
                            {
                                self.stats.quiesce_lion_double_move_count_exemptions += 1;
                            }
                        } else {
                            if self.checking_move_exempts_qsearch_pruning(position, mv) {
                                break 'pruning;
                            }
                            #[cfg(feature = "search-stats")]
                            {
                                self.stats.quiesce_move_count_prunes += 1;
                            }
                            continue 'candidates;
                        }
                    }

                    if futility_value <= alpha {
                        if self.checking_move_exempts_qsearch_pruning(position, mv) {
                            break 'pruning;
                        }
                        if should_raise_futility_best_score(best, futility_value) {
                            best = futility_value;
                            #[cfg(feature = "search-stats")]
                            {
                                self.stats.quiesce_delta_best_score_raises += 1;
                            }
                        }
                        continue 'candidates;
                    }

                    if see_prunes(position, self.rules, self.nnue, mv, alpha - futility_base) {
                        if self.checking_move_exempts_qsearch_pruning(position, mv) {
                            break 'pruning;
                        }
                        #[cfg(feature = "search-stats")]
                        {
                            self.stats.quiesce_see_threshold_prunes += 1;
                        }
                        let estimate = alpha.min(futility_base);
                        if should_raise_futility_best_score(best, estimate) {
                            best = estimate;
                            #[cfg(feature = "search-stats")]
                            {
                                self.stats.quiesce_see_threshold_best_score_raises += 1;
                            }
                        }
                        continue 'candidates;
                    }
                }
                if see_prunes(position, self.rules, self.nnue, mv, self.qsearch_see_floor) {
                    #[cfg(feature = "search-stats")]
                    {
                        self.stats.quiesce_see_floor_prunes += 1;
                    }
                    continue;
                }
                self.enter_node().then_some(())?;
                self.previous_move_to[(ply + 1) as usize] = Some(mv.to);
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
                self.material_keys[(ply + 1) as usize] =
                    key_after_move(self.material_keys[ply as usize], &undo);
                self.pawn_keys[(ply + 1) as usize] =
                    pawn_key_after_move(self.pawn_keys[ply as usize], &undo);
                let correction_keys = correction_piece_keys_after_move(
                    CorrectionPieceKeys {
                        minor: self.minor_keys[ply as usize],
                        non_pawn: self.non_pawn_keys[ply as usize],
                    },
                    &undo,
                );
                self.minor_keys[(ply + 1) as usize] = correction_keys.minor;
                self.non_pawn_keys[(ply + 1) as usize] = correction_keys.non_pawn;
                let path_key = match self.repetition {
                    SearchRepetition::R1 { .. } => search_key(position),
                    SearchRepetition::R2 | SearchRepetition::R3 => {
                        repetition_key(self.repetition.rule(), position)
                    }
                };
                self.path_keys.push(path_key);
                self.path_moves.push(PathMove {
                    record: Some(MoveRecord::from_undo(self.nnue, &undo)),
                    attacking: None,
                });
                let score = self
                    .quiesce(position, -beta, -alpha, ply + 1)
                    .map(|value| -value);
                self.path_keys.pop();
                self.path_moves.pop();
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
        } else {
            Bound::Upper
        };
        self.tt
            .store(key, 0, best, Some(v), bound, best_move, ply, pv_hit);
        Some(best)
    }

    /// 枝刈り条件に当たった候補が王手になる手なら、免除として記録する。
    fn checking_move_exempts_qsearch_pruning(&mut self, position: &Position, mv: Move) -> bool {
        #[cfg(feature = "search-stats")]
        {
            self.stats.quiesce_gives_check_calls += 1;
        }
        let gives_check = position.gives_check(&self.generator, mv);
        #[cfg(feature = "search-stats")]
        if gives_check {
            self.stats.quiesce_check_exemptions += 1;
        }
        gives_check
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

/// 静止探索の1深さ分の領域。
/// すべての対象升を捕獲価値の順位に登録し、候補の列が達した群だけを生成する。
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

    /// 特殊捕獲を生成し、すべての相手駒を捕獲価値の群へ登録して通常駒の利きを保存する。
    /// 合法な捕獲の有無は調べず、群の候補は[`Self::next`]が必要になったときに生成する。
    /// 獅子が経由升で取り、移動元と異なる空升へ進む手は探索候補から除く。
    pub(super) fn initialize(
        &mut self,
        position: &Position,
        generator: &MoveGenerator,
        nnue: &Nnue,
        ranks: &CaptureRanks,
    ) {
        let opponent = position.side_to_move().opposite();
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
                self.add_targets(pieces, unpromoted, &mut allowed);
            } else {
                for square in pieces {
                    let rank = if piece_at_for_ordering(position, square).is_promoted() {
                        promoted
                    } else {
                        unpromoted
                    };
                    self.add_targets(Bitboard::from_squares([square]), rank, &mut allowed);
                }
            }
        }
        generator.collect_ordinary_capturers(position, allowed, &mut self.capturers);
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

    /// 通常捕獲と特殊捕獲の残りから最大の捕獲価値を選び、同価値の候補を1群だけ生成する。
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

    /// 置換表の捕獲を先頭に返し、その後は捕獲価値の降順に必要な群だけを生成する。
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
