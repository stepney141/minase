#!/usr/bin/env python3
"""調整用ビルドを同時に実行し、負荷時の中央値NPSの平均を出力する。
規則はJCSAとし、指定したネットをEvalFileで読む。

使い方: python3 scripts/vast-spsa/bench.py <調整用ビルド> <本数> <ログのディレクトリ> <ネットのパス>
"""

from pathlib import Path
import re
import sys

import remote


def main() -> int:
    """本数を検査し、benchの中央値NPSの平均を1行で出力する。"""
    if len(sys.argv) != 5:
        print('使い方: bench.py <調整用ビルド> <本数> <ログのディレクトリ> <ネットのパス>', file=sys.stderr)
        return 1
    if not re.fullmatch(r'[1-9][0-9]*', sys.argv[2]):
        print('本数には正の整数が必要です', file=sys.stderr)
        return 1
    try:
        count = int(sys.argv[2])
        binary = Path(sys.argv[1]).resolve()
        logs = Path(sys.argv[3])
        logs.mkdir(parents=True, exist_ok=True)
        print(remote.bench(binary, count, logs, remote.Children(), Path(sys.argv[4]).resolve(), 'JCSA'))
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
