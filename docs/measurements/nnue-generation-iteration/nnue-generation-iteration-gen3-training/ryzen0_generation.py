#!/usr/bin/env python3
"""借りた機械と並行して、手元の別の機械ryzen0で1世代の教師ファイルを番号の大きい側から生成して受け入れる。

使い方: python3 ryzen0_generation.py <世代N> <最後の番号> <ryzen0に置いたネットのパス> <ネットのSHA-256> <同時対局数> \
          <借りた機械での生成のサービス名>
systemd-run --userのサービスとして作業機で起動し、出力をログへ追記させる。

2026年11月23日に利用者が、vast.aiの残高が尽きた後の教師の生成をryzen0で続けることと、
残高が尽きる前からryzen0を並行して動かすことを決めた。
ryzen0（AMD Ryzen Threadripper 2990WX、32コア64スレッド）には、反復のコミットを /var/tmp/minase-ngi に
git bundleから展開し、cargo build --release --locked --bin minase でビルドした生成器を置く。

借りた機械は番号の小さい側から、ryzen0は番号の大きい側から生成する。
本スクリプトは、どの実行ディレクトリ data/nnue-generation-iteration/gen<N>-*/gen-<N>/teacher/ にもまだない番号のうち
最大のものを選び、SSHでryzen0の生成器を1本ずつ動かして、生成中の番号を gen<N>-ryzen0/current に書く。
借りた機械での生成のサービスが動いている間は、残りが1本だけになったらその1本を借りた機械に任せて待つ。
借りた機械がryzen0の生成中の番号に届く前に止める処理は、作業機の監視が current を読んで行う。
生成の条件とシードは、借りた機械での生成（scripts/vast/nnue/teacher_generation.py）と同じである。
回収済みの塊が作業機の実行ディレクトリ gen<N>-*/remote/work/book-<i>/ にあれば、ryzen0の出力先が空のときに限って送り、
続きから生成させる。生成器は出力先の塊から再開するので、SSHが切れても同じ番号をやり直せば続きから進む。
完成した教師ファイルと生成ログは gen<N>-ryzen0/remote/ へ回収し、借りた機械の場合と同じ common.inspect_teacher の
検査に通ったものだけを、gen<N>-ryzen0/gen-<N>/ へハードリンクで置く。
すべての番号がそろったら、全部の実行ディレクトリの教師ファイルと生成ログを、連続生成スクリプトの実行ディレクトリ
gen-<N>/teacher/ と gen-<N>/logs/ へハードリンクで置いて終わる。
検査に通らない場合と、生成器が0以外で終わった場合は、止まって報告する。
"""

from datetime import datetime, timezone
import os
from pathlib import Path
import shlex
import subprocess
import sys
import time

ROOT = Path('/home/stepney141/board-games/minase/data/nnue-generation-iteration')
WORKTREE = Path('/home/stepney141/board-games/minase/data/worktrees/nnue-generation-iteration')
sys.path.insert(0, str(WORKTREE / 'scripts/vast/common'))
import common  # noqa: E402

HOST = 'ryzen0'
REMOTE_REPOSITORY = '/var/tmp/minase-ngi'
REMOTE_WORK = '/var/tmp/minase-ngi-work'
SETTINGS = {'games': 7500, 'nodes': 20000, 'random_moves': 0, 'max_ply': 4000, 'hash_mb': 16, 'chunk_games': 250}
SSH = ['ssh', '-o', 'BatchMode=yes', '-o', 'ServerAliveInterval=30', '-o', 'ServerAliveCountMax=6', HOST]


def log(message: str) -> None:
    print(f'{datetime.now(timezone.utc):%Y-%m-%dT%H:%M:%SZ} {message}', flush=True)


