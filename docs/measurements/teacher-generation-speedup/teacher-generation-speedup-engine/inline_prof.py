#!/usr/bin/env python3
"""Attribute frame-pointer perf samples to functions, resolving inline frames with addr2line.

Non-leaf frames are return addresses, so they are looked up at ip-1.
Usage: inline_prof.py <perf.data> <binary> [--top N] [--focus REGEX]
"""
import argparse, collections, re, subprocess, sys
ap = argparse.ArgumentParser()
ap.add_argument('data'); ap.add_argument('binary')
ap.add_argument('--top', type=int, default=70)
ap.add_argument('--focus', default=None)
ap.add_argument('--exclude', default=r'^(core::|alloc::|std::|<core::|<alloc::|<std::|<\[|<&|<i32|<u|<i|<bool)')
ap.add_argument('--callers', default=None, help='for frames matching REGEX print the distribution of their nearest caller (inline-resolved, after excluding std wrappers)')
ap.add_argument('--callees', default=None, help='for frames matching REGEX print inclusive share of nearest callee')
ap.add_argument('--raw', action='store_true')
ap.add_argument('--tree', default=None)
ap.add_argument('--tree-depth', type=int, default=3)
ap.add_argument('--tree-min', type=float, default=0.25)
args = ap.parse_args()
bn = args.binary.split('/')[-1]
proc = subprocess.run(['perf','script','-i',args.data,'-F','ip,sym,dso','--no-inline'],capture_output=True,text=True)
samples=[]; cur=[]
for line in proc.stdout.splitlines():
    s=line.strip()
    if not s:
        if cur: samples.append(cur); cur=[]
        continue
    parts=s.split()
    dso=parts[-1] if parts[-1].startswith('(') else ''
    cur.append((int(parts[0],16),dso))
if cur: samples.append(cur)
offset=0
for l in subprocess.run(['perf','script','-i',args.data,'--show-mmap-events'],capture_output=True,text=True).stdout.splitlines():
    if 'PERF_RECORD_MMAP' in l and bn in l and 'r-xp' in l:
        m=re.search(r'\[(0x[0-9a-f]+)\((0x[0-9a-f]+)\) @ (0x[0-9a-f]+|0) ',l)
        if m:
            mstart=int(m[1],16); pgoff=int(m[3],16)
            hdr=subprocess.run(['readelf','-lW',args.binary],capture_output=True,text=True).stdout
            for hl in hdr.splitlines():
                f=hl.split()
                if len(f)>=8 and f[0]=='LOAD' and 'E' in hl.split(f[5],1)[1]:
                    seg_off=int(f[1],16); seg_va=int(f[2],16); break
            offset=mstart-pgoff+seg_off-seg_va; break
keys=set()
for st in samples:
    for i,(ip,dso) in enumerate(st):
        if bn in dso:
            keys.add(ip-offset-(0 if i==0 else 1))
keys=sorted(keys)
res=subprocess.run(['addr2line','-i','-f','-C','-a','-e',args.binary],input='\n'.join(hex(k) for k in keys)+'\n',capture_output=True,text=True)
stacks={}; idx=-1; lines=res.stdout.splitlines(); i=0
while i<len(lines):
    l=lines[i]
    if l.startswith('0x') and ' ' not in l:
        idx+=1; stacks[keys[idx]]=[]; i+=1; continue
    stacks[keys[idx]].append(l); i+=2
def norm(fn):
    fn=re.sub(r'::h[0-9a-f]{16}$','',fn)
    fn=fn.replace('minase::search::alphabeta::','').replace('minase_core::','mc:').replace('minase::','')
    fn=re.sub(r'<(\w+::)*Network<[^>]*>[^>]*>','<Network>',fn)
    return fn
excl=re.compile(args.exclude)
total=0; incl=collections.Counter(); selfc=collections.Counter()
focus=re.compile(args.focus) if args.focus else None
callers=collections.Counter(); callees=collections.Counter()
creg=re.compile(args.callers) if args.callers else None
creg2=re.compile(args.callees) if args.callees else None
allusers=[]
for st in samples:
    total+=1
    names=[]
    for i,(ip,dso) in enumerate(st):
        fr=stacks.get(ip-offset-(0 if i==0 else 1)) if bn in dso else None
        if fr:
            names.extend(norm(f) for f in fr)
        else:
            names.append('['+dso.strip('()').split('/')[-1]+']')
    if focus and not any(focus.search(n) for n in names): continue
    user=[n for n in names if not excl.search(n)] if not args.raw else names
    if not user: continue
    allusers.append(user)
    selfc[user[0]]+=1
    for n in set(user): incl[n]+=1
    if creg:
        seen=set()
        for k,n in enumerate(user):
            if creg.search(n) and k+1<len(user):
                key=(n,user[k+1])
                if key not in seen:
                    callers[key]+=1; seen.add(key)
    if creg2:
        seen=set()
        for k,n in enumerate(user):
            if creg2.search(n):
                key=(n,user[k-1] if k>0 else '(self)')
                if key not in seen:
                    callees[key]+=1; seen.add(key)
if args.tree:
    # exclusive-root call tree: for each sample, take the innermost frame matching --tree, walk toward the leaf
    tree=collections.Counter(); treg=re.compile(args.tree)
    for u in allusers:
        idxs=[k for k,n in enumerate(u) if treg.search(n)]
        if not idxs: continue
        k=idxs[0]
        path=[]
        for j in range(k,-1,-1):
            path.append(u[j])
            tree[tuple(path[:args.tree_depth+1])]+=1 if len(path)<=args.tree_depth+1 else 0
    def rec(prefix,ind):
        kids=[(p,c) for p,c in tree.items() if len(p)==len(prefix)+1 and p[:len(prefix)]==prefix]
        kids.sort(key=lambda x:-x[1])
        for p,c in kids:
            if 100*c/total<args.tree_min: continue
            print(f"{100*c/total:7.2f}  {'  '*ind}{p[-1][:90]}")
            if len(p)<=args.tree_depth: rec(p,ind+1)
    roots=collections.Counter()
    for p,c in tree.items():
        if len(p)==1: roots[p]=c
    for p,c in roots.most_common():
        print(f"{100*c/total:7.2f}  {p[0][:90]}")
        rec(p,1)
    sys.exit(0)
print(f'total samples {total}')
print(f'{"incl%":>7} {"self%":>7}  function')
for n,c in incl.most_common(args.top):
    print(f'{100*c/total:7.2f} {100*selfc[n]/total:7.2f}  {n[:110]}')
if creg:
    print('--- callers')
    for (n,c),v in callers.most_common(60):
        print(f'{100*v/total:7.2f}  {n[:50]}  <- {c[:70]}')
if creg2:
    print('--- callees')
    for (n,c),v in callees.most_common(60):
        print(f'{100*v/total:7.2f}  {n[:50]}  -> {c[:70]}')
