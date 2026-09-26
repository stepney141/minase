//! 実戦開始局面の読み込みとランダム序盤の生成。

use minase::datagen::{data_error, invalid_data};
use minase::rng::{XorShift64, derive_seed};
use minase::training::provenance::GameOrigin;
use minase::{Color, Game, GameStatus, Position, Rules};
use std::{
    fs::File,
    io::{self, BufRead, BufReader},
    num::{NonZeroU64, NonZeroUsize},
    path::Path,
};

/// 実戦開始の局面と、元棋譜での位置。
pub(super) struct Opening {
    pub(super) origin: GameOrigin,
    position: Position,
    pub(super) ply: u32,
}

/// 開始局面一覧または生成引数の不整合。
#[derive(Debug)]
pub(super) enum OpeningError {
    RandomMoves,
    MissingGames,
    Empty,
    Line {
        line: usize,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    InvalidFields,
    InvalidPosition,
}

impl std::fmt::Display for OpeningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RandomMoves => f.write_str("--openings requires --random-moves 0"),
            Self::MissingGames => f.write_str("--games is required without --openings"),
            Self::Empty => f.write_str("opening list is empty"),
            Self::Line { line, source } => write!(f, "opening line {line}: {source}"),
            Self::InvalidFields => {
                f.write_str("expected <id> <ply> <extended SFEN>, with matching move number")
            }
            Self::InvalidPosition => {
                f.write_str("opening must have both royals, legal moves, and no deferred promotion")
            }
        }
    }
}

impl std::error::Error for OpeningError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Line { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

/// 一覧の全行を検査し、先獅子の状態を含めて復元する。
pub(super) fn read_openings(path: &Path) -> io::Result<Vec<Opening>> {
    use minase::notation::sfen::parse_extended_sfen;
    let mut openings = Vec::new();
    for (index, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let parse = || -> Result<Opening, Box<dyn std::error::Error + Send + Sync>> {
            let line = line?;
            let mut fields = line.split_whitespace();
            let id = fields.next().ok_or(OpeningError::InvalidFields)?;
            let ply: u32 = fields.next().ok_or(OpeningError::InvalidFields)?.parse()?;
            let sfen = fields.collect::<Vec<_>>().join(" ");
            let setup = parse_extended_sfen(&sfen, Rules::ENGINE_DEFAULT.moves)?;
            if ply >= u32::from(u16::MAX) || setup.next_move_number() != ply + 1 {
                return Err(OpeningError::InvalidFields.into());
            }
            let (mut position, lion, _) = setup.into_parts();
            position.set_lion_capture(lion)?;
            let game = Game::from_position(Rules::ENGINE_DEFAULT, position.clone());
            if !position.promotion_deferred().is_empty()
                || game.legal_moves().is_empty()
                || position.royal_pieces(Color::Black).is_empty()
                || position.royal_pieces(Color::White).is_empty()
            {
                return Err(OpeningError::InvalidPosition.into());
            }
            Ok(Opening {
                origin: GameOrigin {
                    game: u32::try_from(index + 1)?,
                    id: id.to_owned(),
                    ply: Some(ply),
                },
                position,
                ply,
            })
        };
        openings.push(parse().map_err(|source| {
            data_error(OpeningError::Line {
                line: index + 1,
                source,
            })
        })?);
    }
    if openings.is_empty() {
        return Err(data_error(OpeningError::Empty));
    }
    Ok(openings)
}

/// 生成の最初の探索に渡す局面を返す。
pub(super) fn starting_game(
    rules: Rules,
    seed: NonZeroU64,
    opening: Option<&Opening>,
) -> io::Result<Game> {
    match opening {
        Some(opening) => Ok(Game::from_position(rules, opening.position.clone())),
        None => generate_opening(rules, seed),
    }
}

/// 決定的な8手から16手のランダム序盤を作る。
pub(super) fn generate_opening(rules: Rules, game_seed: NonZeroU64) -> io::Result<Game> {
    let mut opening_seed = derive_seed(game_seed.get(), 0);
    loop {
        let mut game = Game::new(rules);
        let mut rng = XorShift64::new(opening_seed);
        let opening_plies = 8 + rng.index(NonZeroUsize::new(9).unwrap());
        let mut finished = false;
        for _ in 0..opening_plies {
            let legal_moves = game.legal_moves();
            if legal_moves.is_empty() {
                return Err(invalid_data(
                    "an opening game is ongoing but has no legal moves",
                ));
            }
            let move_count = NonZeroUsize::new(legal_moves.len())
                .expect("the empty move list was rejected above");
            let selected = legal_moves[rng.index(move_count)];
            let status = game.play(selected).map_err(|error| {
                invalid_data(format!("random opening move was rejected: {error}"))
            })?;
            if matches!(status, GameStatus::Finished(_)) {
                finished = true;
                break;
            }
        }
        if !finished {
            return Ok(game);
        }
        opening_seed = derive_seed(opening_seed.get(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        play::{PlaySettings, play_game_from_opening},
        test_support::{RescoreFixture, test_table},
    };
    use std::fs;

    // フェーズ5: 一覧を最初の探索へそのまま渡し、開始局面だけは記録しない。
    #[test]
    fn human_opening_is_exact_and_records_start_after_its_original_ply() {
        use minase::notation::{
            sfen::{SetupPosition, to_extended_sfen},
            usi,
        };
        let fixture = RescoreFixture::new();
        let input: serde_json::Value =
            include_str!("../../../tests/fixtures/lishogi_import_cases.ndjson")
                .lines()
                .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                .find(|game| game["id"] == "opening-prefix-2u7dwJf9")
                .unwrap();
        let mut original = Game::new(Rules::ENGINE_DEFAULT);
        for text in input["moves"].as_str().unwrap().split_whitespace().take(60) {
            original
                .play(usi::parse(original.position(), text).unwrap())
                .unwrap();
        }
        let setup = SetupPosition::new(
            original.position().clone(),
            original.position().lion_capture_square(),
            61,
        )
        .unwrap();
        let path = fixture.directory.join("openings.txt");
        fs::write(&path, format!("source 60 {}\n", to_extended_sfen(&setup))).unwrap();
        let openings = read_openings(&path).unwrap();
        let start = starting_game(
            Rules::ENGINE_DEFAULT,
            derive_seed(42, 1),
            Some(&openings[0]),
        )
        .unwrap();
        assert_eq!(start.position(), original.position());
        assert_eq!(start.ply_count(), 0);
        let completed = play_game_from_opening(
            &minase::eval::weights().unwrap(),
            Rules::ENGINE_DEFAULT,
            1,
            PlaySettings {
                base_seed: 42,
                nodes: 200,
                random_moves: 0,
                max_ply: 4000,
            },
            &mut test_table(),
            Some(&openings[0]),
        )
        .unwrap();
        assert_eq!(completed.stats.discarded_games, 0);
        assert_eq!(completed.stats.planned_injections, 0);
        assert!(!completed.records.is_empty());
        assert!(completed.records.iter().all(|r| r.record.ply() > 60));
    }
}
