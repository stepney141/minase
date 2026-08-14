//! 局面の表現と、着手の適用・巻き戻し。

use core::fmt;
use std::sync::OnceLock;

use crate::core::bitboard::Bitboard;
use crate::core::mv::{CapturedPiece, Move, Undo};
use crate::core::piece::{COLOR_COUNT, Color, PIECE_KIND_COUNT, PieceCode, PieceKind};
use crate::core::rules::{PromotionChoice, RuleCode, Rules, in_promotion_zone};
use crate::core::square::{BOARD_FILES, BOARD_RANKS, BOARD_SQUARE_COUNT, RAW_SQUARE_COUNT, Square};
use crate::rng::XorShift64;

/// 1升あたりのzobrist駒キー数(色×駒種×成否)。
const ZOBRIST_PIECE_CODE_COUNT: usize = COLOR_COUNT * PIECE_KIND_COUNT * 2;
/// zobristキー生成に使う乱数列のシード。
const ZOBRIST_SEED: u64 = 0x4d49_4e41_5345_5a31;

/// zobristハッシュの基底乱数表。
struct ZobristKeys {
    /// 升×駒コードごとのキー。
    pieces: Box<[u64]>,
    /// 手番が後手のとき加えるキー。
    side_to_move: u64,
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
    fn piece(&self, square: Square, piece: PieceCode) -> u64 {
        let color = piece.color().expect("piece key requires a colored piece");
        let kind = piece.kind().expect("piece key requires a valid piece kind");
        let piece_code = (color.index() * PIECE_KIND_COUNT + kind.index()) * 2
            + usize::from(piece.is_promoted());
        self.pieces[square.dense_index() * ZOBRIST_PIECE_CODE_COUNT + piece_code]
    }

    /// 手番のキーを返す。先手番は0とする。
    #[inline]
    fn side(&self, side_to_move: Color) -> u64 {
        match side_to_move {
            Color::Black => 0,
            Color::White => self.side_to_move,
        }
    }

    /// 先獅子トリガー状態の寄与を返す。
    #[inline]
    fn lion_trigger_state(&self, trigger: Option<LionTrigger>) -> u64 {
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
    fn promotion_deferred(&self, square: Square) -> u64 {
        self.promotion_deferred[square.dense_index()]
    }
}

static ZOBRIST_KEYS: OnceLock<ZobristKeys> = OnceLock::new();

/// プロセス全体で共有する乱数表を返す。初回呼び出し時に構築する。
fn zobrist_keys() -> &'static ZobristKeys {
    ZOBRIST_KEYS.get_or_init(ZobristKeys::build)
}

/// 先獅子の発動原因となった直前の獅子捕獲。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LionTrigger {
    /// 獅子が取られた升。
    pub(crate) square: Square,
    /// 獅子を取った麒麟が同じ着手で成ったかどうか。
    pub(crate) by_kirin_promotion: bool,
}

/// 中将棋の局面。盤面・手番・次の合法手に影響する一時状態を保持する。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Position {
    /// 各升の駒コード。番兵込みの生インデックスで引く。
    board: [PieceCode; RAW_SQUARE_COUNT],
    /// 駒がある升の集合。
    occupied: Bitboard,
    /// 対局者別の駒の集合。
    by_color: [Bitboard; COLOR_COUNT],
    /// 対局者別・駒種別の駒の集合。
    by_kind: [[Bitboard; PIECE_KIND_COUNT]; COLOR_COUNT],
    /// 手番側。
    side_to_move: Color,
    /// 直前の着手で獅子以外の駒に取られた獅子の情報。先獅子(第15条)の判定に使う。
    lion_taken_by_non_lion: Option<LionTrigger>,
    /// 現局面のzobristハッシュ。着手のたびに増分更新する。
    zobrist: u64,
    /// P1で成り権を保留中の駒がある升の集合。
    promotion_deferred: Bitboard,
    /// P1成り権保留状態のzobristハッシュ。
    rights_zobrist: u64,
}

/// [`Position`]の操作または[`Position::validate`]が検出する不正な状態。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PositionError {
    /// 先獅子の獅子捕獲升に手番側の駒がある。
    InvalidLionCapture {
        /// 問題の獅子捕獲升。
        square: Square,
    },
    /// 番兵位置に番兵コード以外が置かれている。
    PaddingIsNotWall {
        /// 検出した生の駒コード値。
        raw: u8,
    },
    /// 有効升に番兵コードが置かれている。
    ValidSquareIsWall {
        /// 問題の升。
        square: Square,
    },
    /// 駒コードから色または駒種を復元できない。
    InvalidPieceCode {
        /// 問題の升。
        square: Square,
    },
    /// 盤面配列と占有集合が食い違っている。
    OccupancyMismatch {
        /// 問題の升。
        square: Square,
    },
    /// 両対局者の駒集合が重なっている。
    ColorOverlap,
    /// 対局者別集合の集計が合わない。
    ColorAggregateMismatch {
        /// 問題の対局者。
        color: Color,
    },
    /// 駒種別集合が盤面配列と合わない。
    KindMismatch {
        /// 問題の升。
        square: Square,
        /// 問題の対局者。
        color: Color,
        /// 問題の駒種。
        kind: PieceKind,
    },
    /// いずれかのビットボードで番兵ビットが立っている。
    PaddingBitSet,
    /// 成り権保留ビットの升に適格な駒がない。
    InvalidPromotionDeferred {
        /// 問題の升。
        square: Square,
    },
}

impl fmt::Display for PositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLionCapture { square } => write!(
                formatter,
                "invalid lion-capture square occupied by the side to move: {square:?}"
            ),
            _ => write!(formatter, "position invariant failed: {self:?}"),
        }
    }
}

impl std::error::Error for PositionError {}

/// [`PositionBuilder`]による局面構築のエラー。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PositionBuildError {
    /// 既に駒がある升へ配置しようとした。
    SquareOccupied {
        /// 問題の升。
        square: Square,
    },
    /// 空升または番兵のコードを駒として配置しようとした。
    EmptyOrWallPiece,
    /// 完成した局面が不変条件を満たさない。
    InvalidPosition(PositionError),
}

impl fmt::Display for PositionBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "position construction failed: {self:?}")
    }
}

impl std::error::Error for PositionBuildError {}

impl Position {
    /// 指定手番で駒のない局面を作る。
    pub fn empty(side_to_move: Color) -> Self {
        let mut board = [PieceCode::WALL; RAW_SQUARE_COUNT];
        for square in Square::all() {
            board[square.raw_index()] = PieceCode::EMPTY;
        }

        Self {
            board,
            occupied: Bitboard::EMPTY,
            by_color: [Bitboard::EMPTY; COLOR_COUNT],
            by_kind: [[Bitboard::EMPTY; PIECE_KIND_COUNT]; COLOR_COUNT],
            side_to_move,
            lion_taken_by_non_lion: None,
            zobrist: zobrist_keys().side(side_to_move),
            promotion_deferred: Bitboard::EMPTY,
            rights_zobrist: 0,
        }
    }

    /// 初期配置(第5条)の局面を作る。
    pub fn initial() -> Self {
        let mut builder = PositionBuilder::new(Color::Black);
        let back_rank = [
            PieceKind::Lance,
            PieceKind::FerociousLeopard,
            PieceKind::CopperGeneral,
            PieceKind::SilverGeneral,
            PieceKind::GoldGeneral,
            PieceKind::King,
            PieceKind::DrunkElephant,
            PieceKind::GoldGeneral,
            PieceKind::SilverGeneral,
            PieceKind::CopperGeneral,
            PieceKind::FerociousLeopard,
            PieceKind::Lance,
        ];
        let second_rank = [
            (0, PieceKind::ReverseChariot),
            (2, PieceKind::Bishop),
            (4, PieceKind::BlindTiger),
            (5, PieceKind::Kirin),
            (6, PieceKind::Phoenix),
            (7, PieceKind::BlindTiger),
            (9, PieceKind::Bishop),
            (11, PieceKind::ReverseChariot),
        ];
        let third_rank = [
            PieceKind::SideMover,
            PieceKind::VerticalMover,
            PieceKind::Rook,
            PieceKind::DragonHorse,
            PieceKind::DragonKing,
            PieceKind::Lion,
            PieceKind::FreeKing,
            PieceKind::DragonKing,
            PieceKind::DragonHorse,
            PieceKind::Rook,
            PieceKind::VerticalMover,
            PieceKind::SideMover,
        ];

        for color in Color::ALL {
            let rotate = |file: u8, rank: u8| match color {
                Color::Black => (file, rank),
                Color::White => (BOARD_FILES - 1 - file, BOARD_RANKS - 1 - rank),
            };
            let mut put = |file, rank, kind| {
                let (file, rank) = rotate(file, rank);
                builder
                    .put(
                        Square::new(file, rank).unwrap(),
                        PieceCode::new(color, kind),
                    )
                    .unwrap();
            };

            for (file, kind) in back_rank.into_iter().enumerate() {
                put(file as u8, 0, kind);
            }
            for (file, kind) in second_rank {
                put(file, 1, kind);
            }
            for (file, kind) in third_rank.into_iter().enumerate() {
                put(file as u8, 2, kind);
            }
            for file in 0..BOARD_FILES {
                put(file, 3, PieceKind::Pawn);
            }
            for file in [3, 8] {
                put(file, 4, PieceKind::GoBetween);
            }
        }

        builder.finish().unwrap()
    }

