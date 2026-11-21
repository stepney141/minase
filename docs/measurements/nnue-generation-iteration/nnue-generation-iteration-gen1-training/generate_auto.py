#!/usr/bin/env python3
"""出品が途絶えるたびに借り方を替えて、1世代の残りの教師ファイルを生成し続ける。

使い方: python3 generate_auto.py <世代N> <最後の番号> <教師の生成に使うネット> <UTCの絶対期限> <費用の上限（米ドル）> <除外するmachine_id（カンマ区切り）>
systemd-run --userのサービスとして起動し、出力をログへ追記させる。

制御スクリプト scripts/vast/nnue/teacher_generation.py は、合う出品がなくなると終了する。
本スクリプトは、その終了のたびに次の順で制御スクリプトを起動し直す（2026年11月20日の利用者の決定）。

1. オンデマンドで、時間単価1.5米ドル以下の出品を借りる。
2. 借りられなければ、中断可能インスタンスで、入札額3.5米ドル以下の出品を借りる。
3. どちらも借りられなければ、10分待って1へ戻る。

制御スクリプトは実行ディレクトリごとに引数を固定するので、実行ディレクトリを
data/nnue-generation-iteration/gen<N>-b<最初の番号>-<借り方> と名づけ、最初の番号と借り方の組ごとに分ける。
受け入れ済みの教師ファイルと生成ログは、連続生成スクリプトの実行ディレクトリの gen-<N>/teacher/ と gen-<N>/logs/ へ
ハードリンクで置き、まだない最小の番号を次の最初の番号にする。
回収済みの塊は、塊の数が最も多い実行ディレクトリから、起動する実行ディレクトリの remote/work/ へ写して引き継ぐ。
費用は、最初の起動時の口座の残高からの減少で数え、上限に達したら止まる。残高への反映は数分遅れる。
最初の起動時の残高は gen<N>-auto.credit-start に保存し、起動し直しても同じ値から数える。
制御スクリプトが「条件を満たす出品がありません」以外の理由で失敗した場合と、絶対期限まで30分を切った場合も止まる。
除外するmachine_idは、制御スクリプトの状態ファイルstate.jsonのexcluded_machine_idsへ起動の前に加える。
生成器が異常終了した機械を、利用者の決定で外すために使う（2026年11月20日にmachine_id 155327を除外した）。
"""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path('/home/stepney141/board-games/minase/data/nnue-generation-iteration')
WORKTREE = Path('/home/stepney141/board-games/minase/data/worktrees/nnue-generation-iteration')
MODES = (('on-demand', '1.5'), ('interruptible', '3.5'))
NO_OFFER = '条件を満たす出品がありません'


def log(message: str) -> None:
    print(f'{datetime.now(timezone.utc):%Y-%m-%dT%H:%M:%SZ} {message}', flush=True)


def credit() -> float:
    output = subprocess.run(['vastai', 'show', 'user', '--raw'], check=True, text=True,
                            stdout=subprocess.PIPE, timeout=600).stdout
    return float(json.loads(output)['credit'])


def legs(generation: int) -> list[Path]:
    """この世代の教師を生成した実行ディレクトリを返す。"""
    return [path for path in sorted(ROOT.glob(f'gen{generation}-*')) if (path / f'gen-{generation}/teacher').is_dir()]


def link_accepted(generation: int) -> None:
    """受け入れ済みの教師ファイルと生成ログを、連続生成スクリプトの実行ディレクトリへ置く。"""
    for name in ('teacher', 'logs'):
        (ROOT / f'gen-{generation}' / name).mkdir(parents=True, exist_ok=True)
    for leg in legs(generation):
        for book in (leg / f'gen-{generation}/teacher').glob('book-*.mnsd'):
            source_log = leg / f'gen-{generation}/logs/{book.stem}.log'
            target = ROOT / f'gen-{generation}/teacher/{book.name}'
            if target.exists():
                continue
            if not source_log.is_file():
                raise SystemExit(f'生成ログがありません: {source_log}')
            target_log = ROOT / f'gen-{generation}/logs/{source_log.name}'
            if not target_log.exists():
                target_log.hardlink_to(source_log)
            target.hardlink_to(book)


def first_missing(generation: int, last: int) -> int | None:
    for number in range(last + 1):
        if not (ROOT / f'gen-{generation}/teacher/book-{number}.mnsd').is_file():
            return number
    return None


def carry_chunks(generation: int, first: int, last: int, run_dir: Path) -> None:
    """未完成の本ごとに、塊の数が最も多い実行ディレクトリの回収済みの出力を引き継ぐ。"""
    for number in range(first, last + 1):
        name = f'book-{number}'
        target = run_dir / 'remote/work' / name
        best, best_count = None, len(list(target.glob('chunk-*.mnsd')))
        for leg in legs(generation):
            source = leg / 'remote/work' / name
            count = len(list(source.glob('chunk-*.mnsd')))
            if count > best_count and (source / 'config.json').is_file():
                best, best_count = source, count
        if best is not None:
            if target.exists():
                shutil.rmtree(target)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copytree(best, target)
            log(f'本{number}: {best}から{best_count}塊を引き継ぎます')


