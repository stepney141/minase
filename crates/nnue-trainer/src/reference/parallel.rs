use crate::model::{Error, Result};
use std::{ops::Range, thread};

pub(super) fn check_threads(threads: usize) -> Result<()> {
    if threads == 0 {
        return Err(Error::Invalid("CPU thread count must be positive"));
    }
    Ok(())
}

pub(super) fn ranges(len: usize, threads: usize) -> Result<Vec<Range<usize>>> {
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

pub(super) fn rows_mut<T>(
    values: &mut [T],
    width: usize,
    threads: usize,
) -> Result<Vec<(usize, &mut [T])>> {
    let ranges = ranges(values.len() / width, threads)?;
    let mut rows = Vec::with_capacity(ranges.len());
    let mut remaining = values;
    for range in ranges {
        let (part, rest) = remaining.split_at_mut(range.len() * width);
        rows.push((range.start, part));
        remaining = rest;
    }
    Ok(rows)
}

// 互いに独立な書き込みだけを分割する。結果は入力順で返し、縮約順を変えない。
pub(super) fn map<I, T, F>(items: I, operation: F) -> Result<Vec<T>>
where
    I: IntoIterator,
    I::Item: Send,
    T: Send,
    F: Fn(I::Item) -> Result<T> + Sync,
{
    let items: Vec<_> = items.into_iter().collect();
    if items.len() <= 1 {
        return items.into_iter().map(operation).collect();
    }
    thread::scope(|scope| {
        let operation = &operation;
        let handles: Vec<_> = items
            .into_iter()
            .map(|item| scope.spawn(move || operation(item)))
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| Error::Invalid("CPU worker panicked"))?
            })
            .collect()
    })
}
