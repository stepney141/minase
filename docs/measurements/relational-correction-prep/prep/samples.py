"""採用PSTと同じ対局分割から、シード1で2つの検証標本を採る。"""
import argparse
import hashlib
import json
import sys
import tomllib

from common import DATA, OUTPUT, WT, identity, invocation, write_json

sys.path.insert(0, str(WT / 'tools/train/src'))
import numpy as np
from minase_train.data.mnsd import Dataset


def select(dataset, size):
    # 2つの標本ごとに生成器を初期化する。標本間の重複は排除しない。
    indices = np.random.default_rng(1).choice(
        dataset.validation_indices, size=size, replace=False)
    files = np.searchsorted(dataset.offsets[1:], indices, side='right')
    rows = indices - dataset.offsets[files]
    if len(np.unique(indices)) != size:
        raise ValueError('duplicate sampled indices')
    for file in np.unique(files):
        if not np.all(dataset.validation_masks[file][rows[files == file]]):
            raise ValueError('sample contains a training record')
    return {
        'size': size,
        'seed': 1,
        'method': 'numpy.random.default_rng(1).choice(validation_indices, size, replace=False)',
        'index_base': 0,
        'row_definition': 'MNSD record index within file, excluding the header',
        'order': 'random draw order',
        'columns': ['file_index', 'row_index'],
        'rows': np.column_stack((files, rows)).tolist(),
        'global_indices_sha256_le_i64': hashlib.sha256(
            indices.astype('<i8').tobytes()).hexdigest(),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check-only', action='store_true',
                        help='全件を読み抽出まで検証するが、ファイルは書かない')
    args = parser.parse_args()
    destination = OUTPUT / 'samples'
    if not args.check_only:
        destination.mkdir(parents=True, exist_ok=False)
    config_path = DATA / 'strength-stage9/lookahead-pst.toml'
    config = tomllib.loads(config_path.read_text())
    paths = config['run']['data']
    if len(paths) != 11 or config['train']['lookahead'] != {'gamma': 0.9, 'plies': 40}:
        raise ValueError('adopted training configuration changed')
    # train_pst.command_train と pst_workflow.training_dataset と同じ指定。
    dataset = Dataset(paths, lookahead=config['train']['lookahead'],
                      rescore=['-'] * len(paths), lambda_override=None)
    counts = {'training': int(dataset.training_indices.size),
              'validation': int(dataset.validation_indices.size)}
    summary = {
        'counts': counts,
        'expected_counts': {'training': 23879611, 'validation': 1241072},
        'counts_match_expected': counts == {'training': 23879611, 'validation': 1241072},
        'numpy_version': np.__version__,
        'config': identity(config_path),
        'reader': identity(WT / 'tools/train/src/minase_train/data/mnsd.py'),
        'command': invocation(),
        'split': 'Dataset.validation_indices; game-level hash modulo 20 == 0',
        'teacher_lookahead': config['train']['lookahead'],
        'files': [{'file_index': i, 'path': str(path),
                   'training_count': len(dataset.training_indices_by_file[i]),
                   'validation_count': len(dataset.validation_indices_by_file[i])}
                  for i, path in enumerate(dataset.paths)],
        'sample_overlap_allowed': True,
    }
    selections = {}
    for name, size in (('teacher', 100000), ('quantization', 10000)):
        sample = select(dataset, size)
        sample['files'] = summary['files']
        sample['numpy_version'] = np.__version__
        selections[name] = sample
        if not args.check_only:
            write_json(destination / f'{name}.json', sample)
        summary[name] = {k: v for k, v in sample.items() if k not in ('rows', 'files')}
    left, right = (set(map(tuple, selections[name]['rows']))
                   for name in ('teacher', 'quantization'))
    summary['sample_overlap_count'] = len(left & right)
    summary['saved'] = not args.check_only
    if not args.check_only:
        write_json(destination / 'summary.json', summary)
    print(json.dumps(summary, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
