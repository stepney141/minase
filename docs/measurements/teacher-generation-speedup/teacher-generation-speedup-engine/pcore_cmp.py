#!/usr/bin/env python3
"""Alternate single-thread self-play on one P-core; compare NPS and cycles/instructions per node; check identical summaries."""
import argparse, os, re, statistics, subprocess, tempfile, hashlib
ap=argparse.ArgumentParser()
ap.add_argument('--cpu',default='3'); ap.add_argument('--rounds',type=int,default=3); ap.add_argument('--games',type=int,default=2)
ap.add_argument('--max-ply',type=int,default=200); ap.add_argument('--check-hash',action='store_true')
ap.add_argument('--cwd',default='/home/stepney141/board-games/minase/target/teacher-speedup/verify/cwd')
ap.add_argument('versions',nargs='+')
a=ap.parse_args()
EV='cpu_core/cycles/u,cpu_core/instructions/u,cpu_core/mem_load_retired.l1_miss/u,cpu_core/mem_load_retired.l2_miss/u,cpu_core/mem_load_retired.l3_miss/u,cpu_core/branch-misses/u'
vers=[v.split('=',1) for v in a.versions]
tmp=tempfile.mkdtemp(prefix='pcmp',dir=None)
res={n:[] for n,_ in vers}; hashes={}
for r in range(a.rounds):
    for name,b in (vers if r%2==0 else vers[::-1]):
        out=f'{tmp}/{name}-{r}.mnsd'; st=f'{tmp}/{name}-{r}.stat'
        cmd=['taskset','-c',a.cpu,'perf','stat','-x,','-o',st,'-e',EV,'--',b,'data','selfplay','generate','--output',out,'--seed','9010200000','--games',str(a.games),'--concurrency','1','--nodes','20000','--random-moves','0','--max-ply',str(a.max_ply),'--hash-mb','16','--allow-dirty']
        p=subprocess.run(cmd,cwd=a.cwd,capture_output=True,text=True)
        if p.returncode: raise SystemExit(p.stderr[-1500:])
        nodes=int(re.search(r'searched_nodes_total: (\d+)',p.stdout)[1]); el=float(re.search(r'elapsed_seconds: ([\d.]+)',p.stdout)[1])
        row={'nps':nodes/el,'nodes':nodes}
        for l in open(st):
            f=l.strip().split(',')
            if len(f)>=3 and f[2] and not l.startswith('#'):
                try: row[f[2].split('/')[1]]=int(f[0])/nodes
                except ValueError: pass
        res[name].append(row)
        hashes[name]=hashlib.sha256(open(out,'rb').read()).hexdigest()[:16]
        os.remove(out)
base=None
for name,_ in vers:
    rows=res[name]; med=statistics.median(x['nps'] for x in rows)
    cyc=statistics.median(x['cycles'] for x in rows); ins=statistics.median(x['instructions'] for x in rows)
    if base is None: base=(med,cyc,ins)
    others=' '.join(f"{k.split('.')[-1]}={statistics.median(x[k] for x in rows):.2f}" for k in rows[0] if k not in('nps','nodes','cycles','instructions'))
    print(f"{name:14s} nps={med:9.0f} x{med/base[0]:.3f} [{min(x['nps'] for x in rows):.0f}-{max(x['nps'] for x in rows):.0f}] cyc/node={cyc:8.1f} ({100*(cyc/base[1]-1):+.1f}%) ins/node={ins:8.1f} ({100*(ins/base[2]-1):+.1f}%) {others} nodes={rows[0]['nodes']} sha={hashes[name]}")
