//! 学習と診断の契約違反を、表示文言から独立して識別する。

/// ゼロ勾配を検査するパラメータ群。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientGroup {
    /// PST端点。
    Pst,
    /// FM埋め込み。
    Embedding,
    /// FM係数。
    Coefficient,
}
impl std::fmt::Display for GradientGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Pst => "pst",
            Self::Embedding => "embedding",
            Self::Coefficient => "a",
        })
    }
}
/// 比較対象の重み。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeightRole {
    /// 基準。
    Base,
    /// 候補。
    Candidate,
}
impl std::fmt::Display for WeightRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Base => "base",
            Self::Candidate => "candidate",
        })
    }
}
/// 学習と診断の拒否理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContractError {
    /// 新規実行の出力ディレクトリに既存の成果物がある。
    OutputNotEmpty,
    /// 実数PSTの量子化結果と入力MNPTの端点が異なる。
    FloatingPstMismatch,
    /// 実数FMの量子化結果と入力MNPTのFMが異なる。
    FloatingFmMismatch,
    /// 診断局面の規則がエンジンの既定規則と異なる。
    NondefaultRules,
    /// 着手診断の入力局面が空である。
    EmptyPositions,
    /// 駒除去の参照PSTが鏡映対称でない。
    AsymmetricReference,
    /// 量子化の平均絶対誤差が2センチポーンを超えた。
    Quantization,
    /// 5更新続けて指定群の損失勾配が0だった。
    ZeroGradient(GradientGroup),
    /// PST専用の比較にFMを含む重みを指定した。
    FmWeights(WeightRole),
    /// 引数の値域違反。
    InvalidArgument(Argument),
}
/// 値域検査の対象。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Argument {
    /// `Data` 引数。
    Data,
    /// `Epochs` 引数。
    Epochs,
    /// `Patience` 引数。
    Patience,
    /// `Batch` 引数。
    Batch,
    /// `ValidationSample` 引数。
    ValidationSample,
    /// `Threads` 引数。
    Threads,
    /// `LambdaRes` 引数。
    LambdaRes,
    /// `RemovalPenalty` 引数。
    RemovalPenalty,
}
impl std::fmt::Display for Argument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Data => "--data requires at least one file",
            Self::Epochs => "--epochs must be in 1..200",
            Self::Patience => "--patience must be positive",
            Self::Batch => "FM batch must be nonempty and fit in a feature array",
            Self::ValidationSample => "--validation-sample must be positive",
            Self::Threads => "--threads must be positive",
            Self::LambdaRes => "--lambda-res must be finite and nonnegative",
            Self::RemovalPenalty => "--removal-penalty must be finite and nonnegative",
        })
    }
}
impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutputNotEmpty => {
                f.write_str("output directory is not empty; use --resume to continue a run")
            }
            Self::FloatingPstMismatch => {
                f.write_str("floating PST does not quantize to the input MNPT endpoints")
            }
            Self::FloatingFmMismatch => {
                f.write_str("floating FM does not quantize to the input MNPT FM")
            }
            Self::NondefaultRules => {
                f.write_str("diagnostic positions must use engine-default rules")
            }
            Self::EmptyPositions => f.write_str("move probe positions must be nonempty"),
            Self::AsymmetricReference => f.write_str("removal reference mirror sharing"),
            Self::Quantization => {
                f.write_str("quantization_error: mean absolute error exceeds 2 centipawns")
            }
            Self::ZeroGradient(group) => {
                write!(f, "{group} loss gradient stayed zero for 5 steps")
            }
            Self::FmWeights(role) => write!(
                f,
                "--{role}: PST comparison diagnostics require PST-only weights (FM rank 0); remove FM with `minase-trainer fm scale --fm-scale 0` before comparing."
            ),
            Self::InvalidArgument(argument) => argument.fmt(f),
        }
    }
}
impl std::error::Error for ContractError {}
