use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::PathBuf,
};

use nnue_trainer::data::{
    BatchSlot, Dataset, FEATURE_COLUMNS, InputFeatureSet, Lookahead, ReverseIndex, SENTINEL,
    build_batches, decode_p_feature,
};
use serde::Serialize;

#[derive(clap::Args)]
pub(crate) struct Args {
    /// Input feature set.
    #[arg(long, value_enum)]
    feature_set: InputFeatureSet,
    /// MNSD files, in global-record order.
    #[arg(long, required = true, num_args = 1..)]
    data: Vec<PathBuf>,
    /// Comma-separated zero-based global record indices.
    #[arg(long, required = true, value_delimiter = ',')]
    indices: Vec<u64>,
    /// Directory in which to write batch.json.
    #[arg(long)]
    output: PathBuf,
    /// Lookahead discount; requires --lookahead-plies. Omit both for original scores.
    #[arg(long, requires = "lookahead_plies")]
    lookahead_gamma: Option<f64>,
    /// Lookahead window in plies (1..=4096); requires --lookahead-gamma.
    #[arg(long, requires = "lookahead_gamma")]
    lookahead_plies: Option<u16>,
}

#[derive(Serialize)]
struct Dump<'a> {
    format: &'static str,
    version: u32,
    feature_set: InputFeatureSet,
    data: Vec<PathBuf>,
    record_indices: &'a [u64],
    lookahead: Option<Lookahead>,
    sentinel: u32,
    input_features: Vec<Vec<&'a [u32]>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_features: Option<Vec<Vec<Vec<u32>>>>,
    linear_features: Vec<&'a [u32]>,
    piece_counts: &'a [u8],
    teacher_scores: &'a [f32],
    teacher_classes: &'a [u32],
    outcomes: &'a [u8],
    first_layer_inverse: &'a ReverseIndex,
    linear_inverse: &'a ReverseIndex,
}

fn payload<'a>(
    dataset: &Dataset,
    slot: &'a BatchSlot,
    lookahead: Option<Lookahead>,
) -> Result<Dump<'a>, nnue_trainer::data::Error> {
    let input_features: Vec<Vec<&[u32]>> = slot
        .features()
        .as_chunks::<{ 2 * FEATURE_COLUMNS }>()
        .0
        .iter()
        .map(|views| {
            views
                .as_chunks::<FEATURE_COLUMNS>()
                .0
                .iter()
                .map(|columns| columns.as_slice())
                .collect()
        })
        .collect();
    let base_features = if slot.feature_set() == InputFeatureSet::P {
        Some(
            input_features
                .iter()
                .map(|views| {
                    views
                        .iter()
                        .map(|columns| {
                            columns
                                .iter()
                                .map(|&feature| decode_p_feature(feature))
                                .collect()
                        })
                        .collect::<nnue_trainer::data::Result<Vec<_>>>()
                })
                .collect::<nnue_trainer::data::Result<Vec<_>>>()?,
        )
    } else {
        None
    };
    Ok(Dump {
        format: "minase-nnue-batch-indices",
        version: 1,
        feature_set: slot.feature_set(),
        data: dataset.paths().map(|path| path.to_owned()).collect(),
        record_indices: slot.record_indices(),
        lookahead,
        sentinel: SENTINEL,
        input_features,
        base_features,
        linear_features: slot
            .linear_features()
            .as_chunks::<FEATURE_COLUMNS>()
            .0
            .iter()
            .map(|columns| columns.as_slice())
            .collect(),
        piece_counts: slot.piece_counts(),
        teacher_scores: slot.teacher_scores(),
        teacher_classes: slot.teacher_classes(),
        outcomes: slot.outcomes(),
        first_layer_inverse: slot.first_layer_inverse(),
        linear_inverse: slot.linear_inverse(),
    })
}

pub(crate) fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let lookahead = match (args.lookahead_gamma, args.lookahead_plies) {
        (Some(gamma), Some(plies)) => Some(Lookahead::new(gamma, plies)?),
        (None, None) => None,
        _ => return Err(nnue_trainer::data::Error::LookaheadParameters.into()),
    };
    let dataset = Dataset::open(&args.data, lookahead)?;
    let slots = build_batches(&dataset, args.feature_set, &[&args.indices])?;
    let dump = payload(&dataset, &slots[0], lookahead)?;
    fs::create_dir_all(&args.output)?;
    let path = args.output.join("batch.json");
    let mut output = BufWriter::new(File::create(&path)?);
    serde_json::to_writer_pretty(&mut output, &dump)?;
    writeln!(output)?;
    output.flush()?;
    println!("{}", path.display());
    Ok(())
}
