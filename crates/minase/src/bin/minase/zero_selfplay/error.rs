//! 下位のエラー型と、生成器の契約違反を保持するエラー。

use std::{any::Any, fmt, io, num::NonZeroUsize, path::PathBuf};

use minase::{
    nn::{NnError, tensorrt_cache::GpuQueryError},
    search::{SearchError, mate_in_three, mcts},
    training::records,
};
use minase_core::GameError;

#[derive(Debug)]
pub(super) enum Error {
    Mcts(mcts::Error),
    Mate(mate_in_three::Error),
    Inference(NnError),
    GpuQuery(GpuQueryError),
    Records(records::Error),
    Game(GameError),
    Search(SearchError),
    Io(io::Error),
    Json(serde_json::Error),
    CacheRootUnset,
    ThreadsNotMultipleOfDevices {
        threads: NonZeroUsize,
        devices: NonZeroUsize,
    },
    CpuDeviceCount {
        devices: NonZeroUsize,
    },
    DeviceCountMismatch {
        requested: NonZeroUsize,
        available: NonZeroUsize,
    },
    NonemptyDirectory(PathBuf),
    Locked {
        path: PathBuf,
        source: io::Error,
    },
    ConfigMismatch(Vec<String>),
    InvalidSettings(&'static str),
    Collision,
    Capacity,
    InvalidAction,
    InvalidValue(f32),
    MissingSearch,
    UnexpectedGame(u32),
    IncompleteChunks,
    ReceiverClosed,
    Panic(Box<dyn Any + Send>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mcts(error) => write!(f, "MCTS search failed: {error}"),
            Self::Mate(error) => error.fmt(f),
            Self::Inference(error) => write!(f, "inference failed: {error}"),
            Self::GpuQuery(error) => write!(f, "cannot query GPU: {error}"),
            Self::Records(error) => write!(f, "MNSD record failed: {error}"),
            Self::Game(error) => write!(f, "game failed: {error}"),
            Self::Search(error) => write!(f, "cannot start search: {error}"),
            Self::Io(error) => error.fmt(f),
            Self::Json(error) => write!(f, "invalid JSON: {error}"),
            Self::CacheRootUnset => f.write_str("TensorRT requires nonempty XDG_CACHE_HOME"),
            Self::ThreadsNotMultipleOfDevices { threads, devices } => write!(
                f,
                "--threads {threads} must be a multiple of --devices {devices}"
            ),
            Self::CpuDeviceCount { devices } => {
                write!(f, "--nn-backend cpu requires --devices 1, got {devices}")
            }
            Self::DeviceCountMismatch {
                requested,
                available,
            } => write!(
                f,
                "--devices {requested} does not match the GPU count reported by nvidia-smi: {available}"
            ),
            Self::NonemptyDirectory(path) => write!(
                f,
                "output directory is not empty and has no config.json: {}",
                path.display()
            ),
            Self::Locked { path, source } => write!(
                f,
                "cannot lock output directory {}: {source}",
                path.display()
            ),
            Self::ConfigMismatch(fields) => {
                write!(f, "generation conditions differ: {}", fields.join(", "))
            }
            Self::InvalidSettings(field) => write!(f, "invalid generation setting: {field}"),
            Self::Collision => f.write_str("Gumbel search collided with an unfinished evaluation"),
            Self::Capacity => f.write_str("selfplay tree capacity exceeded; reduce --simulations"),
            Self::InvalidAction => f.write_str("Gumbel selected an absent root edge"),
            Self::InvalidValue(value) => write!(
                f,
                "selected root value is not finite and within [0, 1]: {value}"
            ),
            Self::MissingSearch => f.write_str("evaluation has no active search"),
            Self::UnexpectedGame(number) => {
                write!(f, "unexpected or duplicate completed game: {number}")
            }
            Self::IncompleteChunks => {
                f.write_str("workers finished before all chunks were committed")
            }
            Self::ReceiverClosed => f.write_str("completed-game receiver was closed"),
            Self::Panic(payload) => {
                f.write_str("selfplay worker panicked")?;
                if let Some(message) = payload.downcast_ref::<String>() {
                    write!(f, ": {message}")?;
                } else if let Some(message) = payload.downcast_ref::<&str>() {
                    write!(f, ": {message}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Mcts(error) => Some(error),
            Self::Mate(error) => Some(error),
            Self::Inference(error) => Some(error),
            Self::GpuQuery(error) => Some(error),
            Self::Records(error) => Some(error),
            Self::Game(error) => Some(error),
            Self::Search(error) => Some(error),
            Self::Io(error) | Self::Locked { source: error, .. } => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<mcts::Error> for Error {
    fn from(error: mcts::Error) -> Self {
        Self::Mcts(error)
    }
}
impl From<mate_in_three::Error> for Error {
    fn from(error: mate_in_three::Error) -> Self {
        Self::Mate(error)
    }
}
impl From<mcts::tree::TreeError> for Error {
    fn from(error: mcts::tree::TreeError) -> Self {
        Self::Mcts(error.into())
    }
}
impl From<NnError> for Error {
    fn from(error: NnError) -> Self {
        Self::Inference(error)
    }
}
impl From<GpuQueryError> for Error {
    fn from(error: GpuQueryError) -> Self {
        Self::GpuQuery(error)
    }
}
impl From<records::Error> for Error {
    fn from(error: records::Error) -> Self {
        Self::Records(error)
    }
}
impl From<GameError> for Error {
    fn from(error: GameError) -> Self {
        Self::Game(error)
    }
}
impl From<SearchError> for Error {
    fn from(error: SearchError) -> Self {
        Self::Search(error)
    }
}
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