    /// 指定升の駒を返す。空升なら`None`を返す。
    #[inline]
    pub fn piece_at(&self, square: Square) -> Option<PieceCode> {
        let piece = self.board[square.raw_index()];
        (!piece.is_empty()).then_some(piece)
    }

    /// 駒がある升の集合を返す。
    #[inline]
    pub const fn occupied(&self) -> Bitboard {
        self.occupied
    }

    /// 指定した対局者の駒の集合を返す。
    #[inline]
    pub const fn pieces_of(&self, color: Color) -> Bitboard {
        self.by_color[color.index()]
    }

    /// 指定した対局者・駒種の駒の集合を返す。
    #[inline]
    pub const fn pieces_of_kind(&self, color: Color, kind: PieceKind) -> Bitboard {
        self.by_kind[color.index()][kind.index()]
    }

    /// 手番側を返す。
    #[inline]
    pub const fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    /// 拡張SFENの獅子捕獲升から先獅子状態を設定する。
    ///
    /// 空升は、角鷹または飛鷲が経由升で獅子を取った状態として受理する。
    /// 非手番側の麒麟由来の成獅子が指定升にあれば、同一升での取り返しに
    /// 必要な原因も復元する(第15条・第29条L2)。
    ///
    /// # エラー
    ///
    /// 指定升に手番側の駒がある場合は
    /// [`PositionError::InvalidLionCapture`]を返し、局面を変更しない。
    pub fn set_lion_capture(&mut self, lion_capture: Option<Square>) -> Result<(), PositionError> {
        let trigger = match lion_capture {
            None => None,
            Some(square) => {
                let piece = self.piece_at(square);
                if piece.is_some_and(|piece| piece.color() == Some(self.side_to_move)) {
                    return Err(PositionError::InvalidLionCapture { square });
                }
                Some(LionTrigger {
                    square,
                    by_kirin_promotion: piece.is_some_and(|piece| {
                        piece.color() == Some(self.side_to_move.opposite())
                            && piece.kind() == Some(PieceKind::Lion)
                            && piece.is_promoted()
                    }),
                })
            }
        };

        let keys = zobrist_keys();
        self.zobrist ^=
            keys.lion_trigger_state(self.lion_taken_by_non_lion) ^ keys.lion_trigger_state(trigger);
        self.lion_taken_by_non_lion = trigger;
        Ok(())
    }

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

    /// P1成り権保留状態のzobristハッシュを返す。
    #[inline]
    pub const fn rights_zobrist(&self) -> u64 {
        self.rights_zobrist
    }

    /// P1で成り権を保留中の駒がある升の集合を返す。
    #[inline]
    pub(crate) const fn promotion_deferred(&self) -> Bitboard {
        self.promotion_deferred
    }

    /// 王駒(王将・玉将・太子、第3条)の集合を返す。
    pub fn royal_pieces(&self, color: Color) -> Bitboard {
        self.pieces_of_kind(color, PieceKind::King)
            | self.pieces_of_kind(color, PieceKind::CrownPrince)
    }

    /// 直前の着手で獅子以外の駒に取られた獅子の情報を返す。先獅子(第15条)の判定に使う。
    pub(crate) const fn lion_taken_by_non_lion(&self) -> Option<LionTrigger> {
        self.lion_taken_by_non_lion
    }

    /// 捕獲升を最大2升返す。
    ///
    /// 捕獲升とは、着手で実際に相手駒を取る升をいう。獅子、角鷹および
    /// 飛鷲の2段階移動では、1手で最大2升の相手駒を取る(第11条第3項・
    /// 第12条第4項)。
    pub fn captured_squares(&self, mv: Move) -> [Option<Square>; 2] {
        let moving_color = self
            .piece_at(mv.from)
            .and_then(PieceCode::color)
            .expect("move origin must contain a piece");
        mv.capture_candidates().map(|candidate| {
            candidate.filter(|&square| {
                self.piece_at(square)
                    .is_some_and(|piece| piece.color() == Some(moving_color.opposite()))
            })
        })
    }

    /// 駒を置き、占有集合とzobristハッシュを増分更新する。
    fn put_piece(&mut self, square: Square, piece: PieceCode) -> Result<(), PositionBuildError> {
        if piece.is_empty() || piece.is_wall() {
            return Err(PositionBuildError::EmptyOrWallPiece);
        }
        if self.piece_at(square).is_some() {
            return Err(PositionBuildError::SquareOccupied { square });
        }

        let color = piece.color().expect("validated piece must have a color");
        let kind = piece.kind().expect("validated piece must have a kind");
        self.board[square.raw_index()] = piece;
        self.occupied.set(square);
        self.by_color[color.index()].set(square);
        self.by_kind[color.index()][kind.index()].set(square);
        self.zobrist ^= zobrist_keys().piece(square, piece);
        Ok(())
    }

    /// 駒を取り除き、占有集合とzobristハッシュを増分更新して駒を返す。
    fn remove_piece(&mut self, square: Square) -> PieceCode {
        let piece = self.board[square.raw_index()];
        debug_assert!(!piece.is_empty() && !piece.is_wall());
        let color = piece.color().expect("occupied square must have a color");
        let kind = piece.kind().expect("occupied square must have a kind");

        self.board[square.raw_index()] = PieceCode::EMPTY;
        self.occupied.clear(square);
        self.by_color[color.index()].clear(square);
        self.by_kind[color.index()][kind.index()].clear(square);
        self.zobrist ^= zobrist_keys().piece(square, piece);
        piece
    }

    /// 手番を反転し、zobristハッシュを更新する。
    #[inline]
    fn flip_side_to_move(&mut self) {
        self.side_to_move = self.side_to_move.opposite();
        self.zobrist ^= zobrist_keys().side_to_move;
    }

    /// 手番を指定した複製を返す。
    ///
    /// 先獅子トリガーは意図的にそのまま保持する。対局管理層の利き調査は、
    /// どちらの合法手を問うかが変わるだけで、直前の着手による一時状態は
    /// 変わらないという解釈を採るためである。
    pub(crate) fn clone_with_side_to_move(&self, side_to_move: Color) -> Self {
        let mut position = self.clone();
        if position.side_to_move != side_to_move {
            position.flip_side_to_move();
        }
        position
    }

    /// 盤面全体からzobristハッシュを計算し直して返す。増分更新の検証に使う。
    pub(crate) fn recompute_zobrist(&self) -> u64 {
        let keys = zobrist_keys();
        let hash = Square::all()
            .filter_map(|square| self.piece_at(square).map(|piece| keys.piece(square, piece)))
            .fold(keys.side(self.side_to_move), |hash, key| hash ^ key);
        hash ^ keys.lion_trigger_state(self.lion_taken_by_non_lion)
    }

    /// P1成り権保留状態からzobristハッシュを計算し直して返す。
    pub(crate) fn recompute_rights_zobrist(&self) -> u64 {
        let keys = zobrist_keys();
        self.promotion_deferred
            .into_iter()
            .map(|square| keys.promotion_deferred(square))
            .fold(0, |hash, key| hash ^ key)
    }

    /// P1の成り権保留ビットを立て、権利ハッシュを更新する。
    fn set_promotion_deferred(&mut self, square: Square) {
        if !self.promotion_deferred.contains(square) {
            self.promotion_deferred.set(square);
            self.rights_zobrist ^= zobrist_keys().promotion_deferred(square);
        }
    }

    /// P1の成り権保留ビットを消し、権利ハッシュを更新する。
    fn clear_promotion_deferred(&mut self, square: Square) {
        if self.promotion_deferred.contains(square) {
            self.promotion_deferred.clear(square);
            self.rights_zobrist ^= zobrist_keys().promotion_deferred(square);
        }
    }

    /// 指定升の駒がP1の成り権保留状態を持てるかどうかを返す。
    fn promotion_deferred_is_valid(&self, square: Square) -> bool {
        self.piece_at(square).is_some_and(|piece| {
            let Some(color) = piece.color() else {
                return false;
            };
            !piece.is_promoted()
                && piece.kind().is_some_and(PieceKind::can_promote)
                && in_promotion_zone(color, square)
        })
    }

