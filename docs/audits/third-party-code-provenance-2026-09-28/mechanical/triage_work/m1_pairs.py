import json,collections
g=json.load(open('triage_work/m1_groups.json'))
for x in g: x['score']=x['en']+x['ja']/2
sel=[x for x in g if x['en']>=8 or x['ja']>=16 or x['score']>=8]
print('groups',len(g),'sel',len(sel))
d={}
for x in sorted(sel,key=lambda x:(-x['score'],not x['base'])):
    k=(x['mpath'],x['repo'],x['cpath'])
    if k not in d: d[k]=dict(x,nruns=0)
    d[k]['nruns']+=1
print('pairs',len(d))
json.dump(list(d.values()),open('triage_work/m1_pairs.json','w'),ensure_ascii=False)
for x in d.values():
    print(x['en'],x['ja'],x['nruns'],x['base'],x['mpath'],x['mline'],'|',x['repo'],x['cpath'],x['cline'],x['license'],'|',x['run'][:140])
