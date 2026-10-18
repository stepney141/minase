"""版3の候補と、同じPSTの関係表を0にした評価をRustで測る。"""
from dataclasses import replace
import json
from pathlib import Path
import subprocess
import sys

from common import WT, write_json

sys.path.insert(0, str(WT / 'tools/train/src'))
import numpy as np
from mnpt_v3 import decode
from minase_train.data.mnsd import RECORD_DTYPE, map_records, write_mnsd
from minase_train.data.taper import band_indices, phase_ratios


def zero_weights(source, destination):
    model = decode(source.read_bytes())
    destination.write_bytes(replace(model, pair_weights=(0,) * len(model.pair_weights)).encode())
    return model


def probe(binary, weights, positions, count, *, skip_invalid=False):
    argv = [str(binary.resolve()), '--pst', str(weights.resolve()),
            '--positions', str(positions.resolve())]
    if skip_invalid:
        argv.append('--skip-invalid')
    result = subprocess.run(argv, capture_output=True, text=True, check=True)
    rows = json.loads(result.stdout)
    if len(rows) != count or [r['index'] for r in rows] != list(range(count)):
        raise ValueError('pst_probe omitted or reordered records')
    if not skip_invalid and any('skipped' in r for r in rows):
        raise ValueError('pst_probe skipped a source record')
    return rows


def evaluate(binary, weights, zero, records, directory, *, skip_invalid=False):
    directory.mkdir()
    positions = directory / 'sample.bin'
    write_mnsd(positions, records, seed=1, network_checksum=weights.read_bytes()[48:80])
    candidate = probe(binary, weights, positions, len(records), skip_invalid=skip_invalid)
    baseline = probe(binary, zero, positions, len(records), skip_invalid=skip_invalid)
    rows = []
    for left, right in zip(candidate, baseline):
        if ('skipped' in left) != ('skipped' in right):
            raise ValueError('candidate and PST-only validity disagree')
        row = dict(left)
        if 'skipped' not in left:
            row['eval_pst'] = right['eval']
        rows.append(row)
    write_json(directory / 'probe.json', rows)
    return rows


def teacher_records(path):
    sample = json.loads(path.read_text())
    if sample['index_base'] != 0 or sample['columns'] != ['file_index', 'row_index']:
        raise ValueError('teacher sample must use zero-based file/row indices')
    rows = np.asarray(sample['rows'])
    if rows.shape != (sample['size'], 2) or not len(rows) or rows.dtype.kind not in 'iu':
        raise ValueError('teacher sample must contain size nonempty integer file/row pairs')
    if np.any(rows < 0) or np.any(rows[:, 0] >= len(sample['files'])):
        raise ValueError('teacher file/row index out of range')
    if len(np.unique(rows, axis=0)) != len(rows):
        raise ValueError('duplicate teacher sample rows')
    records = np.empty(len(rows), dtype=RECORD_DTYPE)
    paths = []
    for index, item in enumerate(sample['files']):
        if item['file_index'] != index:
            raise ValueError('teacher file order mismatch')
        source = Path(item['path'])
        paths.append(source)
        selected = rows[:, 0] == index
        mapped = map_records(source)
        local = rows[selected, 1]
        if np.any(local >= len(mapped)):
            raise ValueError('teacher row index out of range')
        records[selected] = mapped[local]
    return records, paths
