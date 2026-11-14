//! 自己対局データの生成条件を確定し、再開できる出力処理を起動する。

use super::{
    cli::GenerateArguments,
    opening::{OpeningError, read_openings},
    output::{GenerationConditions, generate_to_directory},
};
use minase::datagen::git::{git_output, validate_commit_hash};
use minase::datagen::{data_error, hex, invalid_data};
use minase_core::Rules;
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    num::{NonZeroU16, NonZeroU32},
};

/// git、埋め込みネット、および引数から生成条件を確定して教師を生成する。
pub(super) fn generate(arguments: &GenerateArguments) -> io::Result<()> {
    let opening_bytes = arguments.openings.as_ref().map(fs::read).transpose()?;
    let openings = opening_bytes.as_deref().map(read_openings).transpose()?;
    if openings.is_some() && arguments.random_moves != 0 {
        return Err(data_error(OpeningError::RandomMoves));
    }
    let game_count = match &openings {
        Some(openings) => u32::try_from(openings.len()).map_err(data_error)?,
        None => arguments
            .games
            .ok_or_else(|| data_error(OpeningError::MissingGames))?,
    };
    let games = NonZeroU32::new(game_count).ok_or_else(|| data_error(OpeningError::Empty))?;
    let nodes = NonZeroU32::new(arguments.nodes)
        .ok_or_else(|| invalid_data("node count must be greater than zero"))?;
    let max_ply = NonZeroU16::new(arguments.max_ply)
        .ok_or_else(|| invalid_data("maximum ply must be greater than zero"))?;
    let rules = Rules::LISHOGI;
    let generation_commit = git_output(&["rev-parse", "HEAD"])?;
    validate_commit_hash(&generation_commit)?;
    let dirty = !git_output(&["status", "--porcelain"])?.is_empty();
    if dirty && !arguments.allow_dirty {
        return Err(invalid_data(
            "the working tree is dirty; commit changes or pass --allow-dirty",
        ));
    }
    let nnue = minase::eval::weights().map_err(data_error)?;
    let openings_sha256 = opening_bytes
        .as_deref()
        .map(|bytes| hex(&Sha256::digest(bytes)));
    let conditions = GenerationConditions::new(
        rules,
        generation_commit,
        *nnue.checksum(),
        arguments.seed,
        games,
        arguments.chunk_games,
        nodes,
        arguments.random_moves,
        max_ply,
        arguments.hash_mb,
        openings_sha256,
        dirty,
    )
    .map_err(data_error)?;
    let report = generate_to_directory(
        &arguments.output_dir,
        &conditions,
        nnue.as_ref(),
        openings.as_deref(),
        arguments.concurrency,
    )?;
    report.print(&conditions, arguments.concurrency);
    Ok(())
}
