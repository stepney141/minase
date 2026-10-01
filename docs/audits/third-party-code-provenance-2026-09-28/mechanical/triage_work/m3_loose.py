import sys;sys.path.insert(0,'triage_work')
from common import *
import collections
from fractions import Fraction
rows=[]
with open('m3_hits.tsv') as f:
    r=csv.reader(f,delimiter='\t');h=next(r)
    for row in r:
        if row[12].startswith('定数が2種類以下'): continue
        cp=parse(row[1])
        if VENDOR.search(cp['path']): continue
        rows.append(row)
tables={int(x[10]) for x in rows}|{int(x[11]) for x in rows}
TAB={}
for l in open('m3_arrays.jsonl'):
    d=json.loads(l)
    if d['table_id'] in tables: TAB[d['table_id']]=d
def num(v):
    try: return Fraction(v)
    except Exception: return None
out={}
for row in rows:
    mp=parse(row[0]);cp=parse(row[1]);L=int(row[4])
    mt=TAB[int(row[10])];ct=TAB[int(row[11])]
    off=int(row[6]);coff=int(row[7])
    if row[3] in ('subsequence','exact','scaled'):
        mv=mt['values'][off:off+L]; cv=ct['values'][coff:coff+L]
    else: mv=mt['values'];cv=ct['values']
    nm=[num(v) for v in mv]
    mono = all(x is not None for x in nm) and (all(a<b for a,b in zip(nm,nm[1:])) or all(a>b for a,b in zip(nm,nm[1:])))
    # arithmetic progression
    ap = all(x is not None for x in nm) and len({b-a for a,b in zip(nm,nm[1:])})==1
    small = all(x is not None and abs(x)<=12 for x in nm)
    nd=min(len(set(mv)),len(set(cv)))
    key=(mp['path'],cp['repo'].split(' (')[0],cp['path'])
    rec=dict(len=L,nd=nd,type=row[3],scale=row[5],mono=mono,ap=ap,small=small,note=row[12][:12],mt=mtest(mt['record'],mp['path'],mp['line']),mpath=mp['path'],mline=mp['line'],crepo=cp['repo'],cpath=cp['path'],cline=cp['line'],lic=row[2],mprev=','.join(mv[:14])[:110],cprev=','.join(cv[:14])[:110],mloc=row[0],cloc=row[1])
    if key not in out or (nd,L)>(out[key]['nd'],out[key]['len']): out[key]=rec
L=sorted(out.values(),key=lambda x:(-x['nd'],-x['len']))
json.dump(L,open('triage_work/m3_loose.json','w'),ensure_ascii=False)
print(len(rows),len(L))
n=0
for x in L:
    if x['ap'] or x['small']: continue
    n+=1
    if n>int(sys.argv[1]): break
    print(n,x['len'],x['nd'],x['type'],x['scale'],'mono' if x['mono'] else '','T' if x['mt'] else '',x['note'],x['mpath'],x['mline'],'|',x['crepo'][:25],x['cpath'][-45:],x['cline'],x['lic'][:8],'|',x['mprev'][:80],'||',x['cprev'][:80])
