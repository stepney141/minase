import sys;sys.path.insert(0,'triage_work')
from common import *
import collections
cnt=collections.Counter(); notes=collections.Counter(); out={}
with open('m4_hits.tsv') as f:
    r=csv.reader(f,delimiter='\t');h=next(r)
    for row in r:
        L=int(row[6])
        if L<40: cnt['short']+=1;continue
        notes[row[13][:20]]+=1
        if row[13]: cnt['noted']+=1;continue
        mp=parse(row[0]);cp=parse(row[2])
        if VENDOR.search(cp['path']): cnt['vendor']+=1;continue
        mrec=int(row[11])
        if mtest(mrec,mp['path'],mp['line']) and not CTEST.search(cp['path']): cnt['minase_test']+=1;continue
        cnt['kept']+=1
        key=(mp['path'],cp['repo'].split(' (')[0],cp['path'],mp['line'] if 'a05478a' in mp['refs'] else 0)
        rec=dict(len=L,mpath=mp['path'],mline=mp['line'],mend=int(row[1]),crepo=cp['repo'],cpath=cp['path'],cline=cp['line'],cend=int(row[3]),lic=row[4],lang=row[5],mcov=row[7],base='a05478a' in mp['refs'],mrec=mrec,crec=int(row[12]),mloc=row[0],cloc=row[2])
        if key not in out or L>out[key]['len']: out[key]=rec
print(cnt,notes.most_common(5))
# dedup across history: key (mpath, crepo, cpath, snippet cline)
agg={}
for x in out.values():
    k=(x['mpath'],x['crepo'].split(' (')[0],x['cpath'],x['cline'])
    if k not in agg or (x['base'],x['len'])>(agg[k]['base'],agg[k]['len']): agg[k]=x
L=sorted(agg.values(),key=lambda x:-x['len'])
print(len(out),len(L))
json.dump(L,open('triage_work/m4_pairs.json','w'),ensure_ascii=False)
for i,x in enumerate(L):
    print(i+1,x['len'],x['base'],x['mpath'],f"{x['mline']}-{x['mend']}",'|',x['crepo'][:30],x['cpath'][-55:],f"{x['cline']}-{x['cend']}",x['lic'][:12])