def remote(command: str, **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run([*SSH, command], check=True, text=True, **kwargs)


def legs(generation: int) -> list[Path]:
    """この世代の教師を受け入れた実行ディレクトリを返す。"""
    return [path for path in sorted(ROOT.glob(f'gen{generation}-*')) if (path / f'gen-{generation}/teacher').is_dir()]


def missing(generation: int, last: int) -> list[int]:
    """どの実行ディレクトリにも受け入れ済みの教師ファイルがない番号を、小さい順に返す。"""
    accepted = {int(path.stem.split('-')[1]) for leg in legs(generation)
                for path in (leg / f'gen-{generation}/teacher').glob('book-*.mnsd')}
    return [number for number in range(last + 1) if number not in accepted]


def carry_chunks(generation: int, number: int, directory: str) -> None:
    """ryzen0の出力先が空なら、作業機に回収済みの塊が最も多い実行ディレクトリから送る。"""
    listing = remote(f'mkdir -p {directory} && ls {directory}', stdout=subprocess.PIPE).stdout.split()
    if listing:
        return
    best, best_count = None, 0
    for leg in sorted(ROOT.glob(f'gen{generation}-*')):
        source = leg / 'remote/work' / f'book-{number}'
        count = len(list(source.glob('chunk-*.mnsd')))
        if count > best_count and (source / 'config.json').is_file():
            best, best_count = source, count
    if best is None:
        return
    files = [str(best / 'config.json')]
    for chunk in sorted(best.glob('chunk-*.mnsd')):
        summary = chunk.with_suffix('.json')
        if summary.is_file():
            files.extend([str(chunk), str(summary)])
    subprocess.run(['rsync', '-a', *files, f'{HOST}:{directory}/'], check=True)
    log(f'本{number}: {best}から{best_count}塊を引き継ぎます')


def link_all(generation: int) -> None:
    """全部の実行ディレクトリの教師ファイルと生成ログを、連続生成スクリプトの実行ディレクトリへ置く。"""
    chain = ROOT / f'gen-{generation}'
    for name in ('teacher', 'logs'):
        (chain / name).mkdir(parents=True, exist_ok=True)
    for leg in legs(generation):
        for book in (leg / f'gen-{generation}/teacher').glob('book-*.mnsd'):
            source_log = leg / f'gen-{generation}/logs/{book.stem}.log'
            if not source_log.is_file():
                raise SystemExit(f'生成ログがありません: {source_log}')
            for source, target in ((source_log, chain / 'logs' / source_log.name), (book, chain / 'teacher' / book.name)):
                if not target.exists():
                    os.link(source, target)


def main(argv: list[str]) -> int:
    generation, last, network, checksum, concurrency = int(argv[0]), int(argv[1]), argv[2], argv[3], int(argv[4])
    rented_service = argv[5]
    commit = (ROOT / 'iteration-commit').read_text().strip()
    seed_base = 9000000000 + 100000000 * generation
    run_dir = ROOT / f'gen{generation}-ryzen0'
    for directory in (run_dir / 'remote/teacher', run_dir / 'remote/logs', run_dir / f'gen-{generation}/teacher',
                      run_dir / f'gen-{generation}/logs'):
        directory.mkdir(parents=True, exist_ok=True)
    current = run_dir / 'current'
    head = remote(f'git -C {REMOTE_REPOSITORY} rev-parse HEAD', stdout=subprocess.PIPE).stdout.strip()
    remote_sum = remote(f'sha256sum {shlex.quote(network)}', stdout=subprocess.PIPE).stdout.split()[0]
    if head != commit or remote_sum != checksum:
        raise SystemExit(f'ryzen0のコミットまたはネットが一致しません: {head} {remote_sum}')
    while True:
        remaining = missing(generation, last)
        if not remaining:
            break
        rented_active = subprocess.run(['systemctl', '--user', 'is-active', '-q', rented_service]).returncode == 0
        if rented_active and len(remaining) == 1:
            # 残りの1本は、借りた機械が生成している番号である。
            time.sleep(300)
            continue
        number = remaining[-1]
        current.write_text(f'{number}\n')
        directory = f'{REMOTE_WORK}/gen-{generation}/book-{number}'
        carry_chunks(generation, number, directory)
        seed = seed_base + 100000 * number
        command = (f'cd {REMOTE_REPOSITORY} && nice -n 10 target/release/minase data selfplay generate '
                   f'--eval-file {shlex.quote(network)} --output-dir {directory} --seed {seed} '
                   f'--games {SETTINGS["games"]} --nodes {SETTINGS["nodes"]} --random-moves {SETTINGS["random_moves"]} '
                   f'--concurrency {concurrency} --max-ply {SETTINGS["max_ply"]} --hash-mb {SETTINGS["hash_mb"]} '
                   f'--chunk-games {SETTINGS["chunk_games"]} >> {directory}.log 2>&1')
        log(f'本{number}: 生成を始めます')
        while True:
            code = subprocess.run([*SSH, command]).returncode
            if code == 0:
                break
            if code != 255:
                raise SystemExit(f'本{number}: 生成器が終了コード{code}で終わりました')
            log(f'本{number}: SSHが切れたので、ryzen0の生成器が終わるのを待ってから、同じ出力先で続きを生成します')
            # 角括弧は、pgrepを起動したシェル自身のコマンドラインに一致させないためのものである。
            while subprocess.run([*SSH, f'pgrep -f -- "[-]-output-dir {directory} " > /dev/null']).returncode != 1:
                time.sleep(60)
        path = run_dir / f'remote/teacher/book-{number}.mnsd'
        log_path = run_dir / f'remote/logs/book-{number}.log'
        subprocess.run(['rsync', '-a', f'{HOST}:{directory}/teacher.mnsd', str(path)], check=True)
        subprocess.run(['rsync', '-a', f'{HOST}:{directory}.log', str(log_path)], check=True)
        expected = {'generation_commit': commit, 'network_checksum': checksum, 'rule_set': 'L1,L2,P0,P3,R1,E1,E3',
                    'teacher_search_budget': SETTINGS['nodes'], 'seed': seed}

        def inspect(arguments: list[str], **kwargs) -> subprocess.CompletedProcess:
            return common.run_timed(arguments, 600, **kwargs)

        _, entries = common.inspect_teacher(ROOT / 'bin/minase', path, log_path, expected, SETTINGS, inspect)
        target = run_dir / f'gen-{generation}'
        os.link(log_path, target / 'logs' / log_path.name)
        os.link(path, target / 'teacher' / path.name)
        remote(f'rm -r {directory} {directory}.log')
        log(f'本{number}: 受け入れました 記録局面数={entries}')
    current.unlink(missing_ok=True)
    link_all(generation)
    log('完了: すべての教師ファイルがそろいました')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
