import sys;sys.path.insert(0,'triage_work')
from common import *
import collections
from fractions import Fraction
df=collections.defaultdict(set)
SETSZ=collections.defaultdict(set)
for l in open('m2_sets.jsonl'):
    d=json.loads(l)
    SETSZ[(d['record'],d['function'])].update(d['values'])
    if INV[d['record']]['side']=='corpus':
        for v in d['values']: df[v].add(INV[d['record']]['repo'].split(' (')[0])
df={k:len(v) for k,v in df.items()}
import math
NREPO=len({d['repo'].split(' (')[0] for d in INV.values() if d['side']=='corpus'})
idf=lambda v: math.log2(NREPO/max(df.get(v,1),1))
def frac(v):
    try: return Fraction(v)
    except Exception: return None
def trivial(v):
    f=frac(v)
    if f is None: return True
    if f.denominator==1 and abs(f.numerator)<=16: return True
    if f.denominator==1 and f.numerator>0 and (f.numerator&(f.numerator-1))==0: return True
    if f in (Fraction(1,2),Fraction(1,4),Fraction(3,4),Fraction(1,10)): return True
    return False
def nonround_decimal(v):
    f=frac(v)
    if f is None or f.denominator==1: return False
    return (f*10).denominator!=1 and not trivial(v)
def nonround_int(v):
    f=frac(v)
    return f is not None and f.denominator==1 and abs(f.numerator)>=100 and f.numerator%10!=0
cnt=collections.Counter(); best={}
with open('m2_hits.tsv') as f:
    r=csv.reader(f,delimiter='\t');h=next(r)
    for row in r:
        cnt['all']+=1
        mp=parse(row[1]);cp=parse(row[2])
        if not mp or not cp: cnt['unparsed']+=1;continue
        if VENDOR.search(cp['path']): cnt['vendor']+=1;continue
        mrec=int(row[9]); fl=row[4].rsplit('@',1)
        fline=int(fl[1].split(':')[0]) if len(fl)==2 and fl[1].split(':')[0].isdigit() else mp['line']
        mt=mtest(mrec,mp['path'],fline) or mtest(mrec,mp['path'],mp['line'])
        ct=bool(CTEST.search(cp['path'])) or 'test' in row[5].lower()
        if mt and not ct: cnt['minase_test']+=1;continue
        vals=row[7].split(',')
        rare=[v for v in vals if not trivial(v) and df.get(v,0)<=3]
        nrd=[v for v in vals if nonround_decimal(v)]
        idfsum=sum(idf(v) for v in vals if not trivial(v))
        csz=len(SETSZ.get((int(row[10]),row[5]),()))
        if csz>300 and len(nrd)<2: cnt['big_corpus_table']+=1;continue
        if not (len(rare)>=3 or len(nrd)>=2 or idfsum>=15): cnt['weak']+=1;continue
        score=round(idfsum,2)
        cnt['kept']+=1
        key=(mp['path'],cp['repo'],cp['path'])
        rec=dict(score=round(score,3),n_rare=len(rare),n_nonround=len(nrd),rare=','.join(rare)[:200],mloc=row[1],cloc=row[2],license=row[3],mfun=row[4],cfun=row[5][:80],common=row[6],base='a05478a' in mp['refs'],mtest=mt,ctest=ct,module=row[4]=='<module>' or row[5]=='<module>')
        if key not in best or (rec['score'],rec['base'])>(best[key]['score'],best[key]['base']): 
            n=best.get(key,{}).get('dups',0); best[key]=rec; rec['dups']=n+1
        else: best[key]['dups']+=1
print(cnt,len(best))
import re as _re
for k,x in best.items():
    for kk,pm,pc in [('K1','time_management','Stockfish'),('K2','stats.rs','fishtest'),('K3','magic-bitboard','Stockfish'),('K4','train_pst','HaChu'),('K4b','handcrafted','HaChu'),('K5','params.rs','hobbes'),('K5b','params.rs','akimbo')]:
        if pm in k[0] and pc in k[1]: print('CALIB',kk,x['score'],x['n_rare'],x['mloc'][:80],x['cloc'][:90],x['rare'][:100])
out=sorted(best.values(),key=lambda x:-x['score'])
json.dump(out,open('triage_work/m2_pairs.json','w'),ensure_ascii=False)
for x in out[:70]:
    print(x['score'],x['n_rare'],x['n_nonround'],x['base'],x['mloc'].split(' [')[0],x['mfun'][:40],'|',x['cloc'].split(' [')[0][:110],x['cfun'][:40],x['license'],'|',x['rare'][:120])
