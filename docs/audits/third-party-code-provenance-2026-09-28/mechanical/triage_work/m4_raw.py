import sys;sys.path.insert(0,'triage_work')
from common import *
import collections
L=json.load(open('triage_work/m4_pairs.json'))
KW=set('fn let mut if else match for in while loop return pub use mod impl struct enum self Self as ref true false const static where type crate super trait dyn move break continue def class import from and or not None True False pass lambda elif try except with yield is return usize u8 u16 u32 u64 i8 i16 i32 i64 f32 f64 bool str String Vec Option Some Ok Err Result'.split())
TOK=re.compile(r'[A-Za-z_]\w*|\d[\w.]*|==|!=|<=|>=|=>|->|::|&&|\|\||[^\s\w]')
def snippet(rid,a,b):
    Ls=src_lines(rid); return '\n'.join(Ls[a-1:b])
def lcs(a,b):
    best=0;prev=[0]*(len(b)+1)
    for i in range(1,len(a)+1):
        cur=[0]*(len(b)+1)
        ai=a[i-1]
        for j in range(1,len(b)+1):
            if ai==b[j-1]:
                v=prev[j-1]+1; cur[j]=v
                if v>best: best=v
        prev=cur
    return best
for x in L:
    ms=snippet(x['mrec'],x['mline'],x['mend']); cs=snippet(x['crec'],x['cline'],x['cend'])
    mt=TOK.findall(ms); ct=TOK.findall(cs)
    mid=[t for t in mt if re.match(r'[A-Za-z_]',t) and t not in KW]; cid=set(t for t in ct if re.match(r'[A-Za-z_]',t) and t not in KW)
    x['id_overlap']=round(sum(1 for t in mid if t in cid)/max(len(mid),1),2)
    x['shared_ids']=' '.join(sorted(set(mid)&cid))[:150]
    x['raw_lcs']=lcs(mt,ct)
    x['msnip']=ms[:600]; x['csnip']=cs[:600]
json.dump(L,open('triage_work/m4_pairs.json','w'),ensure_ascii=False)
c=collections.Counter(min(x['raw_lcs']//10*10,60) for x in L); print(sorted(c.items()))
