import json, glob, statistics as st
import sys
R = '/home/stepney141/board-games/minase/data/matches/' + sys.argv[1] + '/pairs/'
rows = []
for p in sorted(glob.glob(R + '*.json')):
    d = json.load(open(p))
    for i, g in enumerate(d['games']):
        cc = g['candidate_color']
        t = g['termination']
        w = t.get('winner')
        res = 'draw' if w is None else ('C' if w == cc else 'B')
        ct = [x for x in g['turns'] if x['side'] == cc]
        bt = [x for x in g['turns'] if x['side'] != cc]
        def depths(ts):
            return [x['evaluation']['depth'] for x in ts if x.get('evaluation')]
        def cps(ts):
            return [x['evaluation']['score'].get('value') if x['evaluation']['score']['kind'] == 'cp' else None for x in ts if x.get('evaluation')]
        rows.append(dict(pair=d['pair_number'], game=i, res=res, cc=cc, reason=t.get('reason'), kind=t.get('kind'), plies=len(g['turns']),
                         cd=st.mean(depths(ct) or [0]), bd=st.mean(depths(bt) or [0]), ccpu=g['candidate_cpu_time_ns'] / 1e9, bcpu=g['baseline_cpu_time_ns'] / 1e9,
                         ccp=cps(ct), bcp=cps(bt), first=[x['response'].get('usi') for x in g['turns'][:14]]))
print('games', len(rows), 'cand wins', sum(r['res'] == 'C' for r in rows), 'base wins', sum(r['res'] == 'B' for r in rows), 'draws', sum(r['res'] == 'draw' for r in rows))
print('mean depth cand', round(st.mean(r['cd'] for r in rows), 2), 'base', round(st.mean(r['bd'] for r in rows), 2), 'cpu per game cand', round(st.mean(r['ccpu'] for r in rows), 1), 'base', round(st.mean(r['bcpu'] for r in rows), 1))
print('by plies bucket (cand wins/base wins/draw):')
for lo, hi in ((0, 100), (100, 200), (200, 300), (300, 400), (400, 600), (600, 2000)):
    s = [r for r in rows if lo <= r['plies'] < hi]
    print(' ', lo, hi, len(s), sum(r['res'] == 'C' for r in s), sum(r['res'] == 'B' for r in s), sum(r['res'] == 'draw' for r in s))
print('short games:')
for r in sorted(rows, key=lambda r: r['plies'])[:8]:
    print(' ', r['pair'], r['game'], r['plies'], r['res'], r['cc'], r['kind'], r['reason'], r['first'][:12], 'ccp', r['ccp'][-3:], 'bcp', r['bcp'][-3:])
def q(v, f):
    v = sorted(v)
    return v[int(len(v) * f)]
ca = [abs(x) for r in rows for x in r['ccp'] if x is not None and abs(x) < 20000]
ba = [abs(x) for r in rows for x in r['bcp'] if x is not None and abs(x) < 20000]
print('abs cp cand p50/p90', q(ca, .5), q(ca, .9), 'base p50/p90', q(ba, .5), q(ba, .9))
for ply in (10, 30, 60, 100, 150):
    c = []
    b = []
    for r in rows:
        if len(r['ccp']) > ply // 2 and len(r['bcp']) > ply // 2:
            x, y = r['ccp'][ply // 2], r['bcp'][ply // 2]
            if x is not None and y is not None and abs(x) < 20000 and abs(y) < 20000:
                c.append(x)
                b.append(y)
    print('ply', ply, 'n', len(c), 'mean cand cp (own view)', round(st.mean(c)), 'mean base cp (own view)', round(st.mean(b)), 'std', round(st.pstdev(c)), round(st.pstdev(b)))