def exclude_machines(run_dir: Path, machines: list[int]) -> None:
    """制御スクリプトが借りない機械を、実行ディレクトリの状態ファイルへ加える。"""
    path = run_dir / 'state.json'
    state = json.loads(path.read_text()) if path.is_file() else {
        'rental_count': 0, 'rentals': [], 'excluded_machine_ids': [], 'current': None}
    state['excluded_machine_ids'] = sorted(set(state['excluded_machine_ids']) | set(machines))
    path.write_text(json.dumps(state) + '\n')


def run_leg(generation: int, first: int, last: int, mode: str, price: str, network: str,
            deadline: str, max_cost: str, excluded: list[int]) -> tuple[int, bool, str]:
    """制御スクリプトを1回動かし、終了コード、生成が進んだか、終了の理由を返す。"""
    run_dir = ROOT / f'gen{generation}-b{first}-{mode}'
    (run_dir / f'gen-{generation}/teacher').mkdir(parents=True, exist_ok=True)
    carry_chunks(generation, first, last, run_dir)
    exclude_machines(run_dir, excluded)
    events = run_dir / 'events.jsonl'
    before = len(events.read_text().splitlines()) if events.is_file() else 0
    commit = (ROOT / 'iteration-commit').read_text().strip()
    command = ['python3', 'scripts/vast/nnue/teacher_generation.py', 'run', '--run-dir', str(run_dir),
               '--repo', str(WORKTREE), '--ref', 'refs/nnue-generation-iteration/iteration', '--commit', commit,
               '--minase-bin', str(ROOT / 'bin/minase'), '--eval-file', network,
               '--generation', str(generation), '--first-book', str(first), '--last-book', str(last),
               '--seed-base', str(9000000000 + 100000000 * generation), '--seed-book-step', '100000',
               '--games', '7500', '--nodes', '20000', '--random-moves', '0', '--max-ply', '4000',
               '--hash-mb', '16', '--chunk-games', '250',
               '--rental', mode, '--max-price', price, '--deadline', deadline,
               '--max-cost', max_cost, '--max-rentals', '1000']
    log(f'起動: {run_dir.name}')
    with (ROOT / f'{run_dir.name}.log').open('a') as output:
        code = subprocess.run(command, cwd=WORKTREE, stdout=output, stderr=subprocess.STDOUT).returncode
    rows = [json.loads(line) for line in events.read_text().splitlines()[before:]] if events.is_file() else []
    progressed = any(row['event'] in ('generation_resumed', 'accepted') for row in rows)
    reasons = [row['reason'] for row in rows if row['event'] == 'finished']
    reason = reasons[-1] if reasons else '終了の記録がありません'
    log(f'終了: {run_dir.name} 終了コード={code} 生成の進行={progressed} 理由={reason}')
    return code, progressed, reason


def main(argv: list[str]) -> int:
    generation, last, network, deadline, budget = int(argv[0]), int(argv[1]), argv[2], argv[3], float(argv[4])
    excluded = [int(value) for value in argv[5].split(',')]
    end = datetime.strptime(deadline, '%Y-%m-%dT%H:%M:%SZ').replace(tzinfo=timezone.utc).timestamp()
    baseline = ROOT / f'gen{generation}-auto.credit-start'
    if not baseline.is_file():
        baseline.write_text(f'{credit()}\n')
    start_credit = float(baseline.read_text())
    log(f'開始: 残高={start_credit} 費用の上限={budget} 絶対期限={deadline}')
    mode_index = 0
    while True:
        link_accepted(generation)
        first = first_missing(generation, last)
        if first is None:
            log('完了: すべての教師ファイルがそろいました')
            return 0
        spent = start_credit - credit()
        if spent >= budget:
            log(f'停止: 費用が上限に達しました（{spent:.2f}米ドル）')
            return 1
        if end - time.time() < 1800:
            log('停止: 絶対期限まで30分を切りました')
            return 1
        mode, price = MODES[mode_index]
        code, progressed, reason = run_leg(generation, first, last, mode, price, network, deadline, argv[4],
                                           excluded)
        if code == 0:
            continue
        if reason != NO_OFFER:
            log(f'停止: 制御スクリプトが出品の不足以外の理由で終わりました（{reason}）')
            return 1
        if progressed:
            mode_index = 0
            continue
        mode_index += 1
        if mode_index == len(MODES):
            mode_index = 0
            log('待機: どちらの借り方でも借りられないので10分待ちます')
            time.sleep(600)


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
