use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::PathBuf,
};

use nnue_trainer::data::{
    BatchSlot, Dataset, FEATURE_COLUMNS, InputFeatureSet, Lookahead, ReverseIndex, SENTINEL,
    build_batches, decode_p_feature,
};
use nnue_trainer::{
    data::Observations,
    model::{Checkpoint, Masks, Model, Parameter, Parameters, Progress, Shape, load_initial},
    reference::{AdamW, AdamWConfig, Losses, RemovalReference, loss_and_gradients},
};
use serde::Serialize;
use sha2::{Digest, Sha256};

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
    /// Initialize a model from J75's stored (uncompressed) pst-float.npz.
    #[arg(long, requires_all = ["width", "k", "seed", "steps", "reference_batches"])]
    init_pst: Option<PathBuf>,
    /// Even hidden width, 2..=512. Requires --init-pst.
    #[arg(long, requires = "init_pst")]
    width: Option<usize>,
    /// Output K (from the base MNPT), independent of the estimated teacher K.
    #[arg(long, requires = "init_pst")]
    k: Option<f32>,
    /// SplitMix64 model initialization seed.
    #[arg(long, requires = "init_pst")]
    seed: Option<u64>,
    /// Standard deviation of first-layer initialization.
    #[arg(long, default_value_t = 0.05)]
    init_std: f32,
    /// Scale applied to the initial PST (0 makes the removal lower-bound penalty fire everywhere).
    #[arg(long, default_value_t = 1.0, requires = "init_pst")]
    init_pst_scale: f32,
    /// Warm-up updates on --indices before saving checkpoint.bin; 0 also allowed.
    #[arg(long, requires = "init_pst")]
    steps: Option<u64>,
    /// One comma-separated batch of global indices. Repeat exactly 3 times.
    #[arg(long = "reference-batch", requires = "init_pst", action = clap::ArgAction::Append)]
    reference_batches: Vec<String>,
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
    if args.init_pst.is_some() {
        dump_reference(&args, &dataset, &slots[0], lookahead)?;
    }
    Ok(())
}

#[derive(Serialize)]
struct ReferenceStep {
    batch: String,
    gradients: String,
    after: String,
    losses: Losses,
}

#[derive(Serialize)]
struct ReferenceDump {
    format: &'static str,
    version: u32,
    checkpoint: &'static str,
    teacher_ks: Vec<f64>,
    steps: Vec<ReferenceStep>,
}

