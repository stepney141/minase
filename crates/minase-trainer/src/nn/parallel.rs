//! 独立した演算を分割し、結果の順序を保つ並列処理。

use super::{Error, Result};
use std::{ops::Range, thread};

/// スレッド数が正であることを検査する。
pub fn check_threads(threads: usize) -> Result<()> {
    if threads == 0 {
        return Err(Error::Invalid("CPU thread count must be positive"));
    }
    Ok(())
}

/// 連続した添字を、長さの差が1以下になるよう順に分ける。
pub fn ranges(len: usize, threads: usize) -> Result<Vec<Range<usize>>> {
    check_threads(threads)?;
    let workers = threads.min(len);
    Ok((0..workers)
        .map(|worker| {
            let start = worker * (len / workers) + worker.min(len % workers);
            let size = len / workers + usize::from(worker < len % workers);
            start..start + size
        })
        .collect())
}

/// 各入力を別スレッドで処理し、完了順によらず入力順に結果を返す。
/// エラーやパニックが起きても全ワーカーを回収してから返す。
pub fn map<I, T, E, F>(items: I, operation: F) -> std::result::Result<Vec<T>, E>
where
    I: IntoIterator,
    I::Item: Send,
    T: Send,
    E: Send + From<Error>,
    F: Fn(I::Item) -> std::result::Result<T, E> + Sync,
{
    let items: Vec<_> = items.into_iter().collect();
    if items.len() <= 1 {
        return items.into_iter().map(operation).collect();
    }
    thread::scope(|scope| {
        let operation = &operation;
        let mut handles = Vec::with_capacity(items.len());
        let mut spawn_error = None;
        for item in items {
            match thread::Builder::new().spawn_scoped(scope, move || operation(item)) {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    spawn_error = Some(E::from(Error::Io(error)));
                    break;
                }
            }
        }
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| E::from(Error::Invalid("CPU worker panicked")))
                    .and_then(|result| result)
            })
            .collect();
        if let Some(error) = spawn_error {
            return Err(error);
        }
        results.into_iter().collect()
    })
}
