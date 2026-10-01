import csv,sys,collections
sys.path.insert(0,'triage_work'); from loc import *
csv.field_size_limit(10**9)
meth=sys.argv[1]; fn=f'{meth.lower()}_hits.tsv'
res=collections.defaultdict(list)
with open(fn) as f:
    r=csv.reader(f,delimiter='\t'); h=next(r)
    im=h.index('minase_location'); ic=h.index('corpus_location')
    for row in r:
        m=parse(row[im]); c=parse(row[ic])
        if not m or not c: continue
        for k,fm,fc in K:
            if fm(m['path']) and fc(c['repo'],c['path']):
                res[k].append(row)
for k,_,_ in K:
    rows=res.get(k,[])
    print(k,len(rows))
    if meth=='M1': rows.sort(key=lambda x:-int(x[5]))
    elif meth=='M2': rows.sort(key=lambda x:-int(x[6]))
    elif meth=='M3': rows.sort(key=lambda x:-int(x[4]))
    elif meth=='M4': rows.sort(key=lambda x:-int(x[6]))
    for row in rows[:6]: print('   ','\t'.join(x[:200] for x in row))
