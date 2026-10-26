use std::{
    collections::HashMap,
    env,
    error::Error,
    fs,
    io::{self, Write},
    path::Path,
    process::{self, Command},
    sync::Arc,
    time::{Duration, Instant},
};

use ort::{
    ep,
    logging::LogLevel,
    memory::{AllocationDevice, Allocator, AllocatorType, MemoryInfo, MemoryType},
    session::{IoBinding, Session},
    value::{Tensor, TensorRef},
};
use serde_json::json;

const INPUT_ELEMENTS: usize = 95 * 12 * 12;
const POLICY_ELEMENTS: i64 = 249 * 12 * 12;
const READS_PER_POSITION: usize = 81;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Provider {
    TensorRt,
    Cuda,
    Cpu,
}

impl Provider {
    fn name(self) -> &'static str {
        match self {
            Self::TensorRt => "tensorrt",
            Self::Cuda => "cuda",
            Self::Cpu => "cpu",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IoMode {
    Copy,
    Binding,
}

impl IoMode {
    fn name(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Binding => "binding",
        }
    }
}

struct Args {
    model: String,
    provider: Provider,
    io: IoMode,
    fp16: bool,
    batches: Vec<usize>,
    seconds: Duration,
    warmup: usize,
    cache_dir: String,
    detailed_build_log: bool,
}

impl Args {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let mut arguments = env::args().skip(1);
        let mut options = HashMap::new();
        while let Some(name) = arguments.next() {
            if !matches!(
                name.as_str(),
                "--model"
                    | "--provider"
                    | "--io"
                    | "--fp16"
                    | "--batches"
                    | "--seconds"
                    | "--warmup"
                    | "--cache-dir"
                    | "--detailed-build-log"
            ) {
                return Err(format!("unknown argument: {name}").into());
            }
            let value = arguments
                .next()
                .filter(|value| !value.is_empty() && !value.starts_with("--"))
                .ok_or_else(|| format!("{name} requires a value"))?;
            if options.insert(name.clone(), value).is_some() {
                return Err(format!("duplicate argument: {name}").into());
            }
        }
        let required = |name: &str| {
            options
                .get(name)
                .ok_or_else(|| format!("missing argument: {name}"))
        };
        let provider = match required("--provider")?.as_str() {
            "tensorrt" => Provider::TensorRt,
            "cuda" => Provider::Cuda,
            "cpu" => Provider::Cpu,
            value => return Err(format!("invalid --provider: {value}").into()),
        };
        let io = match required("--io")?.as_str() {
            "copy" => IoMode::Copy,
            "binding" => IoMode::Binding,
            value => return Err(format!("invalid --io: {value}").into()),
        };
        if provider == Provider::Cpu && io == IoMode::Binding {
            return Err("--io binding requires --provider cuda or tensorrt".into());
        }
        let fp16 = parse_switch("--fp16", required("--fp16")?)?;
        if fp16 && provider != Provider::TensorRt {
            return Err("--fp16 on requires --provider tensorrt".into());
        }
        let batches: Vec<usize> = required("--batches")?
            .split(',')
            .map(str::parse)
            .collect::<Result<_, _>>()?;
        if batches.contains(&0) {
            return Err("--batches must contain positive integers".into());
        }
        let seconds = Duration::try_from_secs_f64(required("--seconds")?.parse()?)?;
        if seconds.is_zero() {
            return Err("--seconds must be positive".into());
        }
        Ok(Self {
            model: required("--model")?.clone(),
            provider,
            io,
            fp16,
            batches,
            seconds,
            warmup: required("--warmup")?.parse()?,
            cache_dir: required("--cache-dir")?.clone(),
            detailed_build_log: parse_switch(
                "--detailed-build-log",
                required("--detailed-build-log")?,
            )?,
        })
    }
}

fn parse_switch(name: &str, value: &str) -> Result<bool, Box<dyn Error>> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("{name} must be on or off").into()),
    }
}

fn make_input(batch: usize) -> Result<Vec<f32>, Box<dyn Error>> {
    let length = batch
        .checked_mul(INPUT_ELEMENTS)
        .ok_or("--batches is too large")?;
    let mut data = Vec::new();
    data.try_reserve_exact(length)?;
    let mut state = 0x1234_5678_9abc_def0_u64;
    for _ in 0..length {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        data.push(if state.is_multiple_of(100) { 1.0 } else { 0.0 });
    }
    Ok(data)
}

fn make_policy_indices() -> [usize; READS_PER_POSITION] {
    let mut state = 0x1234_5678_9abc_def0_u64;
    std::array::from_fn(|_| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % POLICY_ELEMENTS as u64) as usize
    })
}

