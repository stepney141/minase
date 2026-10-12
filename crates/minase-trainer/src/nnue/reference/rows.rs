use crate::{nn::parallel::ranges, nnue::model::Result};

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