    /// 盤面配列とビットボード集計の整合など、内部不変条件を検査する。
    pub fn validate(&self) -> Result<(), PositionError> {
        let union = self.by_color[0] | self.by_color[1];
        if union != self.occupied {
            return Err(PositionError::ColorAggregateMismatch {
                color: Color::Black,
            });
        }
        if self.by_color[0].intersects(self.by_color[1]) {
            return Err(PositionError::ColorOverlap);
        }

        for color in Color::ALL {
            let mut kinds = Bitboard::EMPTY;
            for kind in PieceKind::ALL {
                kinds |= self.by_kind[color.index()][kind.index()];
            }
            if kinds != self.by_color[color.index()] {
                return Err(PositionError::ColorAggregateMismatch { color });
            }
        }

        for raw in 0..RAW_SQUARE_COUNT {
            match Square::from_raw(raw as u8) {
                None => {
                    if !self.board[raw].is_wall() {
                        return Err(PositionError::PaddingIsNotWall { raw: raw as u8 });
                    }
                }
                Some(square) => {
                    let piece = self.board[raw];
                    if piece.is_wall() {
                        return Err(PositionError::ValidSquareIsWall { square });
                    }
                    if piece.is_empty() {
                        if self.occupied.contains(square) {
                            return Err(PositionError::OccupancyMismatch { square });
                        }
                    } else {
                        let color = piece
                            .color()
                            .ok_or(PositionError::InvalidPieceCode { square })?;
                        let kind = piece
                            .kind()
                            .ok_or(PositionError::InvalidPieceCode { square })?;
                        if !self.occupied.contains(square) {
                            return Err(PositionError::OccupancyMismatch { square });
                        }
                        if !self.by_kind[color.index()][kind.index()].contains(square) {
                            return Err(PositionError::KindMismatch {
                                square,
                                color,
                                kind,
                            });
                        }
                    }
                }
            }
        }

        let padding_mask = [!Bitboard::VALID_WORD; 3];
        let has_padding = |board: Bitboard| {
            board
                .words()
                .iter()
                .zip(padding_mask)
                .any(|(word, mask)| word & mask != 0)
        };
        if has_padding(self.occupied)
            || self.by_color.into_iter().any(has_padding)
            || self.by_kind.into_iter().flatten().any(has_padding)
            || has_padding(self.promotion_deferred)
        {
            return Err(PositionError::PaddingBitSet);
        }
        for square in self.promotion_deferred {
            if !self.promotion_deferred_is_valid(square) {
                return Err(PositionError::InvalidPromotionDeferred { square });
            }
        }
        Ok(())
    }

    /// 合法性を検査せずに`mv`を適用し、[`Undo`]トークンを返す。
    ///
    /// 呼び出し側は、まさにこの局面に対して、`rules`と同じ規則の
    /// [`MoveGenerator::generate_moves`](crate::MoveGenerator::generate_moves)
    /// が生成した着手だけを渡さなければならない。それ以外の着手を渡すと、
    /// panicするか、zobristハッシュや先獅子トリガーを含めて局面を静かに
    /// 壊すことがある。合法性検査付きの適用には
    /// [`Position::try_make_move`]を使う。
    pub fn make_move_unchecked(&mut self, mv: Move, rules: Rules) -> Undo {
        let previous_zobrist = self.zobrist;
        let previous_promotion_deferred = self.promotion_deferred;
        let previous_rights_zobrist = self.rights_zobrist;
        let previous_lion_taken = self.lion_taken_by_non_lion;
        let capture_squares = self.captured_squares(mv);
        let moving_kind = self
            .piece_at(mv.from)
            .and_then(PieceCode::kind)
            .expect("move origin must contain a valid piece");
        let had_promotion_option =
            rules.promotion_choice(self, &mv, moving_kind) == PromotionChoice::PromotionOptional;
        if rules.contains(RuleCode::P1) {
            self.clear_promotion_deferred(mv.from);
            for square in capture_squares.into_iter().flatten() {
                self.clear_promotion_deferred(square);
            }
        }
        let moved_piece_before = self.remove_piece(mv.from);
        debug_assert_eq!(moved_piece_before.color(), Some(self.side_to_move));

        let mut captured = [None; 2];
        for (index, square) in capture_squares.into_iter().enumerate() {
            if let Some(square) = square {
                let piece = self.remove_piece(square);
                debug_assert_eq!(piece.color(), Some(self.side_to_move.opposite()));
                captured[index] = Some(CapturedPiece { square, piece });
            }
        }

        let moved_piece_after = if mv.promote {
            moved_piece_before
                .promote()
                .expect("promoting move must have a promotable piece")
        } else {
            moved_piece_before
        };
        self.put_piece(mv.to, moved_piece_after)
            .expect("generated move must end on an empty square");
        if rules.contains(RuleCode::P1)
            && had_promotion_option
            && !mv.promote
            && in_promotion_zone(
                moved_piece_before
                    .color()
                    .expect("moving piece must have an owner"),
                mv.to,
            )
        {
            self.set_promotion_deferred(mv.to);
        }
        self.flip_side_to_move();
        let by_kirin_promotion = moved_piece_before.kind() == Some(PieceKind::Kirin) && mv.promote;
        self.lion_taken_by_non_lion = (moved_piece_before.kind() != Some(PieceKind::Lion))
            .then(|| {
                captured
                    .into_iter()
                    .flatten()
                    .filter(|captured| captured.piece.kind() == Some(PieceKind::Lion))
                    .map(|captured| LionTrigger {
                        square: captured.square,
                        by_kirin_promotion,
                    })
                    .next_back()
            })
            .flatten();
        let keys = zobrist_keys();
        self.zobrist ^= keys.lion_trigger_state(previous_lion_taken)
            ^ keys.lion_trigger_state(self.lion_taken_by_non_lion);

        Undo {
            mv,
            moved_piece_before,
            captured,
            previous_lion_taken,
            previous_zobrist,
            previous_promotion_deferred,
            previous_rights_zobrist,
        }
    }

    /// `undo`に記録された着手を巻き戻す。
    ///
    /// トークンはこの局面に対する[`Position::make_move_unchecked`]が返した
    /// ものでなければならず、着手は適用と逆の順序で巻き戻す必要がある。
    /// どちらかの前提を破ると、panicするか局面を静かに壊すことがある。
    pub fn unmake_move(&mut self, undo: Undo) {
        self.flip_side_to_move();
        self.remove_piece(undo.mv.to);
        self.put_piece(undo.mv.from, undo.moved_piece_before)
            .expect("move origin must be empty while unmaking");
        for captured in undo.captured.into_iter().flatten() {
            self.put_piece(captured.square, captured.piece)
                .expect("capture square must be empty while unmaking");
        }
        let keys = zobrist_keys();
        self.zobrist ^= keys.lion_trigger_state(self.lion_taken_by_non_lion)
            ^ keys.lion_trigger_state(undo.previous_lion_taken);
        self.lion_taken_by_non_lion = undo.previous_lion_taken;
        debug_assert_eq!(self.zobrist, undo.previous_zobrist);
        self.zobrist = undo.previous_zobrist;
        debug_assert_eq!(self.rights_zobrist, self.recompute_rights_zobrist());
        self.promotion_deferred = undo.previous_promotion_deferred;
        debug_assert_eq!(
            undo.previous_rights_zobrist,
            self.recompute_rights_zobrist()
        );
        self.rights_zobrist = undo.previous_rights_zobrist;
    }
}

/// 駒を1枚ずつ置いて局面を組み立てるビルダー。
pub struct PositionBuilder {
    /// 構築中の局面。
    position: Position,
}

impl PositionBuilder {
    /// 指定手番の空局面から構築を始める。
    pub fn new(side_to_move: Color) -> Self {
        Self {
            position: Position::empty(side_to_move),
        }
    }

    /// 指定升へ駒を置く。
    pub fn put(&mut self, square: Square, piece: PieceCode) -> Result<(), PositionBuildError> {
        self.position.put_piece(square, piece)
    }

    /// 指定升の駒をP1の成り権保留中として記録する。
    pub fn mark_promotion_deferred(&mut self, square: Square) -> Result<(), PositionBuildError> {
        if !self.position.promotion_deferred_is_valid(square) {
            return Err(PositionBuildError::InvalidPosition(
                PositionError::InvalidPromotionDeferred { square },
            ));
        }
        self.position.set_promotion_deferred(square);
        Ok(())
    }

