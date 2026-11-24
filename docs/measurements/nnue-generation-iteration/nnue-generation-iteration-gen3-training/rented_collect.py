#!/usr/bin/env python3
"""vast.aiの認証が切れて制御スクリプトが終了した後に、借りたままの生成機から教師ファイルを回収して受け入れる。

使い方: python3 rented_collect.py <世代N> <最後の番号> <実行ディレクトリの名前> <ネットのSHA-256> <UTCの停止時刻>
systemd-run --userのサービスとして作業機で起動し、出力をログへ追記させる。

2026年11月24日に、第3世代の生成の途中でvast.aiの2段階認証のセッションが切れ、制御スクリプト
scripts/vast/nnue/teacher_generation.py が、生成機を削除できないまま終了した。
生成機の作業プロセスは制御スクリプトがなくても生成を続けるので、本スクリプトが、認証の要らないSSHだけで次を行う。

- 60秒おきに、生成機の teacher/gen-<N>/、logs/gen-<N>/、および work/gen-<N>/ を、実行ディレクトリの remote/ の下へ回収する。
- そろった教師ファイルと生成ログを、制御スクリプトと同じ common.inspect_teacher の検査にかけ、通ったものを
  実行ディレクトリの gen-<N>/teacher/ と gen-<N>/logs/ へハードリンクで置く。
- どの実行ディレクトリにもまだない最小の番号が、ryzen0の生成中の番号（gen<N>-ryzen0/current）以上になるか、
  残りの番号がなくなるか、指定の停止時刻になったら、最後の回収の後に、生成機の generation_worker.py stop で
  生成機を停止する。この停止はコンテナ専用の鍵を使うので、口座の認証を要らない。

生成機の削除は口座の認証を要るので、本スクリプトは行わない。認証を更新した後に、利用者か作業者が削除する。
"""

from datetime import datetime, timezone
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path('/home/stepney141/board-games/minase/data/nnue-generation-iteration')
WORKTREE = Path('/home/stepney141/board-games/minase/data/worktrees/nnue-generation-iteration')
sys.path.insert(0, str(WORKTREE / 'scripts/vast/common'))
import common  # noqa: E402

REMOTE = '/workspace/job'
SETTINGS = {'games': 7500, 'nodes': 20000, 'random_moves': 0, 'max_ply': 4000, 'hash_mb': 16, 'chunk_games': 250}


def log(message: str) -> None:
    print(f'{datetime.now(timezone.utc):%Y-%m-%dT%H:%M:%SZ} {message}', flush=True)


def missing(generation: int, last: int) -> list[int]:
    """どの実行ディレクトリにも受け入れ済みの教師ファイルがない番号を、小さい順に返す。"""
    accepted = {int(path.stem.split('-')[1])
                for path in ROOT.glob(f'gen{generation}-*/gen-{generation}/teacher/book-*.mnsd')}
    return [number for number in range(last + 1) if number not in accepted]


def collect(host: str, port: int, generation: int, run_dir: Path) -> bool:
    """生成機の3つのディレクトリを回収し、すべて成功したかを返す。"""
    shell = 'ssh ' + ' '.join(common.ssh_options(port, '-p'))
    success = True
    for name in ('work', 'teacher', 'logs'):
        command = ['rsync', '-a', '--timeout=120', '-e', shell, '--exclude', '*.tmp',
                   '--exclude', '.selfplay_generate.lock', '--exclude', 'teacher.mnsd',
                   f'root@{host}:{REMOTE}/{name}/gen-{generation}/', str(run_dir / 'remote' / name) + '/']
        code = subprocess.run(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode
        if code not in (0, 24):
            success = False
    return success


def accept(generation: int, last: int, run_dir: Path, commit: str, checksum: str) -> None:
    seed_base = 9000000000 + 100000000 * generation
    for number in range(last + 1):
        path = run_dir / f'remote/teacher/book-{number}.mnsd'
        log_path = run_dir / f'remote/logs/book-{number}.log'
        target = run_dir / f'gen-{generation}/teacher/book-{number}.mnsd'
        if target.exists() or not path.is_file() or not log_path.is_file():
            continue
        if number not in missing(generation, last):
            continue
        expected = {'generation_commit': commit, 'network_checksum': checksum, 'rule_set': 'L1,L2,P0,P3,R1,E1,E3',
                    'teacher_search_budget': SETTINGS['nodes'], 'seed': seed_base + 100000 * number}

        def inspect(arguments: list[str], **kwargs) -> subprocess.CompletedProcess:
            return common.run_timed(arguments, 600, **kwargs)

        _, entries = common.inspect_teacher(ROOT / 'bin/minase', path, log_path, expected, SETTINGS, inspect)
        os.link(log_path, run_dir / f'gen-{generation}/logs' / log_path.name)
        os.link(path, target)
        log(f'本{number}: 受け入れました 記録局面数={entries}')


def main(argv: list[str]) -> int:
    generation, last, name, checksum, stop_at = int(argv[0]), int(argv[1]), argv[2], argv[3], argv[4]
    end = datetime.strptime(stop_at, '%Y-%m-%dT%H:%M:%SZ').replace(tzinfo=timezone.utc).timestamp()
    run_dir = ROOT / name
    commit = (ROOT / 'iteration-commit').read_text().strip()
    rental = json.loads((run_dir / 'state.json').read_text())['rentals'][-1]
    host, port = rental['address']
    log(f'開始: 生成機 {rental["instance_id"]} {host}:{port} 停止時刻={stop_at}')
    failed_since = None
    while True:
        collected = collect(host, port, generation, run_dir)
        accept(generation, last, run_dir, commit, checksum)
        if collected:
            failed_since = None
        else:
            failed_since = failed_since or time.time()
            if time.time() - failed_since >= 1800:
                log('停止: 回収の失敗が30分続きました。生成機の状態を確認してください')
                return 1
        remaining = missing(generation, last)
        current = ROOT / f'gen{generation}-ryzen0/current'
        ryzen0 = int(current.read_text()) if current.is_file() else last + 1
        reason = None
        if not remaining:
            reason = '残りの番号がなくなりました'
        elif remaining[0] >= ryzen0:
            reason = f'最小の残りの番号{remaining[0]}がryzen0の生成中の番号{ryzen0}に届きました'
        elif time.time() >= end:
            reason = '停止時刻になりました'
        if reason is not None and collected:
            log(f'生成機を停止します: {reason}')
            shell = ['ssh', *common.ssh_options(port, '-p'), f'root@{host}',
                     f'python3 {REMOTE}/generation_worker.py stop']
            code = subprocess.run(shell).returncode
            log(f'停止の要求の終了コード={code}。生成機の削除は、認証を更新した後に行ってください')
            return 0 if code == 0 else 1
        time.sleep(60)


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
