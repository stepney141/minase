//! SPSAで同一エンジンの探索係数と時間管理係数を調整する。

mod apply;
mod cli;
mod engine;
mod model;
mod params;
mod runner;
mod storage;
mod summary;

pub(super) use cli::Arguments;
use cli::Command;
use engine::{resolve_engine, restore_engine};
use minase::harness::*;
use minase_core::{Rules, rules::parse_rule_set};
use model::{ALPHA, GAMMA, Settings, validate_settings};
use params::{declarations, parse_parameters};
use std::{
    collections::BTreeMap,
    fs, io,
    time::{Duration, Instant},
};
use storage::{Manifest, Store};
use summary::{failure_text, parameter_line, print_theta};

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
            println!("{}", parameter_line(&d));
        }
        return Ok(());
    }
    minase::eval::weights()?;
    let (store, saved, player) = match (&arguments.run_dir, &arguments.resume) {
        (Some(path), None) => {
            let (manifest, player) = new_session(&arguments)?;
            (Store::create(path, &manifest)?, BTreeMap::new(), player)
        }
        (None, Some(path)) => {
            let (store, saved) = Store::resume(path)?;
            let player = restore_engine(&store.manifest)?;
            (store, saved, player)
        }
        _ => unreachable!("clap requires exactly one operation"),
    };
    let manifest = &store.manifest;
    let rules = Rules::from_codes(&parse_rule_set(&manifest.rules_source)?)?;
    let timeout = Duration::from_secs(manifest.response_timeout_secs);
    if (saved.len() as u64) < manifest.settings.iterations {
        store.begin_invocation()?;
    }
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

fn new_session(
    arguments: &Arguments,
) -> Result<(Manifest, PlayerConfig), Box<dyn std::error::Error>> {
    // 必須引数の存在はclapが検査済み。
    let rules_source = arguments.rules.clone().expect("required rules");
    let codes = parse_rule_set(&rules_source)?;
    Rules::from_codes(&codes)?;
    let each = arguments.each.expect("required each");
    let player = resolve_engine(
        arguments.engine.clone().expect("required engine"),
        each,
        &rules_source,
    )?;
    let timeout = Duration::from_secs(arguments.response_timeout);
    let parameters = parse_parameters(
        &fs::read_to_string(arguments.params.as_ref().expect("required params"))?,
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
    };
    Ok((manifest, player))
}

pub(super) fn main(arguments: Arguments) {
    if let Err(error) = execute(arguments) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests;
