//! 既存の学習データへの来歴の付与。

use super::rescore::RescoreCommandError;
use minase::datagen::{
    data_error,
    provenance::{ProvenanceArguments, write_mapped_provenance},
};
use minase::training::provenance::{ResultOrigin, StartOrigin};
use std::io;

/// 元MNSDから教師情報を取り、明示された由来とともに来歴を書き出す。
pub(super) fn write_provenance(arguments: &ProvenanceArguments) -> io::Result<()> {
    if arguments.result_origin != ResultOrigin::Selfplay
        || arguments.start_origin != StartOrigin::Random
    {
        return Err(data_error(RescoreCommandError::UnsupportedOrigin));
    }
    write_mapped_provenance(arguments, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cli::Arguments, test_support::RescoreFixture};
    use clap::Parser;
    use minase::datagen::provenance::hex;
    use minase::training::{
        provenance::{Provenance, SearchCondition},
        rescore,
    };
    use std::fs::File;

    #[test]
    fn provenance_command_uses_header_and_refuses_overwrite_or_implicit_origins() {
        let fixture = RescoreFixture::new();
        let mut arguments = ProvenanceArguments {
            input: fixture.arguments.input.clone(),
            output: fixture.directory.join("provenance.json"),
            result_origin: ResultOrigin::Selfplay,
            start_origin: StartOrigin::Random,
            lambda: 0.75,
            search_condition: SearchCondition::InGame,
        };
        write_provenance(&arguments).unwrap();
        let provenance = Provenance::read(File::open(&arguments.output).unwrap()).unwrap();
        assert_eq!(
            provenance.mnsd_sha256,
            hex(&rescore::sha256(File::open(&arguments.input).unwrap()).unwrap())
        );
        assert_eq!(provenance.teacher.generation_commit, "a".repeat(40));
        assert_eq!(provenance.teacher.network_checksum, hex(&[7; 32]));
        assert_eq!(provenance.teacher.nodes, 100_000);
        assert_eq!(provenance.lambda, 0.75);
        assert!(provenance.games.is_none());
        assert_eq!(
            write_provenance(&arguments).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        arguments.output = fixture.directory.join("invalid.json");
        arguments.start_origin = StartOrigin::HumanGame;
        assert!(write_provenance(&arguments).is_err());
        assert!(!arguments.output.exists());
        for required in ["--result-origin", "--start-origin", "--lambda"] {
            let mut args = vec![
                "selfplay_gen",
                "provenance",
                "--input",
                "in",
                "--output",
                "out",
                "--result-origin",
                "selfplay",
                "--start-origin",
                "random",
                "--lambda",
                "0.75",
            ];
            let index = args.iter().position(|&value| value == required).unwrap();
            args.drain(index..index + 2);
            assert!(Arguments::try_parse_from(args).is_err());
        }
    }
}
