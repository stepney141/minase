import sys;sys.path.insert(0,'triage_work')
from common import *
import collections
groups=collections.defaultdict(lambda:{'windows':set(),'rows':[]})
# corpus_occurrences counts distinct blobs; count repository families instead (Stockfish tags etc. = 1)
FAM=collections.defaultdict(set)
with open('m1_hits.tsv') as f:
    r=csv.reader(f,delimiter='\t');next(r)
    for row in r:
        if row[3] in('en5','ja12','expression'):
            FAM[row[4]].add(row[1].split('@',1)[0].split(' (')[0])
with open('m1_hits.tsv') as f:
    r=csv.reader(f,delimiter='\t');h=next(r)
    for row in r:
        if row[8] or row[11] or row[3] not in('en5','ja12','expression') or len(FAM[row[4]])>2: continue
        cp=parse(row[1])
        if VENDOR.search(cp['path']): continue
        mp=parse(row[0])
        key=(int(row[9]),mp['line'],int(row[10]),cp['line'])
        g=groups[key]; g['windows'].add(row[4]); g['rows'].append(row)
need=collections.defaultdict(set)
for (mr,ml,cr,cl) in groups: need[mr].add(ml); need[cr].add(cl)
texts={}
with open('m1_texts.jsonl') as f:
    for l in f:
        d=json.loads(l)
        if d['record'] in need and d['line'] in need[d['record']]:
            texts.setdefault((d['record'],d['line']),[]).append(d)
def lcs_tokens(a,b):
    best=0;bi=0
    prev=[0]*(len(b)+1)
    for i in range(1,len(a)+1):
        cur=[0]*(len(b)+1)
        for j in range(1,len(b)+1):
            if a[i-1]==b[j-1]:
                cur[j]=prev[j-1]+1
                if cur[j]>best: best=cur[j];bi=i
        prev=cur
    return best,a[bi-best:bi]
def toks(s):
    return re.findall(r'[\u3040-\u30ff\u4e00-\u9fff]|[^\s\u3040-\u30ff\u4e00-\u9fff]+',s)
out=[]
for key,g in groups.items():
    mr,ml,cr,cl=key
    best=(0,[]); 
    for a in texts.get((mr,ml),[]):
        for b in texts.get((cr,cl),[]):
            x=lcs_tokens(toks(a['normalized']),toks(b['normalized']))
            if x[0]>best[0]: best=x
    run=best[1]; ja=sum(1 for t in run if re.search(r'[぀-ヿ一-鿿]',t)); en=len(run)-ja
    row=g['rows'][0]; mp=parse(row[0]); cp=parse(row[1])
    out.append(dict(mrec=mr,mpath=mp['path'],mline=ml,base='a05478a' in mp['refs'],crec=cr,repo=cp['repo'],cpath=cp['path'],cline=cl,license=row[2],
        windows=len(g['windows']),en=en,ja=ja,run=' '.join(run)[:300],minocc=min(int(x[6]) for x in g['rows']),
        mtext=(texts.get((mr,ml),[{}])[0].get('decoded','') or '')[:200].replace('\n',' ').replace('\t',' '),
        ctext=(texts.get((cr,cl),[{}])[0].get('decoded','') or '')[:200].replace('\n',' ').replace('\t',' ')))
json.dump(out,open('triage_work/m1_groups.json','w'),ensure_ascii=False)
print(len(groups),len(out))
