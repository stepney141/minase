#!/usr/bin/env python3
"""Alternate concurrency-N self-play with different environment settings; report elapsed, user/sys CPU, and user cycles per node."""
import argparse, os, re, resource, statistics, subprocess, tempfile
ap=argparse.ArgumentParser()
ap.add_argument('--cpus',default='8-19'); ap.add_argument('--concurrency',type=int,default=12); ap.add_argument('--rounds',type=int,default=3)
ap.add_argument('--max-ply',type=int,default=300); ap.add_argument('--bin',required=True)
ap.add_argument('--cwd',default='/home/stepney141/board-games/minase/data/worktrees/teacher-speedup')
ap.add_argument('variants',nargs='+',help='name:VAR=VAL,VAR=VAL or name:')
a=ap.parse_args()
EV='cpu_atom/cycles/u,cpu_atom/instructions/u,cpu_atom/dTLB-load-misses/u'
EV='cpu_atom/cycles/u,cpu_atom/instructions/u'
vs=[]
for v in a.variants:
    n,_,rest=v.partition(':')
    e={}
    if rest:
        for kv in rest.split(','):
            k,_,val=kv.partition('='); e[k]=val
    vs.append((n,e))
tmp=tempfile.mkdtemp(prefix='envc',dir='/home/stepney141/board-games/minase/target/teacher-speedup/run')
res={n:[] for n,_ in vs}
for r in range(a.rounds):
    for n,e in (vs if r%2==0 else vs[::-1]):
        out=f'{tmp}/{n}-{r}.mnsd'; st=f'{tmp}/{n}-{r}.stat'
        env=dict(os.environ); env.update(e)
        before=resource.getrusage(resource.RUSAGE_CHILDREN)
        p=subprocess.run(['taskset','-c',a.cpus,'perf','stat','-x,','-o',st,'-e',EV,'--',a.bin,'data','selfplay','generate','--output',out,'--seed','9010200000','--games',str(a.concurrency),'--concurrency',str(a.concurrency),'--nodes','20000','--random-moves','0','--max-ply',str(a.max_ply),'--hash-mb','16','--allow-dirty'],cwd=a.cwd,capture_output=True,text=True,env=env)
        after=resource.getrusage(resource.RUSAGE_CHILDREN)
        if p.returncode: raise SystemExit(p.stderr[-1500:])
        nodes=int(re.search(r'searched_nodes_total: (\d+)',p.stdout)[1]); el=float(re.search(r'elapsed_seconds: ([\d.]+)',p.stdout)[1])
        row={'nps':nodes/el,'el':el,'nodes':nodes,'user':after.ru_utime-before.ru_utime,'sys':after.ru_stime-before.ru_stime,'minflt':after.ru_minflt-before.ru_minflt}
        for l in open(st):
            f=l.strip().split(',')
            if len(f)>=3 and f[2] and not l.startswith('#'):
                try: row[f[2].split('/')[1]]=int(f[0])/nodes
                except ValueError: pass
        res[n].append(row); os.remove(out)
        print(f"r{r} {n} el={el:.2f} user={row['user']:.1f} sys={row['sys']:.1f} flt={row['minflt']} cyc={row.get('cycles',0):.0f}",flush=True)
print()
base=None
for n,_ in vs:
    rows=res[n]; g=lambda k: statistics.median(x[k] for x in rows)
    if base is None: base=g('nps')
    print(f"{n:12s} nps={g('nps'):.0f} x{g('nps')/base:.3f} el={g('el'):.2f} user={g('user'):.1f} sys={g('sys'):.2f} minflt={g('minflt'):.0f} cyc/node={g('cycles'):.0f} ins/node={g('instructions'):.0f}")