fn dump_reference(
    args: &Args,
    dataset: &Dataset,
    warmup: &BatchSlot,
    lookahead: Option<Lookahead>,
) -> Result<(), Box<dyn std::error::Error>> {
    let invalid = |message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message);
    if args.reference_batches.len() != 3 {
        return Err(invalid("--init-pst requires exactly 3 --reference-batch arguments").into());
    }
    let indices: Vec<Vec<u64>> = args
        .reference_batches
        .iter()
        .map(|s| s.split(',').map(str::parse).collect())
        .collect::<Result<_, _>>()?;
    let batch_slices: Vec<_> = indices.iter().map(Vec::as_slice).collect();
    let batches = build_batches(dataset, args.feature_set, &batch_slices)?;
    let width = args.width.ok_or_else(|| invalid("--width is required"))?;
    let k = args.k.ok_or_else(|| invalid("--k is required"))?;
    let seed = args.seed.ok_or_else(|| invalid("--seed is required"))?;
    let steps = args.steps.ok_or_else(|| invalid("--steps is required"))?;
    let initial_path = args
        .init_pst
        .as_ref()
        .ok_or_else(|| invalid("--init-pst is required"))?;
    let mut initial = load_initial(initial_path)?;
    if !args.init_pst_scale.is_finite() {
        return Err(invalid("--init-pst-scale must be finite").into());
    }
    for pair in &mut initial {
        pair[0] *= args.init_pst_scale;
        pair[1] *= args.init_pst_scale;
    }
    let observations = Observations::compute(dataset, args.feature_set)?;
    let shape = Shape::new(args.feature_set, width, k)?;
    let model = Model::initialize(
        shape,
        &initial,
        Masks::from_observations(args.feature_set, &observations)?,
        seed,
        args.init_std,
    )?;
    let optimizer = AdamW::new(shape, AdamWConfig::default())?;
    let teacher_ks = dataset.estimate_teacher_ks()?;
    let mut input_hash = Sha256::new();
    for header in dataset.headers() {
        input_hash.update(header.encode());
    }
    // MNSDの内容はDataset::openが来歴のSHAと照合済み。ハッシュ列も設定に含める。
    for path in dataset.paths() {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(".provenance.json");
        input_hash.update(Sha256::digest(fs::read(PathBuf::from(sidecar))?));
    }
    input_hash.update(Sha256::digest(fs::read(initial_path)?));
    let config = serde_json::to_vec(&(
        shape.architecture(),
        k,
        seed,
        lookahead,
        args.init_std,
        args.init_pst_scale,
        optimizer.config(),
        &teacher_ks,
    ))?;
    let mut checkpoint = Checkpoint {
        model,
        optimizer,
        progress: Progress {
            epoch: 0,
            best: None,
            elapsed_seconds: 0.0,
            seed,
            config_hash: Sha256::digest(config).into(),
            input_hash: input_hash.finalize().into(),
        },
    };
    let reference = RemovalReference::v0();
    let started = std::time::Instant::now();
    for _ in 0..steps {
        let result = loss_and_gradients(&checkpoint.model, warmup, &teacher_ks, &reference)?;
        checkpoint
            .optimizer
            .update(&mut checkpoint.model, &result.parameters)?;
    }
    checkpoint.progress.elapsed_seconds = started.elapsed().as_secs_f64();
    write_new(&args.output.join("checkpoint.bin"), &checkpoint.encode()?)?;
    let mut records = Vec::new();
    for (step, batch) in batches.iter().enumerate() {
        let batch_name = format!("reference-batch-{}.json", step + 1);
        let gradients = format!("gradients-{}.f32", step + 1);
        let after = format!("after-{}.bin", step + 1);
        write_json(
            &args.output.join(&batch_name),
            &payload(dataset, batch, lookahead)?,
        )?;
        let result = loss_and_gradients(&checkpoint.model, batch, &teacher_ks, &reference)?;
        write_gradients(&args.output.join(&gradients), &result.parameters)?;
        checkpoint
            .optimizer
            .update(&mut checkpoint.model, &result.parameters)?;
        checkpoint.progress.elapsed_seconds = started.elapsed().as_secs_f64();
        write_new(&args.output.join(&after), &checkpoint.encode()?)?;
        records.push(ReferenceStep {
            batch: batch_name,
            gradients,
            after,
            losses: result.losses,
        });
    }
    let path = args.output.join("reference.json");
    write_json(
        &path,
        &ReferenceDump {
            format: "minase-nnue-reference",
            version: 1,
            checkpoint: "checkpoint.bin",
            teacher_ks,
            steps: records,
        },
    )?;
    println!("{}", path.display());
    Ok(())
}

fn write_new(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = File::create_new(path)?;
    file.write_all(bytes)
}
fn write_json(
    path: &std::path::Path,
    value: &impl Serialize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = BufWriter::new(File::create_new(path)?);
    serde_json::to_writer_pretty(&mut file, value)?;
    writeln!(file)?;
    file.flush()?;
    Ok(())
}
fn write_gradients(path: &std::path::Path, values: &Parameters) -> std::io::Result<()> {
    let mut file = BufWriter::new(File::create_new(path)?);
    for p in Parameter::ALL {
        for v in &values[p] {
            file.write_all(&v.to_le_bytes())?;
        }
    }
    file.flush()
}
