//! 局面と成り権保留状態のzobristハッシュ。

use super::Position;
use super::lion_trigger::LionTrigger;
use crate::core::board::{BOARD_SQUARE_COUNT, Square};
use crate::core::piece::{COLOR_COUNT, Color, PIECE_KIND_COUNT, PieceCode};
use crate::rng::XorShift64;
use core::hash::{Hash, Hasher};
use core::num::NonZeroU64;
use std::sync::OnceLock;

/// 1升あたりのzobrist駒キー数(色×駒種×成否)。
const ZOBRIST_PIECE_CODE_COUNT: usize = COLOR_COUNT * PIECE_KIND_COUNT * 2;
/// zobristキー生成に使う乱数列のシード。
const ZOBRIST_SEED: NonZeroU64 = NonZeroU64::new(0x4d49_4e41_5345_5a31).unwrap();

/// zobristハッシュの基底乱数表。
pub(super) struct ZobristKeys {
    /// 升×駒コードごとのキー。
    pieces: Box<[u64]>,
    /// 手番が後手のとき加えるキー。
    pub(super) side_to_move: u64,
    /// 先獅子トリガーがあるとき加えるキー。
    lion_trigger: u64,
    /// 先獅子トリガーが麒麟成りによるとき加えるキー。
    lion_trigger_kirin: u64,
    /// 升ごとの成り権保留キー。
    promotion_deferred: Box<[u64]>,
    /// 先獅子トリガーの対象升ごとのキー。
    lion_trigger_square: Box<[u64]>,
}

impl ZobristKeys {
    /// 乱数表を構築する。
    fn build() -> Self {
        let mut rng = XorShift64::new(ZOBRIST_SEED);
        let pieces = (0..BOARD_SQUARE_COUNT * ZOBRIST_PIECE_CODE_COUNT)
            .map(|_| rng.next())
            .collect();
        let side_to_move = rng.next();
        let lion_trigger = rng.next();
        let lion_trigger_kirin = rng.next();
        let promotion_deferred = (0..BOARD_SQUARE_COUNT).map(|_| rng.next()).collect();
        let lion_trigger_square = (0..BOARD_SQUARE_COUNT).map(|_| rng.next()).collect();
        Self {
            pieces,
            side_to_move,
            lion_trigger,
            lion_trigger_kirin,
            promotion_deferred,
            lion_trigger_square,
        }
    }

    /// 指定した升・駒のキーを返す。
    #[inline]
    pub(super) fn piece(&self, square: Square, piece: PieceCode) -> u64 {
        let color = piece.color().expect("piece key requires a colored piece");
        let kind = piece.kind().expect("piece key requires a valid piece kind");
        let piece_code = (color.index() * PIECE_KIND_COUNT + kind.index()) * 2
            + usize::from(piece.is_promoted());
        self.pieces[square.dense_index() * ZOBRIST_PIECE_CODE_COUNT + piece_code]
    }

    /// 手番のキーを返す。先手番は0とする。
    #[inline]
    pub(super) fn side(&self, side_to_move: Color) -> u64 {
        match side_to_move {
            Color::Black => 0,
            Color::White => self.side_to_move,
        }
    }

    /// 先獅子トリガー状態の寄与を返す。
    #[inline]
    pub(super) fn lion_trigger_state(&self, trigger: Option<LionTrigger>) -> u64 {
        trigger.map_or(0, |trigger| {
            self.lion_trigger
                ^ self.lion_trigger_square[trigger.square.dense_index()]
                ^ if trigger.by_kirin_promotion {
                    self.lion_trigger_kirin
                } else {
                    0
                }
        })
    }

    /// 指定升の成り権保留キーを返す。
    #[inline]
    pub(super) fn promotion_deferred(&self, square: Square) -> u64 {
        self.promotion_deferred[square.dense_index()]
    }
}

static ZOBRIST_KEYS: OnceLock<ZobristKeys> = OnceLock::new();

/// プロセス全体で共有する乱数表を返す。初回呼び出し時に構築する。
pub(super) fn zobrist_keys() -> &'static ZobristKeys {
    ZOBRIST_KEYS.get_or_init(ZobristKeys::build)
}

/// 通常Zobrist値と成り権保留Zobrist値をハッシュに用いる。衝突時の同一性は`Eq`が
/// 確定し、反復判定や探索キーの契約とは無関係である
/// (設計書predecessor-generator.md「Positionのハッシュ」)。
impl Hash for Position {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.zobrist.hash(state);
        self.rights_zobrist.hash(state);
    }
}

impl Position {
    /// 盤面・手番・先獅子トリガーを含むzobristハッシュを返す。
    ///
    /// 先獅子トリガーの有無、対象升および麒麟成りフラグを区別する。
    /// これらは、採用可能な全ローカルルール(L1・L2を含む)の下で
    /// 次の合法手を区別する局面要素である(第24条第1項c)。
    /// この値および[`Self::rights_zobrist`]の衝突は実用上無視できるものとし、
    /// 反復規則による対局裁定でも完全な局面署名との照合は行わない。
    #[inline]
    pub const fn zobrist(&self) -> u64 {
        self.zobrist
    }

    /// 成り権保留状態(P1・P2・P5)のzobristハッシュを返す。
    #[inline]
    pub const fn rights_zobrist(&self) -> u64 {
        self.rights_zobrist
    }

    /// 盤面全体からzobristハッシュを計算し直して返す。増分更新の検証に使う。
    pub(crate) fn recompute_zobrist(&self) -> u64 {
        let keys = zobrist_keys();
        let hash = Square::all()
            .filter_map(|square| self.piece_at(square).map(|piece| keys.piece(square, piece)))
            .fold(keys.side(self.side_to_move), |hash, key| hash ^ key);
        hash ^ keys.lion_trigger_state(self.lion_taken_by_non_lion)
    }

    /// 成り権保留状態(P1・P2・P5)からzobristハッシュを計算し直して返す。
    pub(crate) fn recompute_rights_zobrist(&self) -> u64 {
        let keys = zobrist_keys();
        self.promotion_deferred
            .into_iter()
            .map(|square| keys.promotion_deferred(square))
            .fold(0, |hash, key| hash ^ key)
    }
}
