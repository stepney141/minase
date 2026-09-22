//! SPSAで同一エンジンの探索係数と時間管理係数を調整する。

use clap::{ArgGroup, Parser, Subcommand};
use minase::{Rules, core::rules::parse_rule_set, harness::*};
use std::{
    collections::BTreeMap,
    fs, io,
    path::PathBuf,
    time::{Duration, Instant},
};

#[path = "spsa_runner/apply.rs"]
mod apply;
#[path = "spsa_runner/model.rs"]
mod model;
#[path = "spsa_runner/params.rs"]
mod params;
#[path = "spsa_runner/runner.rs"]
mod runner;
#[path = "spsa_runner/storage.rs"]
mod storage;
use model::{ALPHA, GAMMA, Settings};
use params::{declarations, parse_parameters};
use storage::{Manifest, Store};

#[derive(Parser)]
#[command(name = "spsa_runner", subcommand_negates_reqs = true, args_conflicts_with_subcommands = true,
    group(ArgGroup::new("operation").required(true).args(["run_dir", "resume"])))]
struct Arguments {
    /// 新規に作る実行ディレクトリ。
    #[arg(long)]
    run_dir: Option<PathBuf>,
    /// 同じ実行条件で再開するディレクトリ。
    #[arg(long)]
    resume: Option<PathBuf>,
    /// 全対局と摂動の基本シード。
    #[arg(long, required = true)]
    seed: Option<u64>,
    /// tuningビルドのcommit指定または起動コマンド。
    #[arg(long, required = true, value_parser = tuning_spec)]
    engine: Option<PlayerSpec>,
    /// 6欄の係数ファイル。
    #[arg(long, required = true)]
    params: Option<PathBuf>,
    /// 両エンジンと審判に適用する規則。
    #[arg(long, required = true)]
    rules: Option<String>,
    /// 両エンジンの思考制限。
    #[arg(long, required = true, value_parser = parse_search_limit)]
    each: Option<SearchLimit>,
    /// 同時に対局させるペアの数。
    #[arg(long, required = true)]
    concurrency: Option<std::num::NonZeroUsize>,
    /// 開始時に固定する総反復数。
    #[arg(long, required = true, value_parser = parse_positive_u64)]
    iterations: Option<u64>,
    /// 1反復のペア数。
    #[arg(long, required = true)]
    pairs_per_iteration: Option<std::num::NonZeroUsize>,
    /// 1局の手数上限。
    #[arg(long, default_value = "4096", value_parser = parse_positive_u32)]
    max_ply: u32,
    /// 1回の応答期限を秒で指定する。
    #[arg(long, default_value = "120", value_parser = parse_positive_u64)]
    response_timeout: u64,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 完了した調整結果を係数表の既定値へ反映する。
    Apply {
        /// 完了したセッションの実行ディレクトリ。
        #[arg(long)]
        run_dir: PathBuf,
        /// 書き換える係数表のソースファイル。
        #[arg(long)]
        source: PathBuf,
    },
    /// USIのTune_宣言から係数ファイルを標準出力へ生成する。
    Params {
        #[arg(long, value_parser = tuning_spec)]
        engine: PlayerSpec,
        #[arg(long)]
        rules: String,
    },
}

fn tuning_spec(text: &str) -> Result<PlayerSpec, String> {
    let spec = parse_player_spec(text)?;
    if matches!(spec.kind, PlayerKind::Random | PlayerKind::Cecp { .. }) {
        return Err(
            "SPSA requires a USI tuning engine; random and cecp: are not supported".to_owned(),
        );
    }
    Ok(spec)
}

fn resolve_engine(spec: PlayerSpec, each: SearchLimit, rules: &str) -> io::Result<PlayerConfig> {
    if let PlayerKind::Commit(revision) = &spec.kind {
        let (path, hash, sha256) = resolve_commit(revision, Some("tuning"))?;
        Ok(PlayerConfig {
            text: spec.text,
            identity: EngineIdentity::Commit { hash, sha256 },
            path,
            args: vec![
                "--protocol".to_owned(),
                "usi".to_owned(),
                "--rules".to_owned(),
                rules.to_owned(),
            ],
            protocol: Protocol::Usi,
            is_random: false,
            limit: each,
            hash_mb: None,
            rules_source: rules.to_owned(),
            options: Vec::new(),
        })
    } else {
        let mut player = resolve_player(spec, each, None, rules, Vec::new())?;
        // 裸のコマンド名は起動時と同じPATHで解決し、ハッシュ対象を固定する。
        if player.path.components().count() == 1 {
            let path = std::env::var_os("PATH")
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "PATH is not set"))?;
            player.path = std::env::split_paths(&path)
                .map(|p| p.join(&player.path))
                .find(|p| p.is_file())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "engine command is not in PATH")
                })?;
        }
        player.path = fs::canonicalize(&player.path)?;
        Ok(player)
    }
}

fn validate_settings(settings: &Settings) -> io::Result<()> {
    let total = settings
        .iterations
        .checked_mul(settings.pairs_per_iteration as u64)
        .and_then(|n| n.checked_mul(2));
    if settings.iterations == 0
        || settings.iterations == u64::MAX
        || settings.pairs_per_iteration == 0
        || settings.concurrency == 0
        || total.is_none_or(|n| n > i64::MAX as u64)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "session size is zero or exceeds supported counters",
        ));
    }
    for p in &settings.parameters {
        for k in [1, settings.iterations] {
            let (c, a) = model::rates(settings, p, k);
            if !c.is_finite()
                || c <= 0.0
                || !a.is_finite()
                || a <= 0.0
                || !(a / c).is_finite()
                || a / c <= 0.0
                || !((a / c) * (2.0 * settings.pairs_per_iteration as f64)).is_finite()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("non-finite or underflowing SPSA rates: {}", p.name),
                ));
            }
        }
    }
    Ok(())
}

