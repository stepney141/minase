use super::Error;
use minase::training::records::{Error as FormatError, Origin, decode_position, encode_board};
use minase_core::{Color, GameResult, Position, Square};

/// 手番側から見た終局結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// 負け。
    Loss,
    /// 引き分け。
    Draw,
    /// 勝ち。
    Win,
}
impl Outcome {
    /// 対局結果を指定した陣営の視点へ変換する。
    pub fn from_game_result(result: GameResult, side: Color) -> Self {
        match result {
            GameResult::Win { winner, .. } if winner == side => Self::Win,
            GameResult::Win { .. } => Self::Loss,
            GameResult::Draw { .. } => Self::Draw,
        }
    }
}

/// 学習・診断へ渡す局面と教師情報。Datasetから取り出したときだけ盤を展開する。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    board: [u8; 144],
    side: Color,
    lion: Option<Square>,
    lion_promotion: bool,
    pub(super) metadata: Metadata,
    origin: Origin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Metadata {
    pub game: u32,
    pub ply: u16,
    pub score: i16,
    pub outcome: Outcome,
}

impl Sample {
    /// 局面と手番側の教師情報から標本を作る。
    pub fn from_position(
        position: &Position,
        score: i16,
        outcome: Outcome,
        game_number: u32,
        ply: u16,
        origin: Origin,
    ) -> Result<Self, Error> {
        let board = encode_board(position)?;
        let trigger = position.lion_taken_by_non_lion();
        Ok(Self {
            board,
            side: position.side_to_move(),
            lion: trigger.map(|t| t.square),
            lion_promotion: trigger.is_some_and(|t| t.by_kirin_promotion),
            metadata: Metadata {
                game: game_number,
                ply,
                score,
                outcome,
            },
            origin,
        })
    }
    /// 駒コードと成り権保留の印を升順に返す。
    pub fn board_codes(&self) -> &[u8; 144] {
        &self.board
    }
    /// 局面を作った経路を返す。
    pub fn origin(&self) -> Origin {
        self.origin
    }
    /// 指定した由来の標本を返す。
    pub fn with_origin(mut self, origin: Origin) -> Self {
        self.origin = origin;
        self
    }
    /// 手番を返す。
    pub fn side_to_move(&self) -> Color {
        self.side
    }
    /// 先獅子の対象升を返す。
    pub fn lion_square(&self) -> Option<Square> {
        self.lion
    }
    /// 先獅子の麒麟成りの印を返す。
    pub fn lion_by_kirin_promotion(&self) -> bool {
        self.lion_promotion
    }
    /// 手番側の探索値を返す。
    pub fn score(&self) -> i16 {
        self.metadata.score
    }
    /// 手番側の終局結果を返す。
    pub fn outcome(&self) -> Outcome {
        self.metadata.outcome
    }
    /// 対局番号を返す。
    pub fn game_number(&self) -> u32 {
        self.metadata.game
    }
    /// 開始局面からの項目番号を加えた手数を返す。
    pub fn ply(&self) -> u16 {
        self.metadata.ply
    }
    /// 診断用に駒だけを除く。先獅子の印を保つため、復元時に検証を要する。
    pub(crate) fn without_piece(&self, square: usize) -> Self {
        let mut sample = self.clone();
        sample.board[square] = 0;
        sample
    }
    /// 盤、手番、成り権保留、先獅子を復元し、不変条件を検査する。
    pub fn to_position(&self) -> Result<Position, Error> {
        decode_position(&self.board, self.side, self.lion, self.lion_promotion).map_err(|error| {
            match error {
                FormatError::PositionBuild(error) => Error::PositionBuild(error),
                FormatError::Position(error) => Error::Position(error),
                FormatError::LionPromotionMismatch => Error::LionPromotionMismatch {
                    recorded: self.lion_promotion,
                    restored: !self.lion_promotion,
                },
                error => error.into(),
            }
        })
    }
    pub(super) fn append_compact(&self, bytes: &mut Vec<u8>) {
        let mut occupied = [0; 18];
        for (i, &code) in self.board.iter().enumerate() {
            if code != 0 {
                occupied[i / 8] |= 1 << (i % 8);
            }
        }
        bytes.extend(occupied);
        bytes.extend([
            self.side as u8,
            self.lion.map_or(255, |s| s.dense_index() as u8),
            u8::from(self.lion_promotion),
        ]);
        bytes.extend(self.board.iter().copied().filter(|&code| code != 0));
    }
    pub(super) fn from_compact(bytes: &[u8], metadata: Metadata, origin: Origin) -> Self {
        // この列は検証済み局面からappend_compactだけが作り、外部入力には公開しない。
        let mut board = [0; 144];
        let mut cursor = 21;
        for (i, code) in board.iter_mut().enumerate() {
            if bytes[i / 8] & (1 << (i % 8)) != 0 {
                *code = bytes[cursor];
                cursor += 1;
            }
        }
        Self {
            board,
            side: if bytes[18] == 0 {
                Color::Black
            } else {
                Color::White
            },
            lion: Square::from_dense(usize::from(bytes[19])),
            lion_promotion: bytes[20] != 0,
            metadata,
            origin,
        }
    }
}
