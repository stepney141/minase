"""保存済みの超長手数の対局を1手ずつMCTSのエンジンへ与え、position送信からbestmoveまでの時間を手数帯ごとに集計する。"""
import json, subprocess, sys, time, statistics
engine_path, net, pair_file, game_index, cache_dir = sys.argv[1:6]
pair = json.load(open(pair_file))
moves = pair['opening']['moves'] + [t['response']['usi'] for t in pair['games'][int(game_index)]['turns'] if t['response']['kind'] == 'move']
proc = subprocess.Popen([engine_path, '--protocol', 'usi', '--rules', 'engine-default', '--search', 'mcts', '--nn-model', net],
                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
def send(s): proc.stdin.write(s + '\n'); proc.stdin.flush()
def until(prefix):
    while True:
        line = proc.stdout.readline()
        if not line: raise RuntimeError('engine exited')
        if line.startswith(prefix): return line
send('usi'); until('usiok')
send(f'setoption name NnCacheDir value {cache_dir}'); send('setoption name NnBatchSize value 64')
send('isready'); until('readyok'); send('usinewgame')
times = []
for ply in range(len(moves)):
    started = time.monotonic()
    send('position startpos' + (' moves ' + ' '.join(moves[:ply]) if ply else ''))
    send('go nodes 64')
    until('bestmove')
    times.append(time.monotonic() - started)
send('quit'); proc.wait(timeout=60)
bands = [(0, 100), (1000, 1100), (2000, 2100), (3000, 3100), (len(times) - 100, len(times))]
print(json.dumps({'plies': len(times), 'exit_code': proc.returncode,
  'bands_ms': {f'{a}-{b}': {'median': round(statistics.median(times[a:b]) * 1000, 2), 'max': round(max(times[a:b]) * 1000, 2)} for a, b in bands}}, indent=2))
