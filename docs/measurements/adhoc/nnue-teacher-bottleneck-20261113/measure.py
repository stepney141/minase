"""Run one reproducible teacher-generation profile and sample its CPU occupancy."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def engine_pid(parent, binary):
    try:
        if Path(os.readlink(f'/proc/{parent}/exe')) == binary:
            return parent
        children = Path(f'/proc/{parent}/task/{parent}/children').read_text().split()
    except (FileNotFoundError, ProcessLookupError):
        return None
    for child in children:
        found = engine_pid(int(child), binary)
        if found is not None:
            return found
    return None


def snapshot(pid, elapsed):
    try:
        fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    except (FileNotFoundError, ProcessLookupError):
        return None
    return {
        'elapsed_seconds': elapsed,
        'user_ticks': int(fields[11]),
        'system_ticks': int(fields[12]),
        'threads': int(fields[17]),
        'rss_pages': int(fields[21]),
    }


def main():
    parser = argparse.ArgumentParser()
    for name in ('binary', 'source', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('games', 'concurrency', 'seed'):
        parser.add_argument('--' + name, type=int, required=True)
    parser.add_argument('--cpus', required=True)
    parser.add_argument('--mode', choices=('stat', 'topdown', 'record', 'stage'), required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    prefix = args.output.resolve()
    if any(prefix.parent.glob(prefix.name + '.*')):
        raise FileExistsError(f'Output prefix already exists: {prefix}')
    prefix.parent.mkdir(parents=True, exist_ok=True)
    command = ['nice', '-n', '19', 'taskset', '-c', args.cpus,
               '/usr/bin/time', '-v', '-o', str(prefix) + '.time']
    if args.mode in ('stat', 'stage'):
        events = ('task-clock,context-switches,cpu-migrations,page-faults,'
                  'cpu_core/cycles/u,cpu_core/instructions/u,'
                  'cpu_core/branches/u,cpu_core/branch-misses/u')
        command += ['perf', 'stat', '-x,', '-o', str(prefix) + '.stat', '-e', events, '--']
    elif args.mode == 'topdown':
        events = ('{cpu_core/slots/u,cpu_core/topdown-retiring/u,'
                  'cpu_core/topdown-bad-spec/u,cpu_core/topdown-fe-bound/u,'
                  'cpu_core/topdown-be-bound/u,cpu_core/topdown-mem-bound/u},'
                  'cpu_core/cycles/u,cpu_core/instructions/u,cpu_core/branch-misses/u')
        command += ['perf', 'stat', '-x,', '-o', str(prefix) + '.stat', '-e', events, '--']
    else:
        command += ['perf', 'record', '-o', str(prefix) + '.data', '--user-callchains',
                    '--call-graph', 'lbr', '-e', 'cpu_core/cycles,period=4000037/u',
                    '-e', 'cpu_core/mem_load_retired.l3_miss,period=4009/upp', '--']
    command += [str(binary), 'data', 'selfplay', 'generate',
                '--output', str(prefix) + '.mnsd', '--seed', str(args.seed),
                '--games', str(args.games), '--concurrency', str(args.concurrency),
                '--nodes', '20000', '--random-moves', '0', '--max-ply', '4000',
                '--hash-mb', '16', '--allow-dirty']
    Path(str(prefix) + '.command.json').write_text(json.dumps(command, indent=2) + '\n')
    start = time.monotonic()
    rows = []
    with Path(str(prefix) + '.stdout').open('w') as out, Path(str(prefix) + '.stderr').open('w') as err:
        process = subprocess.Popen(command, cwd=args.source, stdout=out, stderr=err)
        while process.poll() is None:
            pid = engine_pid(process.pid, binary)
            if pid is not None:
                row = snapshot(pid, time.monotonic() - start)
                if row is not None:
                    rows.append(row)
            time.sleep(0.25)
    sample = {'clock_ticks': os.sysconf('SC_CLK_TCK'), 'page_size': os.sysconf('SC_PAGE_SIZE'), 'rows': rows}
    Path(str(prefix) + '.process.json').write_text(json.dumps(sample, indent=2) + '\n')
    if process.returncode:
        raise subprocess.CalledProcessError(process.returncode, command)
    summary = {}
    for line in Path(str(prefix) + '.stdout').read_text().splitlines():
        if ': ' in line:
            name, value = line.split(': ', 1)
            summary[name] = value
    summary['sha256'] = hashlib.sha256(Path(str(prefix) + '.mnsd').read_bytes()).hexdigest()
    summary['process_wall_seconds'] = time.monotonic() - start
    Path(str(prefix) + '.summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps({'case': prefix.name, **{name: summary[name] for name in (
        'elapsed_seconds', 'recorded_positions', 'recorded_positions_per_second',
        'searched_nodes_total', 'games_completed', 'games_discarded_max_ply', 'sha256')}}, ensure_ascii=False), flush=True)


if __name__ == '__main__':
    main()
