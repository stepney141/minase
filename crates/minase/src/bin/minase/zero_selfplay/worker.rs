//! 複数局から1件ずつ葉を集めるバッチ推論と、スレッドの回収。

use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};

use minase::nn::{Evaluator, NnError};

use super::Error;
use super::game::{CompletedGame, PlayingGame, Settings};

/// 未確定の塊の対局番号を、全スレッドへ重複なく配る。
struct Source {
    numbers: Vec<u32>,
    next: AtomicUsize,
}

impl Source {
    fn take(&self) -> Option<u32> {
        self.numbers
            .get(self.next.fetch_add(1, Ordering::Relaxed))
            .copied()
    }
}

/// 全ワーカーを所有し、エラーや巻き戻しでも停止してjoinする。
/// 停止要求後は進行中の1巡と推論の終了まで待つ。推論自体は中断できない。
pub(super) struct Workers {
    pub(super) results: mpsc::Receiver<CompletedGame>,
    stop: Arc<AtomicBool>,
    handles: Vec<JoinHandle<Result<(), Error>>>,
}

impl Workers {
    /// 構築済みの推論器を各スレッドへ移す。起動に失敗しても先行スレッドを回収する。
    pub(super) fn start(
        numbers: Vec<u32>,
        settings: Settings,
        games_per_thread: NonZeroUsize,
        evaluators: Vec<Box<dyn Evaluator + Send>>,
    ) -> Result<Self, Error> {
        let source = Arc::new(Source {
            numbers,
            next: AtomicUsize::new(0),
        });
        let (sender, results) = mpsc::channel();
        let mut workers = Self {
            results,
            stop: Arc::new(AtomicBool::new(false)),
            handles: Vec::new(),
        };
        for (index, mut evaluator) in evaluators.into_iter().enumerate() {
            let source = Arc::clone(&source);
            let stop = Arc::clone(&workers.stop);
            let sender = sender.clone();
            workers.handles.push(
                thread::Builder::new()
                    .name(format!("zero-selfplay-{index}"))
                    .spawn(move || {
                        let result = catch_unwind(AssertUnwindSafe(|| {
                            run(
                                &source,
                                &stop,
                                settings,
                                games_per_thread,
                                evaluator.as_mut(),
                                &sender,
                            )
                        }))
                        .map_err(Error::Panic)
                        .and_then(|result| result);
                        if result.is_err() {
                            stop.store(true, Ordering::Relaxed);
                        }
                        result
                    })?,
            );
        }
        Ok(workers)
    }

    /// すべてを回収してから、最初の失敗を返す。
    pub(super) fn join(&mut self) -> Result<(), Error> {
        let mut first_error = None;
        for handle in self.handles.drain(..) {
            if let Err(error) = handle
                .join()
                .map_err(Error::Panic)
                .and_then(|result| result)
            {
                self.stop.store(true, Ordering::Relaxed);
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.join();
    }
}

fn run(
    source: &Source,
    stop: &AtomicBool,
    settings: Settings,
    games_per_thread: NonZeroUsize,
    evaluator: &mut dyn Evaluator,
    sender: &mpsc::Sender<CompletedGame>,
) -> Result<(), Error> {
    let mut games: Vec<Option<PlayingGame>> = (0..games_per_thread.get()).map(|_| None).collect();
    let mut logits = Vec::new();
    let mut values = Vec::new();
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let mut requests = Vec::new();
        for (index, slot) in games.iter_mut().enumerate() {
            loop {
                if stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                if slot.is_none() {
                    let Some(number) = source.take() else {
                        break;
                    };
                    *slot = Some(PlayingGame::new(number, settings)?);
                }
                let game = slot.as_mut().expect("the slot was filled above");
                if let Some(request) = game.advance()? {
                    requests.push((index, request));
                    break;
                }
                let completed = slot
                    .take()
                    .expect("the completed game occupies its slot")
                    .finish()?;
                sender.send(completed).map_err(|_| Error::ReceiverClosed)?;
            }
        }
        if requests.is_empty() {
            return Ok(());
        }
        let queries: Vec<_> = requests
            .iter()
            .map(|(_, request)| request.query())
            .collect();
        evaluator.evaluate(&queries, &mut logits, &mut values)?;
        let total_logits = queries.iter().map(|query| query.moves.len()).sum::<usize>();
        for (name, expected, actual) in [
            ("values", requests.len(), values.len()),
            ("logits", total_logits, logits.len()),
        ] {
            if expected != actual {
                return Err(NnError::OutputShape {
                    name,
                    expected: [expected, 1],
                    actual: vec![actual as i64, 1],
                }
                .into());
            }
        }
        let mut offset = 0;
        for ((index, request), value) in requests.iter().zip(&values) {
            let end = offset + request.query().moves.len();
            games[*index]
                .as_mut()
                .expect("a pending request retains its game")
                .evaluate(request, &logits[offset..end], *value)?;
            offset = end;
        }
    }
}