fn print_theta(settings: &Settings, theta: &[f64]) {
    for (p, value) in settings.parameters.iter().zip(theta) {
        println!("{} = {value}", p.name);
    }
}

fn execute(arguments: Arguments) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(Command::Apply { run_dir, source }) = &arguments.command {
        let report = apply::apply(run_dir, source)?;
        use std::io::Write;
        io::stdout()
            .lock()
            .write_all(report.as_bytes())
            .map_err(apply::ApplyError::AfterWrite)?;
        return Ok(());
    }
    if let Some(Command::Params { engine, rules }) = arguments.command {
        Rules::from_codes(&parse_rule_set(&rules)?)?;
        let player = resolve_engine(
            engine,
            SearchLimit::Fixed {
                depth: Some(1),
                nodes: None,
            },
            &rules,
        )?;
        for d in declarations(&probe_usi_options(&player, Duration::from_secs(120))?)? {
            println!(
                "{}, {}, {}, {}, {}, 0.002",
                d.name,
                d.default,
                d.min,
                d.max,
                (f64::from(d.max) - f64::from(d.min)) / 20.0
            );
        }
        return Ok(());
    }
    minase::eval::weights()?;
    // 必須引数の存在はclapが検査済み。
    let rules_source = arguments.rules.expect("required rules");
    let codes = parse_rule_set(&rules_source)?;
    let rules = Rules::from_codes(&codes)?;
    let each = arguments.each.expect("required each");
    let player = resolve_engine(
        arguments.engine.expect("required engine"),
        each,
        &rules_source,
    )?;
    let timeout = Duration::from_secs(arguments.response_timeout);
    let parameters = parse_parameters(
        &fs::read_to_string(arguments.params.expect("required params"))?,
        &declarations(&probe_usi_options(&player, timeout)?)?,
    )?;
    let iterations = arguments.iterations.expect("required iterations");
    let settings = Settings {
        parameters,
        alpha: ALPHA,
        gamma: GAMMA,
        a: 0.1 * iterations as f64,
        iterations,
        pairs_per_iteration: arguments.pairs_per_iteration.expect("required pairs").get(),
        seed: arguments.seed.expect("required seed"),
        concurrency: arguments.concurrency.expect("required concurrency").get(),
    };
    validate_settings(&settings)?;
    let manifest = Manifest {
        engine: player.identity.clone(),
        engine_sha256: sha256_file(&player.path)?,
        settings,
        each: each.cli_text(),
        rules_source,
        canonical_rules: codes.iter().map(ToString::to_string).collect(),
        max_ply: arguments.max_ply,
        response_timeout_secs: arguments.response_timeout,
        cpu: CpuRecord {
            model: cpu_model(),
            physical_cores: physical_core_count(),
            logical_cores: std::thread::available_parallelism()?.get(),
            physical_memory_bytes: physical_memory_bytes(),
        },
        runner: harness_record()?,
    };
    let (store, saved) = match (&arguments.run_dir, &arguments.resume) {
        (Some(path), None) => (Store::create(path, &manifest)?, BTreeMap::new()),
        (None, Some(path)) => Store::resume(path, &manifest)?,
        _ => unreachable!("clap requires exactly one operation"),
    };
    println!("session {}", serde_json::to_string(&manifest)?);
    let start = Instant::now();
    let settings = &manifest.settings;
    let summary = runner::run(
        settings,
        &saved,
        |values, stop| {
            let configure = |integers: &[i32]| {
                let mut configured = player.clone();
                configured.options = settings
                    .parameters
                    .iter()
                    .zip(integers)
                    .map(|(p, value)| (format!("Tune_{}", p.name), value.to_string()))
                    .collect();
                configured
            };
            Ok(run_pair(
                rules,
                &manifest.rules_source,
                settings.seed,
                values.number,
                manifest.max_ply,
                &configure(&values.plus),
                &configure(&values.minus),
                timeout,
                false,
                stop,
            ))
        },
        |record| {
            store.save(record)?;
            println!(
                "iteration {} applied {}: D={} {}",
                record.k,
                record.application,
                record.d,
                failure_text(record.failures)
            );
            if record.application % 100 == 0 {
                print_theta(settings, &record.theta);
            }
            Ok(())
        },
    )?;
    print_theta(settings, &summary.theta);
    for (p, value) in settings.parameters.iter().zip(&summary.theta) {
        println!(
            "{}: start={} start_integer={} final={} final_integer={}",
            p.name,
            p.start,
            p.start.round() as i32,
            value,
            value.round() as i32
        );
    }
    println!(
        "summary: applied={} valid_pairs={} discarded_pairs={}",
        summary.applied, summary.valid_pairs, summary.discarded_pairs
    );
    println!("engine_failures: {}", failure_text(summary.failures));
    println!("elapsed: {:.6} s", start.elapsed().as_secs_f64());
    Ok(())
}

/// 異常の理由別件数を`match_runner`の最終サマリと同じ書式で表す。
fn failure_text(failures: FailureCounts) -> String {
    format!(
        "illegal_moves={} crashes={} timeouts={} time_forfeits={} rejected_moves={}",
        failures.illegal_moves,
        failures.crashes,
        failures.timeouts,
        failures.time_forfeits,
        failures.rejected_moves
    )
}

fn main() {
    if let Err(error) = execute(Arguments::parse()) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
#[path = "spsa_runner/tests.rs"]
mod tests;
