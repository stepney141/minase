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

/// git、外部ファイルのネット、および引数から生成条件を確定して教師を生成する。
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
    let network =
        minase::eval::nnue::read_network_file(&arguments.eval_file).map_err(data_error)?;
    let openings_sha256 = opening_bytes
        .as_deref()
        .map(|bytes| hex(&Sha256::digest(bytes)));
    let conditions = GenerationConditions::new(
        rules,
        generation_commit,
        *network.sha256(),
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
        network.network().as_ref(),
        openings.as_deref(),
        arguments.concurrency,
    )?;
    report.print(&conditions, arguments.concurrency);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selfplay_gen::{
        cli::{Arguments, Operation},
        test_support::TemporaryDirectory,
    };
    use clap::Parser;
    use minase::eval::nnue::NetworkFileError;
    use minase::training::records::Reader;
    use std::path::Path;

    const NETWORK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/nets/test-layer-stack.bin");

    fn arguments(directory: &Path) -> GenerateArguments {
        let Operation::Generate(mut args) = Arguments::try_parse_from([
            "selfplay_gen",
            "generate",
            "--eval-file",
            NETWORK,
            "--output-dir",
            "unused",
            "--games",
            "2",
            "--chunk-games",
            "1",
            "--seed",
            "7",
            "--nodes",
            "1",
            "--random-moves",
            "0",
            "--max-ply",
            "1",
            "--hash-mb",
            "1",
            "--concurrency",
            "2",
            "--allow-dirty",
        ])
        .unwrap()
        .command
        else {
            panic!("generate arguments")
        };
        args.output_dir = directory.join("output");
        args
    }

    /// 指定ファイル全体の識別子を生成条件と教師ヘッダの両方に記録する。
    #[test]
    fn generation_records_entire_network_file_checksum() {
        let temporary = TemporaryDirectory::new();
        let args = arguments(&temporary.0);
        generate(&args).unwrap();
        let expected: [u8; 32] = Sha256::digest(fs::read(NETWORK).unwrap()).into();
        let config: serde_json::Value =
            serde_json::from_slice(&fs::read(args.output_dir.join("config.json")).unwrap())
                .unwrap();
        assert_eq!(config["network_checksum"], hex(&expected));
        let reader =
            Reader::new(fs::File::open(args.output_dir.join("teacher.mnsd")).unwrap()).unwrap();
        assert_eq!(reader.header().network_checksum(), &expected);
    }

    #[test]
    fn simple_generation_records_entire_network_file_checksum() {
        let temporary = TemporaryDirectory::new();
        let mut args = arguments(&temporary.0);
        let bytes = minase::eval::nnue::network::test_util::fixed_simple().encode();
        args.eval_file = temporary.0.join("simple.bin");
        fs::write(&args.eval_file, &bytes).unwrap();
        // 1手で王将を取れる実戦開始形式の局面により、ネットの棋力に終局を依存させない。
        use minase_core::{
            Color::{Black, White},
            PieceKind::{King, Pawn, Rook},
            test_util::{position, sq},
        };
        let start = position(
            Black,
            &[
                (sq(0, 0), Black, King),
                (sq(5, 8), White, King),
                (sq(5, 5), Black, Rook),
                (sq(0, 1), Black, Rook),
                (sq(11, 9), White, Pawn),
            ],
        );
        let opening = temporary.0.join("opening.txt");
        fs::write(
            &opening,
            format!("simple 60 {} - 61\n", minase_core::to_sfen(&start)),
        )
        .unwrap();
        args.openings = Some(opening);
        args.nodes = 100;
        args.max_ply = 61;
        generate(&args).unwrap();
        let expected: [u8; 32] = Sha256::digest(&bytes).into();
        let reader =
            Reader::new(fs::File::open(args.output_dir.join("teacher.mnsd")).unwrap()).unwrap();
        assert!(reader.header().game_count() >= 1);
        assert_eq!(reader.header().network_checksum(), &expected);
    }

    /// 読めないファイルと復号できないファイルの原因を型のまま保持し、出力を作らない。
    #[test]
    fn generation_preserves_network_file_errors_without_output() {
        let temporary = TemporaryDirectory::new();
        let mut args = arguments(&temporary.0);
        args.eval_file = temporary.0.join("network.bin");
        for exists in [false, true] {
            if exists {
                fs::write(&args.eval_file, b"invalid network").unwrap();
            }
            let error = generate(&args).unwrap_err();
            let cause = error
                .get_ref()
                .unwrap()
                .downcast_ref::<NetworkFileError>()
                .unwrap();
            match cause {
                NetworkFileError::Read { path, .. } => {
                    assert!(!exists);
                    assert_eq!(path, &args.eval_file);
                }
                NetworkFileError::Decode { path, .. } => {
                    assert!(exists);
                    assert_eq!(path, &args.eval_file);
                }
            }
            assert!(!args.output_dir.exists());
        }
    }
}
