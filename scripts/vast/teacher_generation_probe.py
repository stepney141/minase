#!/usr/bin/env python3
"""生成機の作業プロセス、CPU時間、および回収対象の大きさをJSONで返す。

使い方: python3 /workspace/job/teacher_generation_probe.py <世代>
Python 3.11の標準ライブラリだけを使い、/workspace/jobと/procを読む。
worker.pidはsetsidで起動した待機用bashが書き、exec後も同じPIDを保つ。
"""

import argparse
import json
import os
from pathlib import Path
import sys


def probe(root: Path, proc: Path, generation: int, ticks: int) -> dict:
    """同じセッションの全プロセスと終了済みの子のCPU時間を合計する。"""
    pid = int((root / 'worker.pid').read_text())
    if pid <= 0:
        raise ValueError('worker.pidが正の整数ではありません')
    alive = False
    cpu_ticks = None
    for directory in proc.iterdir():
        if not directory.name.isdecimal():
            continue
        try:
            # commには空白と括弧も入るので、最後の閉じ括弧の後を欄3として読む。
            fields = (directory / 'stat').read_text().rsplit(')', 1)[1].split()
        except (FileNotFoundError, ProcessLookupError):
            continue
        if int(directory.name) == pid:
            alive = fields[0] not in ('Z', 'X', 'x')
        if int(fields[3]) == pid:
            value = sum(int(fields[index]) for index in (11, 12, 13, 14))
            cpu_ticks = value if cpu_ticks is None else cpu_ticks + value
    size = 0
    for name in ('work', 'teacher', 'logs'):
        for path in (root / name / f'gen-{generation}').rglob('*'):
            try:
                if path.is_file():
                    size += path.stat().st_size
            except FileNotFoundError:
                continue
    status = root / 'logs/exit-status'
    return {'pid': pid, 'alive': alive,
            'exit_status': status.read_text().strip() if status.is_file() else None,
            'cpu_seconds': None if cpu_ticks is None else cpu_ticks / ticks, 'bytes': size}


def main(argv: list[str]) -> int:
    """世代を指定して1回だけ観測する。"""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('generation', type=int, help='0以上の世代番号')
    args = parser.parse_args(argv)
    if args.generation < 0:
        parser.error('世代番号は0以上にしてください')
    try:
        print(json.dumps(probe(Path('/workspace/job'), Path('/proc'), args.generation,
                               os.sysconf('SC_CLK_TCK'))))
        return 0
    except Exception as error:
        print(f'生成機の観測に失敗しました: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
