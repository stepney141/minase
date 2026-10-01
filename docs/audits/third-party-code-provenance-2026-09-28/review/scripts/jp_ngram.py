import os,re,sys,subprocess
S=sys.argv[1]  # directory that holds r5src/ (see training-and-evaluation.md)
src_dirs=[S+'/r5src', S+'/r5src/live']
jp=re.compile(r'[぀-ヿ一-鿿々ー、。「」・（）0-9A-Za-z]{14,}')
runs=set()
for d in src_dirs:
    for fn in os.listdir(d):
        p=os.path.join(d,fn)
        if not os.path.isfile(p) or fn=='yo_learner.cpp': continue
        t=open(p,encoding='utf-8',errors='ignore').read()
        for m in jp.findall(t):
            if re.search(r'[぀-ヿ一-鿿]',m): runs.add(m)
print('minase JP runs',len(runs),file=sys.stderr)
# corpus files
C=sys.argv[2]  # <corpus-dir>
roots=[C+'/yaneurao_YaneuraOu',C+'/TadaoYamaoka_DeepLearningShogi',C+'/SH11235_tatara',C+'/mk-takizawa_elmo_for_learn',C+'/HiraokaTakuya_apery',C+'/HiraokaTakuya_apery_rust',C+'/SH11235_rshogi',C+'/gikou-official_Gikou',C+'/yaneurao_YaneuraOu-ScriptCollection',C+'/tugajin_cpp_animal_shogi']
texts=[]
for r in roots:
    for dp,dn,fns in os.walk(r):
        if '/.git' in dp: continue
        for fn in fns:
            if fn.endswith(('.cpp','.h','.hpp','.py','.rs','.md','.txt','.c','.toml')):
                try: texts.append((os.path.join(dp,fn),open(os.path.join(dp,fn),encoding='utf-8',errors='ignore').read()))
                except: pass
texts.append(('yo_learner.cpp@0a6dd2cb^',open(S+'/r5src/yo_learner.cpp',encoding='utf-8',errors='ignore').read()))
print('corpus files',len(texts),file=sys.stderr)
big='\n'.join(t for _,t in texts)
# check substrings of length 14 sliding windows of each run
hits={}
for r in runs:
    for i in range(0,len(r)-13):
        w=r[i:i+14]
        if not re.search(r'[぀-ヿ一-鿿]{6}',w): continue
        if w in big:
            hits.setdefault(r,set()).add(w)
for r,ws in sorted(hits.items(),key=lambda x:-len(x[1])):
    w=sorted(ws)[0]
    where=[p for p,t in texts if w in t][:3]
    print(len(ws),'|',r,'|',w,'|',where)
