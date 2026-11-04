#!/usr/bin/env python3
"""Alternate concurrency-N self-play on a CPU set and report NPS and E-core counters per node.

Usage: ecore_cmp.py --cpus 8-19 --concurrency 12 --rounds 3 --tag t name=binary[:cwd] ...
"""
import argparse, json, os, re, statistics, subprocess, tempfile, time
ap=argparse.ArgumentParser()
ap.add_argument('--cpus',default='8-19'); ap.add_argument('--concurrency',type=int,default=12)
ap.add_argument('--games',type=int,default=None); ap.add_argument('--rounds',type=int,default=3)
ap.add_argument('--max-ply',type=int,default=200); ap.add_argument('--nodes',type=int,default=20000)
ap.add_argument('--hash-mb',type=int,default=16); ap.add_argument('--seed',type=int,default=9010200000)
ap.add_argument('--events',default='cpu_atom/cycles/u,cpu_atom/instructions/u,cpu_atom/mem_load_uops_retired.l2_miss/u,cpu_atom/longest_lat_cache.miss/u,cpu_atom/mem_bound_stalls_load.all/u,cpu_atom/mem_bound_stalls_load.llc_miss/u')
ap.add_argument('--cwd',default='/home/stepney141/board-games/minase/data/worktrees/teacher-speedup')
ap.add_argument('--env',action='append',default=[])
ap.add_argument('versions',nargs='+')
a=ap.parse_args()
vers=[v.split('=',1) for v in a.versions]
res={n:[] for n,_ in vers}
tmp=tempfile.mkdtemp(prefix='ecmp',dir=os.environ.get('SCRATCH','/home/stepney141/board-games/minase/target/teacher-speedup/run'))
env=dict(os.environ)
for kv in a.env:
    k,v=kv.split('=',1); env[k]=v
for r in range(a.rounds):
    order=vers if r%2==0 else vers[::-1]
    for name,binary in order:
        out=f'{tmp}/{name}-{r}.mnsd'; stat=f'{tmp}/{name}-{r}.stat'
        games=a.games or a.concurrency
        cmd=['taskset','-c',a.cpus]
        if a.events: cmd+=['perf','stat','-x,','-o',stat,'-e',a.events,'--']
        cmd+=[binary,'data','selfplay','generate','--output',out,'--seed',str(a.seed),'--games',str(games),'--concurrency',str(a.concurrency),'--nodes',str(a.nodes),'--random-moves','0','--max-ply',str(a.max_ply),'--hash-mb',str(a.hash_mb),'--allow-dirty']
        p=subprocess.run(cmd,cwd=a.cwd,capture_output=True,text=True,env=env)
        if p.returncode: raise SystemExit(p.stderr[-2000:])
        nodes=int(re.search(r'searched_nodes_total: (\d+)',p.stdout)[1]); el=float(re.search(r'elapsed_seconds: ([\d.]+)',p.stdout)[1])
        row={'nps':nodes/el,'el':el,'nodes':nodes}
        if a.events:
            for l in open(stat):
                f=l.strip().split(',')
                if len(f)>=3 and f[2] and not l.startswith('#'):
                    try: row[f[2]]=int(f[0])/nodes
                    except ValueError: pass
        res[name].append(row)
        os.remove(out)
        print(f'round={r} {name} nps={row["nps"]:.0f} el={el:.2f} nodes={nodes}',flush=True)
print()
base=None
for name,_ in vers:
    rows=res[name]; med=statistics.median(x['nps'] for x in rows)
    if base is None: base=med
    keys=[k for k in rows[0] if k not in('nps','el','nodes')]
    c=' '.join(f"{k.split('/')[1] if '/' in k else k}={statistics.median(x[k] for x in rows):.2f}" for k in keys)
    print(f"{name:12s} nps_med={med:.0f} ratio={med/base:.3f} range={min(x['nps'] for x in rows):.0f}-{max(x['nps'] for x in rows):.0f} {c}")