fn create_session(args: &Args, batch: usize) -> Result<Session, Box<dyn Error>> {
    let mut builder = Session::builder()?.with_log_level(if args.detailed_build_log {
        LogLevel::Verbose
    } else {
        LogLevel::Warning
    })?;
    if args.detailed_build_log {
        builder = builder.with_log_verbosity(1)?;
    }
    if args.provider != Provider::Cpu {
        builder = builder.with_disable_cpu_fallback()?;
    }
    let provider = match args.provider {
        Provider::TensorRt => {
            let profile = format!("input:{batch}x95x12x12");
            ep::TensorRT::default()
                .with_profile_min_shapes(&profile)
                .with_profile_opt_shapes(&profile)
                .with_profile_max_shapes(&profile)
                .with_fp16(args.fp16)
                .with_engine_cache(true)
                .with_engine_cache_path(&args.cache_dir)
                .with_timing_cache(true)
                .with_timing_cache_path(&args.cache_dir)
                .with_detailed_build_log(args.detailed_build_log)
                .build()
        }
        Provider::Cuda => ep::CUDA::default().build(),
        Provider::Cpu => ep::CPU::default().build(),
    };
    Ok(builder
        .with_execution_providers([provider.error_on_failure()])?
        .commit_from_file(&args.model)?)
}

enum InferenceIo<'a> {
    Copy(TensorRef<'a, f32>),
    Binding {
        binding: Box<IoBinding>,
        input: Tensor<f32>,
        // 束縛したテンソルを解放するまで、そのアロケータを保持する。
        _allocator: Allocator,
    },
}

impl<'a> InferenceIo<'a> {
    fn new(
        mode: IoMode,
        session: &Session,
        batch: usize,
        input_data: &'a [f32],
    ) -> Result<Self, Box<dyn Error>> {
        match mode {
            IoMode::Copy => Ok(Self::Copy(TensorRef::from_array_view((
                [batch, 95, 12, 12],
                input_data,
            ))?)),
            IoMode::Binding => {
                // 入力は CUDA のホスト側ピン留めメモリに置く。
                let allocator = Allocator::new(
                    session,
                    MemoryInfo::new(
                        AllocationDevice::CUDA_PINNED,
                        0,
                        AllocatorType::Device,
                        MemoryType::CPUInput,
                    )?,
                )?;
                let mut input = Tensor::<f32>::new(&allocator, [batch, 95, 12, 12])?;
                input
                    .try_extract_tensor_mut::<f32>()?
                    .1
                    .copy_from_slice(input_data);
                let mut binding = session.create_binding()?;
                let batch = i64::try_from(batch)?;
                // 出力は作り置きのページング可能なCPUメモリへ束縛する。ピン留めメモリへ束縛すると、
                // ONNX Runtime 1.28.3 は同じバッファへの複写として拒否する（gpu_data_transfer.cc 104行）。
                binding.bind_output(
                    "policy",
                    Tensor::<f32>::new(&Allocator::default(), [batch, POLICY_ELEMENTS])?,
                )?;
                binding.bind_output(
                    "value",
                    Tensor::<f32>::new(&Allocator::default(), [batch, 1])?,
                )?;
                Ok(Self::Binding {
                    binding: Box::new(binding),
                    input,
                    _allocator: allocator,
                })
            }
        }
    }
}

fn run_once(
    session: &mut Session,
    io: &mut InferenceIo<'_>,
    batch: usize,
    policy_indices: &[usize; READS_PER_POSITION],
    check_all: bool,
) -> Result<(), Box<dyn Error>> {
    let outputs = match io {
        InferenceIo::Copy(input) => session.run(ort::inputs!["input" => input.view()])?,
        InferenceIo::Binding { binding, input, .. } => {
            // 毎回入力を再束縛し、GPU への転送と同期を計測に含める。
            binding.bind_input("input", input)?;
            binding.synchronize_inputs()?;
            let outputs = session.run_binding(binding)?;
            // CPU で出力を読む前に転送完了を待ち、この同期も計測に含める。
            binding.synchronize_outputs()?;
            outputs
        }
    };
    if outputs.len() != 2 {
        return Err(format!("expected 2 outputs, got {}", outputs.len()).into());
    }
    let validate_output = |name: &str, width: i64| -> Result<&[f32], Box<dyn Error>> {
        let output = outputs
            .get(name)
            .ok_or_else(|| format!("missing output: {name}"))?;
        let (shape, values) = output.try_extract_tensor::<f32>()?;
        let expected = [i64::try_from(batch)?, width];
        if **shape != expected {
            return Err(format!("{name} shape is {shape:?}, expected {expected:?}").into());
        }
        if check_all && values.iter().any(|value| !value.is_finite()) {
            return Err(format!("{name} contains a non-finite value").into());
        }
        Ok(values)
    };
    let policy = validate_output("policy", POLICY_ELEMENTS)?;
    let value = validate_output("value", 1)?;
    if !check_all {
        // 計測区間では本番の探索と同様に、方策は平均合法手数に相当する81要素だけを各局面で読む。
        let mut sum = 0.0_f32;
        for (policy, &value) in policy
            .as_chunks::<{ POLICY_ELEMENTS as usize }>()
            .0
            .iter()
            .zip(value)
        {
            for &index in policy_indices {
                sum += policy[index];
            }
            sum += value;
        }
        if !std::hint::black_box(sum).is_finite() {
            return Err("sampled outputs have a non-finite sum".into());
        }
    }
    Ok(())
}

