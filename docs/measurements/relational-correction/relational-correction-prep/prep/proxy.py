"""近接2駒で一致した15根の子局面をS0で探索し、教師支持を数える。"""
import argparse
import importlib.util
import json
import sys
from decimal import Decimal

from common import BASE, OUTPUT, WT, identity, invocation, sha256, write_json

PHASE0 = WT / 'docs/measurements/relational-correction-prep/phase0.py'


def read_rows(path):
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    if len({row['root_id'] for row in rows}) != len(rows):
        raise ValueError(f'{path}: duplicate root_id')
    return rows


def roots():
    classification = WT / 'docs/measurements/relational-correction-prep/classification'
    paths = [classification / f'{name}.jsonl' for name in ('claude', 'codex')]
    selected = [{row['root_id'] for row in read_rows(path)
                 if row['category'] == '近接2駒'} for path in paths]
    agreed = selected[0] & selected[1]
    if len(agreed) != 15:
        raise ValueError(f'expected 15 agreed roots, got {len(agreed)}')
    found = {}
    teachers = sorted((BASE / 'phase0').glob('batch*/teacher.jsonl'))
    for path in teachers:
        for row in read_rows(path):
            if row['root_id'] in agreed:
                if row['root_id'] in found:
                    raise ValueError('root occurs in multiple teacher files')
                if len(row['moves']) != row['ply'] or row['a_star'] == row['s0']:
                    raise ValueError('unexpected root moves or proposals')
                found[row['root_id']] = row
    if set(found) != agreed:
        raise ValueError('missing teacher roots')
    return [found[key] for key in sorted(found)], paths + teachers


def compare_scores(a_child, s_child):
    # どちらも子局面の手番側から根の手番側へ反転する。
    qa, qs = -a_child, -s_child
    difference = qa - qs
    return {'q_a_star': qa, 'q_s0': qs, 'difference_cp': difference,
            'supports_a_star': Decimal(str(difference)) >= Decimal('117.7')}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stdout-only', action='store_true',
                        help='同じ30探索を実行し、JSONは標準出力だけに出す')
    args = parser.parse_args()
    destination = OUTPUT / 'proxy'
    if not args.stdout_only:
        destination.mkdir(parents=True, exist_ok=False)
    selected, paths = roots()
    spec = importlib.util.spec_from_file_location('prep_phase0', PHASE0)
    phase0 = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(phase0)
    binary = BASE / 'bin/minase'
    checksum = sha256(binary)
    if any(row['engine_sha256'] != checksum for row in selected):
        raise ValueError('S0 binary differs from phase0 teacher engine')
    control = phase0.Control()
    results = []
    for root in selected:
        searches = {}
        for key in ('a_star', 's0'):
            search = phase0.run_search(binary, control,
                                       root['moves'] + [root[key]], 'nodes 100000')
            # センチポーン差として集計する。詰み等を任意のcpへ変換しない。
            if search['kind'] != 'cp':
                raise ValueError(f"{root['root_id']} {key}: non-cp result {search}")
            searches[key] = search
        row = {'root_id': root['root_id'], 'moves': root['moves'],
               'a_star': root['a_star'], 's0': root['s0'], 'searches': searches,
               **compare_scores(searches['a_star']['score'], searches['s0']['score'])}
        results.append(row)
        print(f"{row['root_id']}: {row['difference_cp']} cp; "
              f"supports={row['supports_a_star']}", file=sys.stderr, flush=True)
    if sha256(binary) != checksum:
        raise ValueError('S0 binary changed during measurement')
    count = sum(row['supports_a_star'] for row in results)
    report = {
        'engine': identity(binary), 'phase0': identity(PHASE0),
        'inputs': [identity(path) for path in paths],
        'command': invocation(),
        'engine_argv': [str(binary), '--protocol', 'usi', '--rules', 'engine-default'],
        'usi_options': {'USI_Hash': 64, 'Threads': 1, 'ResignValue': 99999},
        'go': 'go nodes 100000', 'fresh_process_per_child': True,
        'perspective': 'root side to move; Q(move) = -child score',
        'threshold_cp': 117.7, 'root_count': len(results),
        'supports_a_star_count': count, 'supports_a_star_fraction': count / len(results),
        'saved': not args.stdout_only,
        'interpretation': 'proxy for search component, not actual training labels or an adoption gate',
        'roots': results,
    }
    if not args.stdout_only:
        write_json(destination / 'proxy.json', report)
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
