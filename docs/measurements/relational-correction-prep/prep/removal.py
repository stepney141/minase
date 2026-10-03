"""既存PST診断と同じ代表局面・非王駒で、局所2駒表による除去差分の符号を調べる。"""
import argparse
import json
from pathlib import Path
import tomllib

from common import WT, identity, invocation, write_json
from phase3_common import evaluate, zero_weights, np, RECORD_DTYPE
from minase_train.data.features import INITIAL_BOARD, NO_LION_SQUARE, PIECE_STATE_BY_BYTE, COLOR_BY_BYTE
from minase_train.data.mnsd import Dataset, provenance_path
from minase_train.data.taper import band_samples, phase_numerators

# train_pst.ROYAL_STATESと同じ王将・太子。非零の関係表は既存のWeightsが拒否するため、
# pst_diagnosticsの代表選択と駒選択を再現し、評価は版3対応のpst_probeで行う。
ROYAL_STATES = (11, 21)


def representatives(dataset, sample_size, seed):
    samples = band_samples(dataset, sample_size, seed)
    candidates = {}
    for sample in samples:
        indices = sample['indices']
        if len(indices):
            band = sample['band']
            index = int(indices[0])
            candidates[band] = min(candidates[band], index) if band in candidates else index
    if not candidates:
        raise ValueError('every validation band is empty')
    initial = np.zeros(1, dtype=RECORD_DTYPE)
    initial['board'] = INITIAL_BOARD
    initial['lion'] = NO_LION_SQUARE
    selected = [candidates[b] for b in sorted(candidates)]
    records = np.concatenate([initial, dataset.gather(np.array(selected, dtype=np.int64))])
    labels = [{'label': 'initial', 'index': None}]
    labels += [{'label': f'band{band}', 'index': candidates[band]} for band in sorted(candidates)]
    return records, labels, samples


def variants(records):
    removed, locations = [], []
    for position, record in enumerate(records):
        board = record['board']
        for square in np.flatnonzero(board != 0):
            piece = int(board[square])
            state = int(PIECE_STATE_BY_BYTE[piece])
            if state in ROYAL_STATES:
                continue
            after = record.copy()
            after['board'][square] = 0
            removed.append(after)
            locations.append({'position': position, 'square': int(square), 'piece_byte': piece,
                              'state': state, 'relative_color': int(COLOR_BY_BYTE[piece] != record['stm'])})
    return np.array(removed, dtype=RECORD_DTYPE), locations


def summarize(before, after, locations):
    if len(after) != len(locations):
        raise ValueError('removal rows and locations differ')
    rows = []
    for evaluated, location in zip(after, locations):
        row = dict(location)
        original = before[location['position']]
        if 'skipped' in evaluated:
            row.update({'skipped': evaluated['skipped'], 'reversal': None})
        else:
            pst = int(evaluated['eval_pst']) - int(original['eval_pst'])
            candidate = int(evaluated['eval']) - int(original['eval'])
            row.update({'delta_pst_cp': pst, 'delta_candidate_cp': candidate,
                        'reversal': pst * candidate < 0})
        rows.append(row)
    examples = [r for r in rows if r['reversal'] is True]
    skipped = sum(r['reversal'] is None for r in rows)
    return {'attempted': len(rows), 'checked': len(rows) - skipped, 'skipped': skipped,
            'sign_reversals': len(examples), 'examples': examples, 'removals': rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--weights', type=Path, required=True)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--probe', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--seed', type=int, default=1)
    parser.add_argument('--sample-size', type=int, default=10000,
                        help='既存診断と同じく、教師世代・局面帯ごとの抽出上限')
    args = parser.parse_args()
    config = tomllib.loads(args.config.read_text())
    training = config['train']
    if ('rescore' in training and any(p != '-' for p in training['rescore'])) or (
            'lambda_override' in training and training['lambda_override'] is not None):
        raise ValueError('use original teacher data without rescore/lambda overrides')
    paths = [Path(p) if Path(p).is_absolute() else WT / p for p in config['run']['data']]
    dataset = Dataset(paths, lookahead=training['lookahead'], rescore=['-'] * len(paths))
    records, labels, samples = representatives(dataset, args.sample_size, args.seed)
    destination = args.output_dir.resolve()
    destination.mkdir(parents=True, exist_ok=False)
    zero = destination / 'pst-only.bin'
    zero_weights(args.weights, zero)
    before = evaluate(args.probe, args.weights, zero, records, destination / 'representatives')
    removed, locations = variants(records)
    after = evaluate(args.probe, args.weights, zero, removed, destination / 'removals', skip_invalid=True)
    report = summarize(before, after, locations)
    inputs = [args.weights, args.config, *paths, *[provenance_path(p) for p in paths]]
    report.update({'seed': args.seed, 'sample_size_per_generation_band': args.sample_size,
                   'selection': 'pst_diagnostics: initial plus minimum sampled global index per band across generations',
                   'delta': 'evaluation after removal minus before removal, unchanged side-to-move cp',
                   'reversal': 'delta_pst_cp * delta_candidate_cp < 0; zero is not a reversal',
                   'samples': [{**s, 'indices': s['indices'].tolist()} for s in samples],
                   'representatives': [{**row, **label, 'probe_index': row['index'], 'phase_numerator': int(q)}
                       for label, row, q in zip(labels, before, phase_numerators(records['board']))],
                   'inputs': [identity(p) for p in inputs],
                   'tools': [identity(p) for p in (args.probe, Path(__file__),
                       Path(__file__).with_name('phase3_common.py'), WT / 'tools/train/src/minase_train/data/taper.py')],
                   'command': invocation()})
    write_json(destination / 'removal.json', report)
    print(json.dumps({k: report[k] for k in ('attempted', 'checked', 'skipped', 'sign_reversals')}, indent=2))


if __name__ == '__main__':
    main()
