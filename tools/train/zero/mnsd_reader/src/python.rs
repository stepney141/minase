//! 読み込み中にGILを解放し、Rustの列をPythonのbytesへ渡す束縛。

use std::path::Path;

use minase_core::Rules;
use pyo3::{
    exceptions::{PyOSError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict},
};

use crate::{Error, FileInfo, Records};

fn python_error(path: &str, error: Error) -> PyErr {
    let message = format!("{path}: {error}");
    match error {
        Error::Io(_) => PyOSError::new_err(message),
        _ => PyValueError::new_err(message),
    }
}

/// 配列を明示的なリトルエンディアンで詰め、Pythonの割り当て失敗を例外として返す。
fn bytes<'py, T, const N: usize>(
    py: Python<'py>,
    values: &[T],
    encode: impl Fn(&T) -> [u8; N],
) -> PyResult<Bound<'py, PyBytes>> {
    let length = values
        .len()
        .checked_mul(N)
        .filter(|&length| length <= isize::MAX as usize)
        .ok_or_else(|| PyValueError::new_err("array byte length exceeds address space"))?;
    PyBytes::new_with(py, length, |buffer| {
        for (target, value) in buffer.as_chunks_mut::<N>().0.iter_mut().zip(values) {
            target.copy_from_slice(&encode(value));
        }
        Ok(())
    })
}

/// MNSDを読み、ヘッダとリトルエンディアンの配列を持つdictを返す。
///
/// ヘッダはorigin（intまたはNone）、teacher_kind、teacher_search_budget、seed、
/// game_count、entry_count（int）、rule_set、generation_commit（str）、
/// engine_default（bool）、network_checksum（32バイトのbytes）である。
/// 配列もbytesであり、game_numbersとgamesはu32、pliesはu16、valuesはi32、visitsはu32、
/// policy_offsetsはu64、policy_probabilitiesはu16、boards、sides、lions、
/// policy_from、policy_mid、policy_to、policy_promoteはu8である。
/// boardsは局面数×144、policy_offsetsは局面数+1、game_numbersは全対局数の長さを持つ。
/// games、plies、sides、lions、values、outcomes、visitsは局面数、各候補列は全候補数の長さを持つ。
/// outcomesはu8で先手勝ち0、後手勝ち1、引き分け2、sidesは先手0、後手1である。
/// lionsとpolicy_midのなしは255、升はdense_index、policy_promoteは0か1である。
/// legal_offsetsはu64の局面数+1、legal_from、legal_mid、legal_to、legal_promoteは
/// u8の全合法手数で、升と成りの表現は候補列と同じである。
/// valuesはαβのセンチポーンまたはMCTSの0〜65,535、候補の確率は65,535を1とする。
/// 入出力の失敗はOSError、形式と内容の誤りはValueErrorを送出する。
#[pyfunction]
fn read(py: Python<'_>, path: String) -> PyResult<Bound<'_, PyDict>> {
    let records = py
        .detach(|| crate::read_file(Path::new(&path)))
        .map_err(|error| python_error(&path, error))?;
    record_dict(py, &records)
}

/// 手順を再生せず、抽出前の件数と全対局番号を返す。
#[pyfunction]
fn scan(py: Python<'_>, path: String) -> PyResult<Bound<'_, PyDict>> {
    let info = py
        .detach(|| crate::scan_file(Path::new(&path)))
        .map_err(|error| python_error(&path, error))?;
    info_dict(py, &info)
}

/// GILを解放して全手順を再生し、指定した局面だけをPythonへ渡す。
#[pyfunction]
fn read_selected(
    py: Python<'_>,
    path: String,
    training_indices: Vec<u64>,
    include_validation: bool,
) -> PyResult<Bound<'_, PyDict>> {
    let records = py
        .detach(|| crate::read_selected(Path::new(&path), &training_indices, include_validation))
        .map_err(|error| python_error(&path, error))?;
    record_dict(py, &records)
}

fn info_dict<'py>(py: Python<'py>, info: &FileInfo) -> PyResult<Bound<'py, PyDict>> {
    let header = &info.header;
    let output = PyDict::new(py);
    output.set_item("origin", info.origin.map(|origin| origin as u8))?;
    output.set_item("teacher_kind", header.teacher_kind() as u8)?;
    output.set_item("rule_set", header.rule_set())?;
    output.set_item("engine_default", header.rules() == Rules::ENGINE_DEFAULT)?;
    output.set_item("generation_commit", header.generation_commit())?;
    output.set_item(
        "network_checksum",
        bytes(py, header.network_checksum(), |&v| [v])?,
    )?;
    output.set_item("teacher_search_budget", header.teacher_search_budget())?;
    output.set_item("seed", header.seed())?;
    output.set_item("game_count", header.game_count())?;
    output.set_item("entry_count", header.entry_count())?;
    output.set_item(
        "game_numbers",
        bytes(py, &info.game_numbers, |v| v.to_le_bytes())?,
    )?;
    output.set_item("training_count", info.training_count)?;
    output.set_item("validation_count", info.validation_count)?;
    Ok(output)
}

fn record_dict<'py>(py: Python<'py>, records: &Records) -> PyResult<Bound<'py, PyDict>> {
    let output = info_dict(py, &records.info)?;
    output.set_item("games", bytes(py, &records.games, |v| v.to_le_bytes())?)?;
    output.set_item("plies", bytes(py, &records.plies, |v| v.to_le_bytes())?)?;
    output.set_item("values", bytes(py, &records.values, |v| v.to_le_bytes())?)?;
    output.set_item("visits", bytes(py, &records.visits, |v| v.to_le_bytes())?)?;
    output.set_item(
        "policy_offsets",
        bytes(py, &records.policy_offsets, |v| v.to_le_bytes())?,
    )?;
    output.set_item(
        "policy_probabilities",
        bytes(py, &records.policy_probabilities, |v| v.to_le_bytes())?,
    )?;
    output.set_item(
        "legal_offsets",
        bytes(py, &records.legal_offsets, |v| v.to_le_bytes())?,
    )?;
    for (key, values) in [
        ("boards", &records.boards),
        ("sides", &records.sides),
        ("lions", &records.lions),
        ("outcomes", &records.outcomes),
        ("policy_from", &records.policy_from),
        ("policy_mid", &records.policy_mid),
        ("policy_to", &records.policy_to),
        ("policy_promote", &records.policy_promote),
        ("legal_from", &records.legal_from),
        ("legal_mid", &records.legal_mid),
        ("legal_to", &records.legal_to),
        ("legal_promote", &records.legal_promote),
    ] {
        output.set_item(key, bytes(py, values, |&v| [v])?)?;
    }
    Ok(output)
}

#[pymodule]
fn minase_mnsd_reader(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("FORMAT_VERSION", minase::training::records::FORMAT_VERSION)?;
    module.add_function(wrap_pyfunction!(read, module)?)?;
    module.add_function(wrap_pyfunction!(scan, module)?)?;
    module.add_function(wrap_pyfunction!(read_selected, module)?)?;
    Ok(())
}
