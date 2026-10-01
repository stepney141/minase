import json,collections,sys
P=json.load(open('triage_work/m2_pairs.json'))
# collapse historical duplicates: key (minase basename-ish path, corpus repo root, corpus path)
agg={}
for x in P:
    mp=x['mloc'].split(' [')[0].split(':',1)[1].rsplit(':',1)[0]
    cp=x['cloc'].split(' [')[0]; crepo=cp.split('@')[0].split(' (')[0]; cpath=cp.split(':',1)[1].rsplit(':',1)[0]
    k=(mp,crepo,cpath)
    if k not in agg or x['score']>agg[k]['score']: agg[k]=dict(x,mp=mp,crepo=crepo,cpath=cpath)
L=sorted(agg.values(),key=lambda x:-x['score'])
print(len(L))
json.dump(L,open('triage_work/m2_pairs_dedup.json','w'),ensure_ascii=False)
for i,x in enumerate(L[:int(sys.argv[1])]):
    print(i+1,x['score'],x['n_rare'],x['n_nonround'],x['base'],x['mp'],x['mfun'][:35],'|',x['crepo'][:28],x['cpath'][-55:],x['cfun'][:30],x['license'][:12],'|',x['common'],x['rare'][:70])
