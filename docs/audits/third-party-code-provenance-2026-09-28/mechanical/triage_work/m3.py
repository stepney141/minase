import sys;sys.path.insert(0,'triage_work')
from common import *
import collections
from fractions import Fraction
PLAUS={Fraction(x) for x in ['1','2','1/2','4','1/4','5/2','2/5','5','1/5','10','1/10','100','1/100','8','1/8','16','1/16','3/2','2/3','1000','1/1000','3','1/3']}
cnt=collections.Counter(); rows=[]
with open('m3_hits.tsv') as f:
    r=csv.reader(f,delimiter='\t');h=next(r)
    for row in r:
        if row[12]: cnt['noted']+=1;continue
        mp=parse(row[0]);cp=parse(row[1])
        if VENDOR.search(cp['path']): cnt['vendor']+=1;continue
        rows.append(row)
tables=set()
for row in rows: tables.add(int(row[10])); tables.add(int(row[11]))
TAB={}
for l in open('m3_arrays.jsonl'):
    d=json.loads(l)
    if d['table_id'] in tables: TAB[d['table_id']]=d
def num(v):
    try: return Fraction(v)
    except Exception: return None
out={}
for row in rows:
    mp=parse(row[0]);cp=parse(row[1]); L=int(row[4])
    mt=TAB[int(row[10])]; ct=TAB[int(row[11])]
    mrec=mt['record']
    if mtest(mrec,mp['path'],mp['line']) and not CTEST.search(cp['path']): cnt['minase_test']+=1;continue
    try: sc=Fraction(row[5])
    except Exception: sc=None
    if sc not in PLAUS and L<16: cnt['implausible_scale']+=1;continue
    off=int(row[6]); mv=mt['values'][off:off+L] if row[3] in ('subsequence','exact','scaled') else mt['values']
    nv=[num(v) for v in mv]
    if all(x is not None for x in nv):
        if len(set(nv))<4: cnt['few_distinct']+=1;continue
        if all(abs(x)<=8 for x in nv): cnt['small_int']+=1;continue
        if all(a<b for a,b in zip(nv,nv[1:])) or all(a>b for a,b in zip(nv,nv[1:])): cnt['monotone']+=1;continue
        # arithmetic progression-ish (constant diff) -> monotone already; step pattern
    else:
        if len(set(mv))<4: cnt['few_distinct']+=1;continue
    ndist=len(set(mv))
    cnt['kept']+=1
    key=(mp['path'],cp['repo'].split(' (')[0],cp['path'])
    rec=dict(len=L,ndist=ndist,type=row[3],scale=row[5],mloc=row[0],cloc=row[1],license=row[2],mprev=row[8][:160],cprev=row[9][:160],base='a05478a' in mp['refs'],mpath=mp['path'],crepo=cp['repo'],cpath=cp['path'],mtable=row[10],ctable=row[11])
    if key not in out or (L,ndist)>(out[key]['len'],out[key]['ndist']): out[key]=rec
print(cnt,len(out))
L=sorted(out.values(),key=lambda x:(-min(x['ndist'],x['len']),-x['len']))
json.dump(L,open('triage_work/m3_pairs.json','w'),ensure_ascii=False)
for i,x in enumerate(L[:int(sys.argv[1]) if len(sys.argv)>1 else 60]):
    print(i+1,x['len'],x['ndist'],x['type'],x['scale'],x['base'],x['mpath'],x['mloc'].split(' [')[0].rsplit(':',1)[1],'|',x['crepo'][:30],x['cpath'][-50:],x['cloc'].split(' [')[0].rsplit(':',1)[1],x['license'][:10],'|',x['mprev'][:70],'||',x['cprev'][:70])