    /// 不変条件を検査し、zobristハッシュを計算し直して局面を返す。
    pub fn finish(mut self) -> Result<Position, PositionBuildError> {
        self.position
            .validate()
            .map_err(PositionBuildError::InvalidPosition)?;
        self.position.zobrist = self.position.recompute_zobrist();
        self.position.rights_zobrist = self.position.recompute_rights_zobrist();
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MoveGenerator;
    use crate::test_util::{position as position_with_pieces, position_from_codes, sq};

    /// 180度回転写像σ(D4マトリクスの座標規約)。内部0始まり座標では(11−筋,11−段)にあたる。
    fn sigma(square: Square) -> Square {
        sq(
            BOARD_FILES - 1 - square.file(),
            BOARD_RANKS - 1 - square.rank(),
        )
    }

    /// 既存局面と同じ盤面を、指定手番・先獅子状態なしで直接構築し直す。
    fn rebuild_board_with_side(source: &Position, side_to_move: Color) -> Position {
        let mut builder = PositionBuilder::new(side_to_move);
        for square in Square::all() {
            if let Some(piece) = source.piece_at(square) {
                builder.put(square, piece).unwrap();
            }
        }
        builder.finish().unwrap()
    }

    /// 非獅子(角行)が相手獅子を取った直後の局面と、獅子が取られた升を返す(第15条1項の前提)。
    fn position_after_non_lion_captures_lion() -> (Position, Square) {
        let captured_lion = sq(1, 1);
        let mut position = position_with_pieces(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::Bishop),
                (captured_lion, Color::White, PieceKind::Lion),
                (sq(10, 10), Color::White, PieceKind::Pawn),
            ],
        );
        position
            .try_make_move(
                Move {
                    from: sq(0, 0),
                    mid: None,
                    to: captured_lion,
                    promote: false,
                },
                &MoveGenerator::standard(),
            )
            .unwrap();
        (position, captured_lion)
    }

    /// 第5条の配置図の機械可読な写し(D4-005-01の期待表)。
    /// 行は段一〜十二、列は配置図の印字順(筋12→筋1)で、内部座標では
    /// 筋f・段rの升が(file=12−f, rank=12−r)にあたる。
    fn article_5_expected_table() -> [[Option<(Color, PieceKind)>; 12]; 12] {
        use PieceKind::{
            Bishop, BlindTiger, CopperGeneral, DragonHorse, DragonKing, DrunkElephant,
            FerociousLeopard, FreeKing, GoBetween, GoldGeneral, King, Kirin, Lance, Lion, Pawn,
            Phoenix, ReverseChariot, Rook, SideMover, SilverGeneral, VerticalMover,
        };
        let b = |kind| Some((Color::Black, kind));
        let w = |kind| Some((Color::White, kind));
        let n = None;
        [
            // 一段目: 香 猛 銅 銀 金 醉 玉 金 銀 銅 猛 香
            [
                w(Lance),
                w(FerociousLeopard),
                w(CopperGeneral),
                w(SilverGeneral),
                w(GoldGeneral),
                w(DrunkElephant),
                w(King),
                w(GoldGeneral),
                w(SilverGeneral),
                w(CopperGeneral),
                w(FerociousLeopard),
                w(Lance),
            ],
            // 二段目: 反 ・ 角 ・ 盲 鳳 麒 盲 ・ 角 ・ 反
            [
                w(ReverseChariot),
                n,
                w(Bishop),
                n,
                w(BlindTiger),
                w(Phoenix),
                w(Kirin),
                w(BlindTiger),
                n,
                w(Bishop),
                n,
                w(ReverseChariot),
            ],
            // 三段目: 横 竪 飛 馬 龍 奔 獅 龍 馬 飛 竪 横
            [
                w(SideMover),
                w(VerticalMover),
                w(Rook),
                w(DragonHorse),
                w(DragonKing),
                w(FreeKing),
                w(Lion),
                w(DragonKing),
                w(DragonHorse),
                w(Rook),
                w(VerticalMover),
                w(SideMover),
            ],
            // 四段目: 歩×12
            [w(Pawn); 12],
            // 五段目: ・ ・ ・ 仲 ・ ・ ・ ・ 仲 ・ ・ ・
            [n, n, n, w(GoBetween), n, n, n, n, w(GoBetween), n, n, n],
            // 六段目・七段目: 空
            [n; 12],
            [n; 12],
            // 八段目: ・ ・ ・ 仲 ・ ・ ・ ・ 仲 ・ ・ ・
            [n, n, n, b(GoBetween), n, n, n, n, b(GoBetween), n, n, n],
            // 九段目: 歩×12
            [b(Pawn); 12],
            // 十段目: 横 竪 飛 馬 龍 獅 奔 龍 馬 飛 竪 横
            [
                b(SideMover),
                b(VerticalMover),
                b(Rook),
                b(DragonHorse),
                b(DragonKing),
                b(Lion),
                b(FreeKing),
                b(DragonKing),
                b(DragonHorse),
                b(Rook),
                b(VerticalMover),
                b(SideMover),
            ],
            // 十一段: 反 ・ 角 ・ 盲 麒 鳳 盲 ・ 角 ・ 反
            [
                b(ReverseChariot),
                n,
                b(Bishop),
                n,
                b(BlindTiger),
                b(Kirin),
                b(Phoenix),
                b(BlindTiger),
                n,
                b(Bishop),
                n,
                b(ReverseChariot),
            ],
            // 十二段: 香 猛 銅 銀 金 王 醉 金 銀 銅 猛 香
            [
                b(Lance),
                b(FerociousLeopard),
                b(CopperGeneral),
                b(SilverGeneral),
                b(GoldGeneral),
                b(King),
                b(DrunkElephant),
                b(GoldGeneral),
                b(SilverGeneral),
                b(CopperGeneral),
                b(FerociousLeopard),
                b(Lance),
            ],
        ]
    }

    // 初期配置は第5条の配置図と全144升で完全に一致し、全駒が未成である(D4-005-01)。
    #[test]
    fn article_5_initial_position_matches_the_full_144_square_table() {
        let initial = Position::initial();
        let table = article_5_expected_table();

        for (row, expected_rank) in table.into_iter().enumerate() {
            // 段d(一=1)は内部rank 12−d、印字列の左からi番目(筋12−i)は内部file iにあたる。
            let rank = BOARD_RANKS - 1 - row as u8;
            for (file, expected) in expected_rank.into_iter().enumerate() {
                let square = sq(file as u8, rank);
                let actual = initial
                    .piece_at(square)
                    .map(|piece| (piece.color().unwrap(), piece.kind().unwrap()));
                assert_eq!(actual, expected, "第5条の{}段目・筋{}", row + 1, 12 - file);
                // 初期局面に成駒は存在しない(第4条3項)。
                if let Some(piece) = initial.piece_at(square) {
                    assert!(!piece.is_promoted());
                }
            }
        }
    }

    // 初期局面の駒数は各側46枚・計92枚・空升52升で、どの列挙経路でも一致する
    // (第4条2項、D4-004-02)。駒種別内訳はD4-005-01の検算に一致する。
    #[test]
    fn article_4_2_initial_piece_counts_match_on_every_enumeration_path() {
        let initial = Position::initial();
        assert_eq!(initial.occupied().popcount(), 92);
        assert_eq!(initial.pieces_of(Color::Black).popcount(), 46);
        assert_eq!(initial.pieces_of(Color::White).popcount(), 46);

        for color in Color::ALL {
            let scanned = Square::all()
                .filter(|&square| {
                    initial.piece_at(square).and_then(PieceCode::color) == Some(color)
                })
                .count();
            assert_eq!(scanned, 46);
            let by_kind_total: u32 = PieceKind::ALL
                .iter()
                .map(|&kind| initial.pieces_of_kind(color, kind).popcount())
                .sum();
            assert_eq!(by_kind_total, 46);
        }
        let empty_squares = Square::all()
            .filter(|&square| initial.piece_at(square).is_none())
            .count();
        assert_eq!(empty_squares, 144 - 92);

        // 各側の駒種別内訳(第5条配置図の検算)。
        let expected_kind_counts = [
            (PieceKind::Pawn, 12),
            (PieceKind::GoBetween, 2),
            (PieceKind::Lance, 2),
            (PieceKind::FerociousLeopard, 2),
            (PieceKind::CopperGeneral, 2),
            (PieceKind::SilverGeneral, 2),
            (PieceKind::GoldGeneral, 2),
            (PieceKind::DrunkElephant, 1),
            (PieceKind::King, 1),
            (PieceKind::ReverseChariot, 2),
            (PieceKind::Bishop, 2),
            (PieceKind::BlindTiger, 2),
            (PieceKind::Phoenix, 1),
            (PieceKind::Kirin, 1),
            (PieceKind::SideMover, 2),
            (PieceKind::VerticalMover, 2),
            (PieceKind::Rook, 2),
            (PieceKind::DragonHorse, 2),
            (PieceKind::DragonKing, 2),
            (PieceKind::FreeKing, 1),
            (PieceKind::Lion, 1),
        ];
        for color in Color::ALL {
            for &(kind, count) in &expected_kind_counts {
                assert_eq!(
                    initial.pieces_of_kind(color, kind).popcount(),
                    count,
                    "{color:?} {kind:?}"
                );
            }
            // 成駒としてのみ現れる8種(第10条)は初期盤上に存在しない。
            for kind in [
                PieceKind::CrownPrince,
                PieceKind::WhiteHorse,
                PieceKind::Whale,
                PieceKind::FlyingOx,
                PieceKind::FreeBoar,
                PieceKind::FlyingStag,
                PieceKind::HornedFalcon,
                PieceKind::SoaringEagle,
            ] {
                assert!(initial.pieces_of_kind(color, kind).is_empty());
            }
        }
    }

    // 初期局面の王駒は先手が王将7十二、後手が玉将6一の各1枚で、太子は存在しない
    // (第5条・第20条1項、D4-005-02・D4-020-01)。王駒の判定は駒種だけで決まる(第3条4項)。
    #[test]
    fn articles_5_and_20_1_each_side_starts_with_exactly_one_royal_king() {
        let initial = Position::initial();
        // 7十二は内部(5,0)、6一は内部(6,11)にあたる(座標規約はD4マトリクス)。
        let expected = [(Color::Black, sq(5, 0)), (Color::White, sq(6, 11))];
        for (color, square) in expected {
            assert_eq!(
                initial.royal_pieces(color).iter().collect::<Vec<_>>(),
                vec![square]
            );
            let piece = initial.piece_at(square).unwrap();
            assert_eq!(piece.kind(), Some(PieceKind::King));
            assert_eq!(piece.color(), Some(color));
            assert!(
                initial
                    .pieces_of_kind(color, PieceKind::CrownPrince)
                    .is_empty()
            );
        }

        // 王駒(王将・玉将・太子)の判定は位置に依存せず、醉象は王駒ではない
        // (第3条4項、第20条2項: 醉象は成って太子となったときに初めて王駒となる)。
        let custom = position_from_codes(
            Color::Black,
            &[
                (sq(3, 3), PieceCode::new(Color::Black, PieceKind::King)),
                (
                    sq(7, 7),
                    PieceCode::new_promoted(Color::Black, PieceKind::CrownPrince).unwrap(),
                ),
                (
                    sq(9, 9),
                    PieceCode::new(Color::Black, PieceKind::DrunkElephant),
                ),
            ],
        );
        assert_eq!(
            custom.royal_pieces(Color::Black),
            Bitboard::from_squares([sq(3, 3), sq(7, 7)])
        );
    }

    // 初期配置は王駒の名称を除きσ(180度回転)で対称である(第5条、D4-005-03)。
    // 左右鏡映では対称にならない行があるため、検査は必ずσで行う。
    #[test]
    fn article_5_initial_position_is_symmetric_under_180_degree_rotation() {
        let initial = Position::initial();
        for square in Square::all() {
            match (initial.piece_at(square), initial.piece_at(sigma(square))) {
                (None, None) => {}
                (Some(piece), Some(mirrored)) => {
                    // 所有者は反転し、駒種と成否は一致する。王将と玉将の名称差は
                    // 駒種上区別しない(第5条、SPEC_UNCLEAR SU-D4-3)。
                    assert_eq!(mirrored.color(), piece.color().map(Color::opposite));
                    assert_eq!(mirrored.kind(), piece.kind());
                    assert_eq!(mirrored.is_promoted(), piece.is_promoted());
                }
                (own, mirrored) => {
                    panic!("σ対称でない: {square:?} => {own:?} / {mirrored:?}")
                }
            }
        }
    }

    // 先手から着手し、1手ごとに手番が相手へ移る(第6条1項、D4-006-01)。
    // 獅子の2段階移動もじっとも全体で1手であり、手番移動は1回だけである
    // (第3条7項・13項、第6条4項、第12条11項)。
    #[test]
    fn article_6_1_black_moves_first_and_the_turn_passes_once_per_move() {
        let generator = MoveGenerator::standard();
        let mut game_position = Position::initial();
        assert_eq!(game_position.side_to_move(), Color::Black);

        let mut moves = Vec::new();
        generator.generate_moves(&game_position, &mut moves);
        game_position.try_make_move(moves[0], &generator).unwrap();
        assert_eq!(game_position.side_to_move(), Color::White);

        moves.clear();
        generator.generate_moves(&game_position, &mut moves);
        game_position.try_make_move(moves[0], &generator).unwrap();
        assert_eq!(game_position.side_to_move(), Color::Black);

        // 獅子の2段階移動(第1段階で捕獲する経由升つきの手)は全体で1手である。
        let lion_home = sq(5, 5);
        let capture_position = position_with_pieces(
            Color::Black,
            &[
                (lion_home, Color::Black, PieceKind::Lion),
                (sq(5, 6), Color::White, PieceKind::Pawn),
            ],
        );
        let mut capture_moves = Vec::new();
        generator.generate_moves(&capture_position, &mut capture_moves);
        let two_stage = capture_moves
            .iter()
            .copied()
            .find(|mv| mv.mid == Some(sq(5, 6)))
            .expect("経由升で捕獲する2段階移動が生成される");
        let mut after_two_stage = capture_position.clone();
        after_two_stage
            .try_make_move(two_stage, &generator)
            .unwrap();
        assert_eq!(after_two_stage.side_to_move(), Color::White);

        // じっとも合法な1手であり、適用後に手番が相手へ移る。
        let lone_lion =
            position_with_pieces(Color::Black, &[(lion_home, Color::Black, PieceKind::Lion)]);
        let mut lone_moves = Vec::new();
        generator.generate_moves(&lone_lion, &mut lone_moves);
        let jitto = lone_moves
            .iter()
            .copied()
            .find(|mv| mv.to == lion_home)
            .expect("じっとが生成される(第12条9項)");
        let mut after_jitto = lone_lion.clone();
        after_jitto.try_make_move(jitto, &generator).unwrap();
        assert_eq!(after_jitto.side_to_move(), Color::White);
    }

    // 取った駒は盤上から除かれ再使用されず、持ち駒に相当する観測状態は存在しない
    // (第4条4項・5項、D4-004-04)。2段階移動の2枚捕獲では総駒数が2減る(第12条4項)。
    #[test]
    fn article_4_4_captured_pieces_are_removed_and_never_reused() {
        let generator = MoveGenerator::standard();

        // 単独捕獲: 総駒数がちょうど1減る。
        let mut single = position_with_pieces(
            Color::Black,
            &[
                (sq(4, 4), Color::Black, PieceKind::Rook),
                (sq(4, 9), Color::White, PieceKind::Pawn),
            ],
        );
        assert_eq!(single.occupied().popcount(), 2);
        single
            .try_make_move(
                Move {
                    from: sq(4, 4),
                    mid: None,
                    to: sq(4, 9),
                    promote: false,
                },
                &generator,
            )
            .unwrap();
        assert_eq!(single.occupied().popcount(), 1);
        assert!(single.pieces_of(Color::White).is_empty());
        assert_eq!(
            single.piece_at(sq(4, 9)).and_then(PieceCode::kind),
            Some(PieceKind::Rook)
        );

        // 獅子の2枚捕獲: 総駒数がちょうど2減る。
        let mut double = position_with_pieces(
            Color::Black,
            &[
                (sq(5, 5), Color::Black, PieceKind::Lion),
                (sq(5, 6), Color::White, PieceKind::Pawn),
                (sq(5, 7), Color::White, PieceKind::Pawn),
            ],
        );
        assert_eq!(double.occupied().popcount(), 3);
        double
            .try_make_move(
                Move {
                    from: sq(5, 5),
                    mid: Some(sq(5, 6)),
                    to: sq(5, 7),
                    promote: false,
                },
                &generator,
            )
            .unwrap();
        assert_eq!(double.occupied().popcount(), 1);
        assert!(double.pieces_of(Color::White).is_empty());

        // 取られた駒は配置・キーのどの観測にも痕跡を残さない: 捕獲後の局面は
        // 同じ配置を直接構築した局面と完全一致する(持ち駒集合は存在しない)。
        let rebuilt =
            position_with_pieces(Color::White, &[(sq(5, 7), Color::Black, PieceKind::Lion)]);
        assert_eq!(double, rebuilt);
        assert_eq!(double.zobrist(), rebuilt.zobrist());
    }

    // 局面キーは全駒の位置・種類・所有者・成否を区別する(第24条1項a・2項、D4-024-01)。
    // 成否の区別は動きが同一の対(生の金将と歩兵の成駒など、第17条4項)にも及ぶ。
    #[test]
    fn article_24_1_a_key_distinguishes_placement_kind_owner_and_promotion() {
        let anchor = [
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
        ];
        let with = |extra: (Square, Color, PieceKind)| {
            let mut pieces = anchor.to_vec();
            pieces.push(extra);
            position_with_pieces(Color::Black, &pieces)
        };

        // (1) 1枚の位置だけが異なる対。
        assert_ne!(
            with((sq(4, 4), Color::Black, PieceKind::GoldGeneral)).zobrist(),
            with((sq(4, 5), Color::Black, PieceKind::GoldGeneral)).zobrist()
        );
        // (2) 同一升・同一所有者で駒種だけが異なる対。
        assert_ne!(
            with((sq(4, 4), Color::Black, PieceKind::GoldGeneral)).zobrist(),
            with((sq(4, 4), Color::Black, PieceKind::SilverGeneral)).zobrist()
        );
        // (3) 同一升・同一駒種で所有者だけが異なる対。
        assert_ne!(
            with((sq(4, 4), Color::Black, PieceKind::Pawn)).zobrist(),
            with((sq(4, 4), Color::White, PieceKind::Pawn)).zobrist()
        );

        // (4) 成否だけが異なる対: 生の金将と歩兵の成駒、生の獅子と麒麟の成駒、
        // 生の醉象と仲人の成駒。
        let anchor_codes = [
            (sq(0, 0), PieceCode::new(Color::Black, PieceKind::King)),
            (sq(11, 11), PieceCode::new(Color::White, PieceKind::King)),
        ];
        for kind in [
            PieceKind::GoldGeneral,
            PieceKind::Lion,
            PieceKind::DrunkElephant,
        ] {
            let with_code = |code| {
                let mut pieces = anchor_codes.to_vec();
                pieces.push((sq(4, 4), code));
                position_from_codes(Color::Black, &pieces)
            };
            assert_ne!(
                with_code(PieceCode::new(Color::Black, kind)).zobrist(),
                with_code(PieceCode::new_promoted(Color::Black, kind).unwrap()).zobrist(),
                "{kind:?}"
            );
        }

        // 逆に、構成要素がすべて一致する2局面はキーが一致する(構築順序を入れ替えて確認)。
        let pieces = [
            (sq(2, 2), Color::Black, PieceKind::Rook),
            (sq(3, 8), Color::White, PieceKind::Bishop),
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
        ];
        let mut reversed = pieces;
        reversed.reverse();
        let forward_position = position_with_pieces(Color::Black, &pieces);
        let reversed_position = position_with_pieces(Color::Black, &reversed);
        assert_eq!(forward_position, reversed_position);
        assert_eq!(forward_position.zobrist(), reversed_position.zobrist());
    }

    // 局面キーは手番側を区別し、その寄与はどの盤面でも現れる(第24条1項b、D4-024-02)。
    #[test]
    fn article_24_1_b_key_distinguishes_side_to_move_on_any_board() {
        assert_ne!(
            Position::empty(Color::Black).zobrist(),
            Position::empty(Color::White).zobrist()
        );

        let pieces = [
            (sq(4, 4), Color::Black, PieceKind::King),
            (sq(7, 7), Color::White, PieceKind::King),
        ];
        assert_ne!(
            position_with_pieces(Color::Black, &pieces).zobrist(),
            position_with_pieces(Color::White, &pieces).zobrist()
        );

        // 初期配置の盤面で手番だけを後手にした局面は、初期局面と同一局面ではない。
        let initial = Position::initial();
        let flipped = rebuild_board_with_side(&initial, Color::White);
        assert!(Square::all().all(|square| initial.piece_at(square) == flipped.piece_at(square)));
        assert_ne!(initial.zobrist(), flipped.zobrist());
    }

    // 局面キーは先獅子による直後の捕獲禁止の有無を反映する(第24条1項c、D4-024-03)。
    #[test]
    fn article_24_1_c_key_reflects_lion_recapture_state() {
        let generator = MoveGenerator::standard();

        // 非獅子が獅子を取った直後は、同一盤面・同一手番の禁止なし局面とキーが異なる。
        let (with_trigger, _) = position_after_non_lion_captures_lion();
        let same_board_without =
            rebuild_board_with_side(&with_trigger, with_trigger.side_to_move());
        assert!(
            Square::all()
                .all(|square| with_trigger.piece_at(square) == same_board_without.piece_at(square))
        );
        assert_ne!(with_trigger.zobrist(), same_board_without.zobrist());

        // 獅子が獅子を取った場合は先獅子が成立せず(第15条6項)、禁止なしの直接構築と一致する。
        let mut lion_takes_lion = position_with_pieces(
            Color::Black,
            &[
                (sq(4, 4), Color::Black, PieceKind::Lion),
                (sq(5, 4), Color::White, PieceKind::Lion),
            ],
        );
        lion_takes_lion
            .try_make_move(
                Move {
                    from: sq(4, 4),
                    mid: None,
                    to: sq(5, 4),
                    promote: false,
                },
                &generator,
            )
            .unwrap();
        assert_eq!(
            lion_takes_lion,
            rebuild_board_with_side(&lion_takes_lion, Color::White)
        );

        // 麒麟が獅子を取って同じ着手で成った場合、標準規則では先獅子が成立し得る(第15条7項)。
        let mut kirin = position_with_pieces(
            Color::Black,
            &[
                (sq(4, 7), Color::Black, PieceKind::Kirin),
                (sq(5, 8), Color::White, PieceKind::Lion),
            ],
        );
        kirin
            .try_make_move(
                Move {
                    from: sq(4, 7),
                    mid: None,
                    to: sq(5, 8),
                    promote: true,
                },
                &generator,
            )
            .unwrap();
        assert_ne!(
            kirin.zobrist(),
            rebuild_board_with_side(&kirin, Color::White).zobrist()
        );

        // 相手が別の着手を行うと禁止は消滅し(第15条5項)、禁止なしの同一配置とキーが一致する。
        let (mut cleared, _) = position_after_non_lion_captures_lion();
        cleared
            .try_make_move(
                Move {
                    from: sq(10, 10),
                    mid: None,
                    to: sq(10, 9),
                    promote: false,
                },
                &generator,
            )
            .unwrap();
        assert_eq!(cleared, rebuild_board_with_side(&cleared, Color::Black));
    }

    // 盤上に獅子が複数ある局面では、禁止の対象升が異なればキーも異なる
    // (第24条1項c、D4-024-03境界)。
    #[test]
    fn article_24_1_c_key_distinguishes_the_prohibition_target_square() {
        let base = position_from_codes(
            Color::Black,
            &[
                (sq(2, 2), PieceCode::new(Color::White, PieceKind::Lion)),
                (
                    sq(9, 9),
                    PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap(),
                ),
                (sq(5, 5), PieceCode::new(Color::Black, PieceKind::Lion)),
            ],
        );

        let mut at_first = base.clone();
        at_first.set_lion_capture(Some(sq(3, 3))).unwrap();
        let mut at_second = base.clone();
        at_second.set_lion_capture(Some(sq(6, 6))).unwrap();
        assert_ne!(at_first.zobrist(), base.zobrist());
        assert_ne!(at_first.zobrist(), at_second.zobrist());

        // 禁止を解除すると元のキーへ完全に戻る。
        at_first.set_lion_capture(None).unwrap();
        assert_eq!(at_first, base);
        assert_eq!(at_first.zobrist(), base.zobrist());

        // 手番側の駒がある升は獅子捕獲升として受理されず、局面は変化しない。
        let mut rejected = base.clone();
        assert_eq!(
            rejected.set_lion_capture(Some(sq(5, 5))),
            Err(PositionError::InvalidLionCapture { square: sq(5, 5) })
        );
        assert_eq!(rejected, base);
    }

    // 構成要素d(P1の成り権保留)は盤面キー(a〜c)と分離して保持される(第24条1項d、
    // D4-024-04)。R2・R3の同一局面判定はa〜cのみ、R1はd込みで行う(第31条)ため、
    // dだけが異なる2局面は盤面キーが一致し権利キーだけが異なる。
    #[test]
    fn article_24_1_d_promotion_rights_key_is_separate_from_the_board_key() {
        let deferred_square = sq(4, 9);
        let piece = PieceCode::new(Color::Black, PieceKind::SilverGeneral);
        let mut plain_builder = PositionBuilder::new(Color::Black);
        plain_builder.put(deferred_square, piece).unwrap();
        let plain = plain_builder.finish().unwrap();

        let mut deferred_builder = PositionBuilder::new(Color::Black);
        deferred_builder.put(deferred_square, piece).unwrap();
        deferred_builder
            .mark_promotion_deferred(deferred_square)
            .unwrap();
        let deferred = deferred_builder.finish().unwrap();

        assert_eq!(plain.zobrist(), deferred.zobrist());
        assert_ne!(plain.rights_zobrist(), deferred.rights_zobrist());

        // 保留状態を持てるのは敵陣内(第3条6項)の未成の成れる駒だけである(第30条P1)。
        // 空升・成れない駒(王将)・成駒・敵陣外の駒への保留指定は拒否される。
        let invalid = |square| {
            Err(PositionBuildError::InvalidPosition(
                PositionError::InvalidPromotionDeferred { square },
            ))
        };
        let mut empty_builder = PositionBuilder::new(Color::Black);
        assert_eq!(
            empty_builder.mark_promotion_deferred(sq(4, 9)),
            invalid(sq(4, 9))
        );

        let cases = [
            (PieceCode::new(Color::Black, PieceKind::King), sq(5, 9)),
            (
                PieceCode::new(Color::Black, PieceKind::SilverGeneral)
                    .promote()
                    .unwrap(),
                sq(6, 9),
            ),
            (
                PieceCode::new(Color::Black, PieceKind::SilverGeneral),
                sq(4, 7),
            ),
        ];
        for (code, square) in cases {
            let mut builder = PositionBuilder::new(Color::Black);
            builder.put(square, code).unwrap();
            assert_eq!(builder.mark_promotion_deferred(square), invalid(square));
        }
    }

    // 局面キーは到達手順・手数・盤外情報に依存しない(第24条3項、D4-024-05)。
    #[test]
    fn article_24_3_key_depends_on_the_position_not_on_the_path() {
        let generator = MoveGenerator::standard();
        let start = || {
            position_with_pieces(
                Color::Black,
                &[
                    (sq(5, 0), Color::Black, PieceKind::King),
                    (sq(4, 4), Color::Black, PieceKind::GoldGeneral),
                    (sq(6, 11), Color::White, PieceKind::King),
                    (sq(7, 7), Color::White, PieceKind::GoldGeneral),
                ],
            )
        };
        let mv = |from, to| Move {
            from,
            mid: None,
            to,
            promote: false,
        };
        let play = |steps: &[Move]| {
            let mut position = start();
            for &step in steps {
                position.try_make_move(step, &generator).unwrap();
            }
            position
        };

        // 手順の入れ替え(transposition)で同一局面へ到達してもキーは一致する。
        let path_a = play(&[
            mv(sq(4, 4), sq(4, 5)),
            mv(sq(7, 7), sq(7, 6)),
            mv(sq(5, 0), sq(5, 1)),
            mv(sq(6, 11), sq(6, 10)),
        ]);
        let path_b = play(&[
            mv(sq(5, 0), sq(5, 1)),
            mv(sq(6, 11), sq(6, 10)),
            mv(sq(4, 4), sq(4, 5)),
            mv(sq(7, 7), sq(7, 6)),
        ]);
        assert_eq!(path_a, path_b);
        assert_eq!(path_a.zobrist(), path_b.zobrist());

        // 往復を含む長い経路(手数が異なる)でも、同じ局面なら同じキーになる。
        let path_long = play(&[
            mv(sq(4, 4), sq(4, 5)),
            mv(sq(7, 7), sq(7, 6)),
            mv(sq(5, 0), sq(5, 1)),
            mv(sq(6, 11), sq(6, 10)),
            mv(sq(4, 5), sq(4, 6)),
            mv(sq(6, 10), sq(6, 11)),
            mv(sq(4, 6), sq(4, 5)),
            mv(sq(6, 11), sq(6, 10)),
        ]);
        assert_eq!(path_long, path_a);
        assert_eq!(path_long.zobrist(), path_a.zobrist());
    }

    // 実装契約(D4-IMP-01): 空盤への1枚配置と同じ升の除去は互いに逆操作であり、
    // 全144升×全駒コード(未成29種＋成駒18種×両所有者)で空盤へ完全に戻る。
    #[test]
    fn placing_then_removing_any_piece_restores_the_empty_position() {
        let pristine = Position::empty(Color::Black);
        let mut codes = Vec::new();
        for color in Color::ALL {
            for kind in PieceKind::ALL {
                codes.push(PieceCode::new(color, kind));
                if let Some(promoted) = PieceCode::new_promoted(color, kind) {
                    codes.push(promoted);
                }
            }
        }

        let mut position = pristine.clone();
        for square in Square::all() {
            for &code in &codes {
                position.put_piece(square, code).unwrap();
                assert!(position.occupied().contains(square));
                assert_eq!(position.remove_piece(square), code);
                assert_eq!(position, pristine, "{square:?} {code:?}");
            }
        }

        // 既に駒がある升への二重配置と、空升・番兵コードの配置は拒否され状態を変えない。
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(sq(4, 4), PieceCode::new(Color::Black, PieceKind::King))
            .unwrap();
        assert_eq!(
            builder.put(sq(4, 4), PieceCode::new(Color::White, PieceKind::King)),
            Err(PositionBuildError::SquareOccupied { square: sq(4, 4) })
        );
        assert_eq!(
            builder.put(sq(5, 5), PieceCode::EMPTY),
            Err(PositionBuildError::EmptyOrWallPiece)
        );
        assert_eq!(
            builder.put(sq(5, 5), PieceCode::WALL),
            Err(PositionBuildError::EmptyOrWallPiece)
        );
    }

    // 実装契約(D4-IMP-02): キーは局面状態の純関数であり、同一の目標局面は
    // 構築経路(配置順序・指し手適用)によらず同一キーになる。
    #[test]
    fn equal_configurations_share_one_key_regardless_of_construction_path() {
        let generator = MoveGenerator::standard();

        // 歩兵の成り(第18条)を経由した局面と、成駒を直接配置した局面。
        let mut played = position_with_pieces(
            Color::Black,
            &[
                (sq(4, 7), Color::Black, PieceKind::Pawn),
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(6, 11), Color::White, PieceKind::King),
            ],
        );
        played
            .try_make_move(
                Move {
                    from: sq(4, 7),
                    mid: None,
                    to: sq(4, 8),
                    promote: true,
                },
                &generator,
            )
            .unwrap();
        let built = position_from_codes(
            Color::White,
            &[
                (
                    sq(4, 8),
                    PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap(),
                ),
                (sq(0, 0), PieceCode::new(Color::Black, PieceKind::King)),
                (sq(6, 11), PieceCode::new(Color::White, PieceKind::King)),
            ],
        );
        assert_eq!(played, built);
        assert_eq!(played.zobrist(), built.zobrist());
    }

    /// 増分更新とundoの検査に使う代表シナリオ(局面・規則・着手列)を返す。
    /// 2枚捕獲・居喰い・じっと・成り・王駒捕獲・成駒の捕獲・先獅子・P1保留を覆う
    /// (D4-IMP-03・D4-IMP-04の境界事例)。
    fn make_unmake_scenarios() -> Vec<(Position, Rules, Vec<Move>)> {
        let mv = |from, mid, to, promote| Move {
            from,
            mid,
            to,
            promote,
        };
        let standard = Rules::standard();
        let p1 = Rules::from_codes(&[RuleCode::P1]).unwrap();
        vec![
            // 獅子の2段階移動による2枚捕獲(第12条4項)。
            (
                position_with_pieces(
                    Color::Black,
                    &[
                        (sq(5, 5), Color::Black, PieceKind::Lion),
                        (sq(5, 6), Color::White, PieceKind::Pawn),
                        (sq(5, 7), Color::White, PieceKind::Pawn),
                    ],
                ),
                standard,
                vec![mv(sq(5, 5), Some(sq(5, 6)), sq(5, 7), false)],
            ),
            // 居喰い(第3条14項)。
            (
                position_with_pieces(
                    Color::Black,
                    &[
                        (sq(5, 5), Color::Black, PieceKind::Lion),
                        (sq(5, 6), Color::White, PieceKind::Pawn),
                    ],
                ),
                standard,
                vec![mv(sq(5, 5), Some(sq(5, 6)), sq(5, 5), false)],
            ),
            // じっと(第3条13項)。
            (
                position_with_pieces(Color::Black, &[(sq(5, 5), Color::Black, PieceKind::Lion)]),
                standard,
                vec![mv(sq(5, 5), None, sq(5, 5), false)],
            ),
            // 成りを伴う手(第18条1項)。
            (
                position_with_pieces(Color::Black, &[(sq(4, 7), Color::Black, PieceKind::Pawn)]),
                standard,
                vec![mv(sq(4, 7), None, sq(4, 8), true)],
            ),
            // 成駒を取る手(undoで取られた駒の成否も復元される)。
            (
                position_from_codes(
                    Color::Black,
                    &[
                        (sq(4, 4), PieceCode::new(Color::Black, PieceKind::Rook)),
                        (
                            sq(4, 9),
                            PieceCode::new_promoted(Color::White, PieceKind::FreeBoar).unwrap(),
                        ),
                    ],
                ),
                standard,
                vec![mv(sq(4, 4), None, sq(4, 9), false)],
            ),
            // 王駒を取る手(第21条1項)。
            (
                position_with_pieces(
                    Color::Black,
                    &[
                        (sq(4, 4), Color::Black, PieceKind::Rook),
                        (sq(4, 9), Color::White, PieceKind::King),
                    ],
                ),
                standard,
                vec![mv(sq(4, 4), None, sq(4, 9), false)],
            ),
            // 非獅子による獅子捕獲(先獅子トリガー、第15条)と、その消滅(第15条5項)。
            (
                position_with_pieces(
                    Color::Black,
                    &[
                        (sq(0, 0), Color::Black, PieceKind::Bishop),
                        (sq(1, 1), Color::White, PieceKind::Lion),
                        (sq(10, 10), Color::White, PieceKind::Pawn),
                    ],
                ),
                standard,
                vec![
                    mv(sq(0, 0), None, sq(1, 1), false),
                    mv(sq(10, 10), None, sq(10, 9), false),
                ],
            ),
            // 麒麟が獅子を取って成る手(第15条7項)。
            (
                position_with_pieces(
                    Color::Black,
                    &[
                        (sq(4, 7), Color::Black, PieceKind::Kirin),
                        (sq(5, 8), Color::White, PieceKind::Lion),
                    ],
                ),
                standard,
                vec![mv(sq(4, 7), None, sq(5, 8), true)],
            ),
            // P1の成り権保留の設定と消滅(第30条P1、第24条1項d)。
            (
                position_with_pieces(
                    Color::Black,
                    &[
                        (sq(4, 7), Color::Black, PieceKind::SilverGeneral),
                        (sq(10, 10), Color::White, PieceKind::GoldGeneral),
                    ],
                ),
                p1,
                vec![
                    mv(sq(4, 7), None, sq(4, 8), false),
                    mv(sq(10, 10), None, sq(10, 9), false),
                    mv(sq(4, 8), None, sq(4, 9), false),
                    mv(sq(10, 9), None, sq(10, 10), false),
                    mv(sq(4, 9), None, sq(4, 10), false),
                    mv(sq(10, 10), None, sq(10, 9), false),
                ],
            ),
        ]
    }

    // 実装契約(D4-IMP-03): 増分維持されるキー・占有集合は、毎手、素の盤面からの
    // 全再計算および整合検査と一致する。
    #[test]
    fn incrementally_maintained_state_matches_full_recomputation() {
        for (mut position, rules, moves) in make_unmake_scenarios() {
            let generator = MoveGenerator::new(rules);
            for mv in moves {
                position.try_make_move(mv, &generator).unwrap();
                assert_eq!(position.zobrist(), position.recompute_zobrist(), "{mv:?}");
                assert_eq!(
                    position.rights_zobrist(),
                    position.recompute_rights_zobrist(),
                    "{mv:?}"
                );
                assert_eq!(position.validate(), Ok(()), "{mv:?}");
            }
        }
    }

    // 実装契約(D4-IMP-04): 着手の適用と取り消しは恒等写像を合成し、全観測可能状態
    // (盤面・手番・キー・先獅子状態・成り権保留)が完全一致で復元される。
    #[test]
    fn unmake_restores_every_observable_component() {
        for (mut position, rules, moves) in make_unmake_scenarios() {
            let generator = MoveGenerator::new(rules);
            let mut trail = Vec::new();
            for mv in moves {
                let snapshot = position.clone();
                let undo = position.try_make_move(mv, &generator).unwrap();
                trail.push((snapshot, undo));
            }
            while let Some((snapshot, undo)) = trail.pop() {
                position.unmake_move(undo);
                assert_eq!(position, snapshot);
                assert_eq!(position.zobrist(), snapshot.zobrist());
                assert_eq!(position.rights_zobrist(), snapshot.rights_zobrist());
            }
        }
    }

    // 実装契約(D4-IMP-09): 固定シードの一様ランダムプレイアウトで、毎手、
    // (1)増分キー＝全再計算、(2)同じ指し手列の再適用による再構築との全観測一致、
    // (3)総駒数=92−累計捕獲枚数、(4)手番の交替則(第6条1項)を検証する。
    // 終了後は全undoで初期局面へ完全復帰し、同一シードは同一の指し手列を再現する。
    #[test]
    fn seeded_random_playouts_uphold_conservation_replay_and_undo_invariants() {
        let generator = MoveGenerator::standard();
        let seeds = [0x5a4f_4252_4953_5401_u64, 0x6d69_6e61_7365_4434];
        let games_per_seed = 4;
        let max_plies = 72;
        let mut captures_seen = 0_u32;
        let mut promotions_seen = 0_u32;
        let mut double_moves_seen = 0_u32;
        let mut recorded_move_lists = Vec::new();

        for seed in seeds {
            let mut rng = XorShift64::new(seed);
            for game in 0..games_per_seed {
                let initial = Position::initial();
                let mut position = initial.clone();
                let mut history = Vec::new();
                let mut played = Vec::new();
                let mut captured_total = 0_u32;

                for ply in 0..max_plies {
                    // n手適用後の手番は、nが偶数なら先手、奇数なら後手である(第6条1項)。
                    let expected_side = if played.len() % 2 == 0 {
                        Color::Black
                    } else {
                        Color::White
                    };
                    assert_eq!(position.side_to_move(), expected_side);

                    let mut moves = Vec::new();
                    generator.generate_moves(&position, &mut moves);
                    if moves.is_empty() {
                        break;
                    }
                    let mv = moves[rng.next() as usize % moves.len()];
                    captured_total += position.captured_squares(mv).iter().flatten().count() as u32;
                    if mv.mid.is_some() {
                        double_moves_seen += 1;
                    }
                    if mv.promote {
                        promotions_seen += 1;
                    }
                    history.push(position.make_move_unchecked(mv, Rules::standard()));
                    played.push(mv);

                    let context = format!("seed={seed:#x} game={game} ply={ply}");
                    assert_eq!(
                        position.zobrist(),
                        position.recompute_zobrist(),
                        "{context}"
                    );
                    assert_eq!(
                        position.rights_zobrist(),
                        position.recompute_rights_zobrist(),
                        "{context}"
                    );
                    assert_eq!(position.validate(), Ok(()), "{context}");
                    // 盤上総駒数＝92−累計捕獲枚数(第4条4項)。
                    assert_eq!(
                        position.occupied().popcount(),
                        92 - captured_total,
                        "{context}"
                    );

                    // 同じ指し手列を初期局面へ適用し直した再構築局面と全観測で一致する。
                    let mut replayed = initial.clone();
                    for &past in &played {
                        replayed.make_move_unchecked(past, Rules::standard());
                    }
                    assert_eq!(position, replayed, "{context}");
                }
                captures_seen += captured_total;

                // 全着手を逆順にundoすると初期局面(第5条・先手番・キー)へ完全復帰する。
                while let Some(undo) = history.pop() {
                    position.unmake_move(undo);
                }
                assert_eq!(position, initial, "seed={seed:#x} game={game}");

                recorded_move_lists.push((seed, played));
            }
        }

        // 検査力の担保: 捕獲・成り・2段階移動がプレイアウト中に実際に出現した。
        assert!(captures_seen > 0, "捕獲が出現しないシードは検査力が弱い");
        assert!(promotions_seen > 0, "成りが出現しないシードは検査力が弱い");
        assert!(
            double_moves_seen > 0,
            "2段階移動が出現しないシードは検査力が弱い"
        );

        // 決定性: 同一シードからの再実行は同一の指し手列を再現する。
        for seed in seeds {
            let mut rng = XorShift64::new(seed);
            for game in 0..games_per_seed {
                let mut position = Position::initial();
                let mut replayed_moves = Vec::new();
                for _ in 0..max_plies {
                    let mut moves = Vec::new();
                    generator.generate_moves(&position, &mut moves);
                    if moves.is_empty() {
                        break;
                    }
                    let mv = moves[rng.next() as usize % moves.len()];
                    position.make_move_unchecked(mv, Rules::standard());
                    replayed_moves.push(mv);
                }
                let recorded = recorded_move_lists
                    .iter()
                    .filter(|(recorded_seed, _)| *recorded_seed == seed)
                    .nth(game)
                    .map(|(_, list)| list.clone())
                    .unwrap();
                assert_eq!(replayed_moves, recorded, "seed={seed:#x} game={game}");
            }
        }
    }
}
