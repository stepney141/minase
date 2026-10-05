//! コミット対コミットの自己対局測定ハーネス。

mod cli;
mod manifest;
mod replay;
mod resume;
mod scheduler;
mod storage;
mod summary;

use clap::error::ErrorKind;
pub(super) use cli::Arguments;
use cli::{Mode, time_seed};
use manifest::run_manifest;
use minase::harness::*;
use minase::stats::{GSPRT_H1_ELO, GsprtDecision};
use minase_core::Rules;
use replay::{completed_pair_from_record, ponder_summary};
use scheduler::{accept_completed_pair, run_worker_loop};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use storage::{ManifestMode, RunStore};
use summary::{integrate_pair_statistics, print_elo_summary, print_gsprt_summary};

/// 引数を検証し、ワーカープールでペア対局を実行して集計を出力する。
pub(super) fn main(arguments: Arguments) {
    if let Err(error) = minase::eval::weights() {
        eprintln!("error: embedded evaluation weights are invalid: {error}");
        process::exit(1);
    }
    if let Err(error) = arguments.validate_ponder() {
        crate::subcommand_command(&["match", "run"])
            .error(ErrorKind::ValueValidation, error)
            .exit();
    }
    if arguments.resume.is_some() && arguments.mode.is_some() {
        crate::subcommand_command(&["match", "run"])
            .error(
                ErrorKind::ArgumentConflict,
                "--resume cannot be combined with a mode subcommand",
            )
            .exit();
    }
    if arguments.run_dir.is_some() && arguments.mode.is_none() {
        crate::subcommand_command(&["match", "run"])
            .error(
                ErrorKind::MissingSubcommand,
                "a new run requires a mode subcommand",
            )
            .exit();
    }
    let (store, saved_records, rules, candidate, baseline) = if let Some(path) = &arguments.resume {
        let (store, records) =
            RunStore::resume(path, arguments.target_pairs).unwrap_or_else(|error| {
                eprintln!("failed to resume run directory {}: {error}", path.display());
                process::exit(1);
            });
        let (rules, candidate, baseline) =
            resume::restore(&store.manifest).unwrap_or_else(|error| {
                eprintln!("failed to restore experiment: {error}");
                process::exit(1);
            });
        (store, records, rules, candidate, baseline)
    } else {
        let rules = match Rules::from_codes(&arguments.rules.codes) {
            Ok(rules) => rules,
            Err(error) => crate::subcommand_command(&["match", "run"])
                .error(ErrorKind::ValueValidation, error.to_string())
                .exit(),
        };
        let base_seed = match arguments.seed {
            Some(seed) => seed,
            None => match time_seed() {
                Ok(seed) => seed,
                Err(error) => {
                    eprintln!("failed to generate a seed from the current time: {error}");
                    process::exit(1);
                }
            },
        };
        let (target_pairs, manifest_mode) =
            match arguments.mode.as_ref().expect("new run mode was validated") {
                Mode::Gsprt { max_pairs } => (
                    *max_pairs,
                    ManifestMode::Gsprt {
                        h0_elo: 0.0,
                        h1_elo: GSPRT_H1_ELO,
                        alpha: 0.05,
                        beta: 0.05,
                    },
                ),
                Mode::Elo { pairs } => (*pairs, ManifestMode::Elo),
            };
        let candidate_limit = arguments.candidate_limit.unwrap_or(arguments.each);
        let baseline_limit = arguments.baseline_limit.unwrap_or(arguments.each);
        let candidate = match resolve_player(
            arguments.candidate.clone(),
            candidate_limit,
            arguments.candidate_hash,
            &arguments.rules.source,
            Vec::new(),
        ) {
            Ok(player) => player,
            Err(error) => {
                eprintln!("failed to resolve candidate engine: {error}");
                process::exit(1);
            }
        };
        let baseline = match resolve_player(
            arguments.baseline.clone(),
            baseline_limit,
            arguments.baseline_hash,
            &arguments.rules.source,
            Vec::new(),
        ) {
            Ok(player) => player,
            Err(error) => {
                eprintln!("failed to resolve baseline engine: {error}");
                process::exit(1);
            }
        };
        let manifest = match run_manifest(
            &candidate,
            &baseline,
            &arguments.rules,
            manifest_mode,
            base_seed,
            arguments.max_ply,
            arguments.response_timeout,
            arguments.concurrency,
            arguments.ponder,
        ) {
            Ok(manifest) => manifest,
            Err(error) => {
                eprintln!("failed to identify the experiment environment: {error}");
                process::exit(1);
            }
        };
        let path = arguments
            .run_dir
            .as_ref()
            .expect("clap requires a run directory");
        let store = RunStore::create(path, manifest, target_pairs).unwrap_or_else(|error| {
            eprintln!("failed to create run directory {}: {error}", path.display());
            process::exit(1);
        });
        (store, BTreeMap::new(), rules, candidate, baseline)
    };
    let manifest = &store.manifest;
    let target_pairs = store.target_pairs;
    let concurrency = manifest.concurrency;
    let base_seed = manifest.seed;
    let max_ply = manifest.max_ply;
    let ponder = manifest.ponder;
    let use_gsprt = matches!(manifest.mode, ManifestMode::Gsprt { .. });
    let rules_text = manifest.canonical_rules.join(",");
    let response_timeout = Duration::from_secs(manifest.response_timeout_secs);
    println!("run_dir: {}", store.path().display());
    println!("rules: {rules_text}");
    println!("seed: {base_seed}");
    println!("max_ply: {max_ply}");
    println!("candidate: {}", candidate.name());
    println!("baseline: {}", baseline.name());
    println!("response_timeout: {} s", manifest.response_timeout_secs);

    let mut results = [0; 5];
    let mut valid_pairs = 0;
    let mut discarded_pairs = 0;
    let mut failures = FailureCounts::default();
    let mut decision = GsprtDecision::Continue;
    let stop = Arc::new(AtomicBool::new(false));
    let saved_numbers = saved_records.keys().copied().collect::<BTreeSet<_>>();
    let mut completed = BTreeMap::new();
    for (number, record) in saved_records {
        let pair = match completed_pair_from_record(
            record,
            rules,
            base_seed,
            max_ply,
            response_timeout,
            &candidate,
            &baseline,
        ) {
            Ok(pair) => pair,
            Err(error) => {
                eprintln!("saved pair {number} is invalid: {error}");
                process::exit(1);
            }
        };
        completed.insert(number, pair);
    }
    let mut next_to_integrate = 1_u64;
    while let Some(pair) = completed.remove(&next_to_integrate) {
        next_to_integrate = next_to_integrate
            .checked_add(1)
            .expect("pair number overflow");
        if !integrate_pair_statistics(
            pair,
            &mut results,
            &mut valid_pairs,
            &mut discarded_pairs,
            &mut failures,
            &mut decision,
            &manifest.mode,
            &stop,
        ) {
            break;
        }
    }
    let mut pending_jobs = (1..=target_pairs)
        .filter(|number| !saved_numbers.contains(number))
        .collect::<VecDeque<_>>();
    let has_new_work = !stop.load(Ordering::Acquire) && !pending_jobs.is_empty();
    if has_new_work && let Err(error) = store.begin_invocation() {
        eprintln!("failed to begin active wall time recording: {error}");
        process::exit(1);
    }
    let start = Instant::now();
    let worker_count = if stop.load(Ordering::Acquire) {
        0
    } else {
        concurrency.min(pending_jobs.len())
    };
    let pool_result = thread::scope(|scope| {
        let (job_sender, job_receiver) = mpsc::channel::<u64>();
        let job_receiver = Arc::new(Mutex::new(job_receiver));
        let (result_sender, result_receiver) = mpsc::channel::<Result<CompletedPair, String>>();
        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let result_sender = result_sender.clone();
            let job_receiver = Arc::clone(&job_receiver);
            let stop = Arc::clone(&stop);
            let candidate = &candidate;
            let baseline = &baseline;
            let rules_text = &rules_text;
            let store = &store;
            workers.push(scope.spawn(move || {
                let worker_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_worker_loop(&job_receiver, &result_sender, &stop, |pair_number, stop| {
                        let pair = run_pair(
                            rules,
                            rules_text,
                            base_seed,
                            pair_number,
                            max_ply,
                            candidate,
                            baseline,
                            response_timeout,
                            ponder,
                            stop,
                        );
                        let Some(pair) = pair else {
                            return Ok(None);
                        };
                        store.save_pair(&pair.record).map_err(|error| {
                            format!("failed to save pair {pair_number}: {error}")
                        })?;
                        Ok(Some(pair))
                    });
                }));
                if worker_result.is_err() {
                    let _ = result_sender.send(Err("a match worker panicked".to_owned()));
                }
            }));
        }
        drop(result_sender);

        for _ in 0..worker_count {
            let pair_number = pending_jobs
                .pop_front()
                .expect("worker count is bounded by pending jobs");
            job_sender
                .send(pair_number)
                .expect("workers must be waiting for initial jobs");
        }
        let mut pool_error = None;

        while next_to_integrate <= target_pairs
            && (!use_gsprt || decision == GsprtDecision::Continue)
        {
            let pair = match result_receiver.recv() {
                Ok(Ok(pair)) => pair,
                Ok(Err(error)) => {
                    pool_error = Some(error);
                    break;
                }
                Err(error) => {
                    pool_error = Some(format!("worker result channel disconnected: {error}"));
                    break;
                }
            };
            // 完了ペアを一旦バッファし、ペア番号順に出力とLLR取り込みを行う。
            // 補充は受信1件につき1件なので、若い番号の完了を待つ間も枠が空かない。
            let replacement = accept_completed_pair(
                pair,
                &mut completed,
                &mut next_to_integrate,
                &mut pending_jobs,
                |pair| {
                    integrate_pair_statistics(
                        pair,
                        &mut results,
                        &mut valid_pairs,
                        &mut discarded_pairs,
                        &mut failures,
                        &mut decision,
                        &manifest.mode,
                        &stop,
                    )
                },
            );
            if let Some(job) = replacement
                && let Err(error) = job_sender.send(job)
            {
                pool_error = Some(format!("worker job channel disconnected: {error}"));
            }
            if pool_error.is_none()
                && let Err(error) = store.checkpoint(start.elapsed())
            {
                pool_error = Some(format!("failed to checkpoint active wall time: {error}"));
            }
            if pool_error.is_some() {
                break;
            }
        }

        stop.store(true, Ordering::Release);
        drop(job_sender);
        let mut worker_panicked = false;
        for worker in workers {
            worker_panicked |= worker.join().is_err();
        }
        worker_panicked |= result_receiver.try_iter().any(|result| result.is_err());
        if worker_panicked {
            Err("a match worker panicked".to_owned())
        } else if let Some(error) = pool_error {
            Err(error)
        } else {
            Ok(())
        }
    });
    if let Err(error) = pool_result {
        eprintln!("match execution failed: {error}");
        process::exit(1);
    }
    if has_new_work && let Err(error) = store.finish_invocation(start.elapsed()) {
        eprintln!("failed to finalize active wall time: {error}");
        process::exit(1);
    }

    let ponder_counts = match ponder_summary(&store, rules, ponder, max_ply, target_pairs) {
        Ok(counts) => counts,
        Err(error) => {
            eprintln!("failed to replay ponder statistics: {error}");
            process::exit(1);
        }
    };
    for (name, counts) in ["candidate", "baseline"].into_iter().zip(ponder_counts) {
        println!(
            "ponder {name}: predictions={} illegal_predictions={} go_ponder={} hits={} moves={}",
            counts.predictions,
            counts.illegal_predictions,
            counts.starts,
            counts.hits,
            counts.moves
        );
    }

    if use_gsprt {
        print_gsprt_summary(
            &results,
            discarded_pairs,
            failures,
            decision,
            &manifest.mode,
            start.elapsed(),
        );
    } else {
        print_elo_summary(&results, discarded_pairs, failures, start.elapsed());
    }
}