fn gpu_memory_mib() -> Result<u64, Box<dyn Error>> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-compute-apps=pid,used_memory",
            "--format=csv,noheader,nounits",
        ])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "nvidia-smi failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let mut found = false;
    let mut memory = 0_u64;
    for line in String::from_utf8(output.stdout)?.lines() {
        let (pid, used_memory) = line
            .split_once(',')
            .ok_or_else(|| format!("invalid nvidia-smi row: {line}"))?;
        if pid.trim().parse::<u32>()? == process::id() {
            memory = memory
                .checked_add(used_memory.trim().parse::<u64>()?)
                .ok_or("GPU memory total is too large")?;
            found = true;
        }
    }
    if !found {
        return Err(format!("nvidia-smi did not report PID {}", process::id()).into());
    }
    Ok(memory)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse()?;
    if !Path::new(&args.model).is_file() {
        return Err(format!("model file does not exist: {}", args.model).into());
    }
    let ort_path = env::var("ORT_DYLIB_PATH")?;
    if !Path::new(&ort_path).is_file() {
        return Err(format!("ORT_DYLIB_PATH is not a file: {ort_path}").into());
    }
    if args.provider == Provider::TensorRt {
        fs::create_dir_all(&args.cache_dir)?;
    }
    let log_level = if args.detailed_build_log {
        LogLevel::Verbose
    } else {
        LogLevel::Warning
    };
    if !ort::init()
        .with_logger(Arc::new(move |level, _category, _id, location, message| {
            if level >= log_level {
                eprintln!("[{level:?}] {location}: {message}");
            }
        }))
        .commit()
    {
        return Err("ONNX Runtime environment was already configured".into());
    }

    let max_batch = *args.batches.iter().max().ok_or("--batches is empty")?;
    let input_data = make_input(max_batch)?;
    let policy_indices = make_policy_indices();
    for &batch in &args.batches {
        let start = Instant::now();
        let mut session = create_session(&args, batch)?;
        let session_create_secs = start.elapsed().as_secs_f64();
        let mut inference_io = InferenceIo::new(
            args.io,
            &session,
            batch,
            &input_data[..batch * INPUT_ELEMENTS],
        )?;

        let start = Instant::now();
        run_once(
            &mut session,
            &mut inference_io,
            batch,
            &policy_indices,
            true,
        )?;
        let first_run_secs = start.elapsed().as_secs_f64();
        for _ in 0..args.warmup {
            run_once(
                &mut session,
                &mut inference_io,
                batch,
                &policy_indices,
                true,
            )?;
        }

        let mut latencies = Vec::new();
        let start = Instant::now();
        loop {
            let run_start = Instant::now();
            run_once(
                &mut session,
                &mut inference_io,
                batch,
                &policy_indices,
                false,
            )?;
            latencies.push(run_start.elapsed().as_secs_f64() * 1000.0);
            if start.elapsed() >= args.seconds {
                break;
            }
        }
        let elapsed_secs = start.elapsed().as_secs_f64();
        let gpu_mem_mib = if args.provider == Provider::Cpu {
            None
        } else {
            Some(gpu_memory_mib()?)
        };

        let runs = latencies.len();
        let mean = latencies.iter().sum::<f64>() / runs as f64;
        latencies.sort_unstable_by(f64::total_cmp);
        // 分位点は昇順の ceil(p × 件数) 番目を使う。
        let percentile = |p: f64| latencies[(p * runs as f64).ceil() as usize - 1];
        let record = json!({
            "model": args.model,
            "provider": args.provider.name(),
            "io": args.io.name(),
            "fp16": args.fp16,
            "batch": batch,
            "reads_per_position": READS_PER_POSITION,
            "session_create_secs": session_create_secs,
            "first_run_secs": first_run_secs,
            "runs": runs,
            "elapsed_secs": elapsed_secs,
            "evals_per_sec": batch as f64 * runs as f64 / elapsed_secs,
            "latency_ms": {"mean": mean, "p50": percentile(0.5), "p95": percentile(0.95)},
            "gpu_mem_mib": gpu_mem_mib,
        });
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{record}")?;
        stdout.flush()?;
    }
    Ok(())
}
