import json,re,csv
W=lambda fn,hdr,rows: (lambda f:(f.write('\t'.join(hdr)+'\n'),[f.write('\t'.join(str(c).replace('\t',' ').replace('\n',' ') for c in r)+'\n') for r in rows]))(open(fn,'w'))
HDR=['rank','minase_location','corpus_location','license','metric','class','reason_or_verdict']
# ---- M1
P=json.load(open('triage_work/m1_pairs.json'))
G=json.load(open('triage_work/m1_groups.json'))
for x in G: x['score']=x['en']+x['ja']/2
tier2=[];seen={(x['run'],x['repo']) for x in P}
for x in sorted(G,key=lambda x:(-x['score'],not x['base'])):
    if x['score']>=8 or x['score']<6: continue
    if re.search(r'\b(12|11|k11|n6|o6|h6|d6|n7|lfcsgkegscfl|mvrhdnqdhrvm|pppppppppppp|toxt1)\b',x['run']): continue
    k=(x['run'],x['repo'])
    if k in seen: continue
    seen.add(k); tier2.append(x)
def c1(x):
    r=x['run']
    if 'working tree is dirty' in r: return ('NEW','判定2: cargo の --allow-dirty 慣行に沿う1文の定型エラー文。tatara(MIT, 2026-07-19)が先行、他のコード一致なし')
    if re.search(r'\b(12|11|k11|k b|n6|o6|h6|d6|n7|lfcsgkegscfl|mvrhdnqdhrvm|pppppppppppp|toxt1)\b',r): return ('FP','test data / rules vocabulary: 中将棋SFEN局面文字列')
    if 'lishogi org' in r: return ('FP','protocol vocabulary: lishogi API URL')
    if 'sys devices system cpu' in r: return ('FP','protocol vocabulary: Linux sysfs パス')
    if re.fullmatch(r'[\d ]+',r): return ('FP','common numeric sequence')
    return ('FP','generic phrase')
rows=[]
for i,x in enumerate(P+tier2[:31]):
    cl,rs=c1(x)
    rows.append([i+1,f"{x['mpath']}:{x['mline']}{' (a05478a)' if x['base'] else ' (history)'}",f"{x['repo']}:{x['cpath']}:{x['cline']}",x['license'],f"en={x['en']} ja={x['ja']} tier={'1' if x['score']>=8 else '2'} run='{x['run'][:90]}'",cl,rs])
W('triage_m1.tsv',HDR,rows); print('m1',len(rows))
# ---- M2
L=json.load(open('triage_work/m2_pairs_dedup.json'))
def c2(x):
    cm=x['common']
    if 'time_management' in x['mp'] and 'Stockfish' in x['crepo']: return ('KNOWN','K1')
    if x['mp'].endswith('params.rs') and 'hobbes' in x['crepo'] and 'parameters.rs' in x['cpath']: return ('KNOWN','K5 (ただし一致は既定値の偶然の重なりでマクロ構造ではない)')
    if 'features.py' in x['mp']: return ('FP','common numeric sequence: 12x12盤の升番号(11..85,144)')
    if '301/500' in x['rare'] or x['mp'].startswith('src/bin/spsa_runner'): return ('FP','standard constants: SPSA の Spall 推奨値 α=0.602, γ=0.101 と fishtest 既定')
    if '29444389791664403' in x['rare']: return ('FP','standard constants: GSPRT 境界 log(19)=2.944')
    if 'ctgbook' in x['cpath']: return ('FP','common numeric sequence: 探索既定値と表データの偶然の重なり')
    return ('FP','common numeric sequence: 丸め・分位点・ε・偶然の小数一致')
rows=[]
for i,x in enumerate(L[:80]):
    cl,rs=c2(x)
    rows.append([i+1,x['mloc'],x['cloc'],x['license'],f"idf_sum={x['score']} rare={x['n_rare']} nonround={x['n_nonround']} common={x['common']} {x['mfun'][:30]}↔{x['cfun'][:30]}",cl,rs])
W('triage_m2.tsv',HDR,rows); print('m2',len(rows))
# ---- M3
S=json.load(open('triage_work/m3_pairs.json')); LO=json.load(open('triage_work/m3_loose.json'))
extra=[];seen=set((x['mprev'][:40],x['cprev'][:40]) for x in S)
for x in LO:
    if x['ap'] or x['small'] or x['mono']: continue
    k=(x['mprev'][:40],x['cprev'][:40])
    if k in seen: continue
    seen.add(k); extra.append(x)
def c3(x):
    if 'HaChu' in x['crepo']: return ('KNOWN','K4')
    if '@expr:(0, 1)' in x['mprev']: return ('FP','rules vocabulary: 8近傍方向ベクトル')
    return ('FP','common numeric sequence / 部分集合の偶然一致(倍率付き並べ替え)')
rows=[]
for i,x in enumerate(S+extra[:30]):
    cl,rs=c3(x)
    rows.append([i+1,x['mloc'],x['cloc'],x.get('license',x.get('lic')),f"{x['type']} len={x['len']} ndist={x.get('ndist',x.get('nd'))} scale={x['scale']} tier={'strict' if x in S else 'loose'}",cl,rs])
W('triage_m3.tsv',HDR,rows); print('m3',len(rows))
# ---- M4
M=json.load(open('triage_work/m4_pairs.json'))
H=sorted([x for x in M if x['raw_lcs']>=30],key=lambda x:-x['raw_lcs'])
top_len=[x for x in sorted(M,key=lambda x:-x['len']) if x['raw_lcs']<30][:30]
def c4(x):
    s=x['shared_ids']
    if 'wrapping_mul' in s: return ('FP','standard algorithm: SplitMix64 の公開定数')
    if 'split_whitespace' in s: return ('FP','protocol vocabulary: USI info 行の分解(4行の慣用句)')
    if x['raw_lcs']<30: return ('FP','Rust idiom: 識別子を正規化した構造一致のみ(実識別子は異なる)')
    return ('FP','Rust idiom: Display/Error実装、io::Error変換、OpenOptions、Command出力検査など')
rows=[]
for i,x in enumerate(H+top_len):
    cl,rs=c4(x)
    rows.append([i+1,f"{x['mpath']}:{x['mline']}-{x['mend']}{' (a05478a)' if x['base'] else ' (history)'}",f"{x['crepo']}:{x['cpath']}:{x['cline']}-{x['cend']}",x['lic'],f"winnow_len={x['len']} raw_lcs={x['raw_lcs']} id_overlap={x['id_overlap']}",cl,rs])
W('triage_m4.tsv',HDR,rows); print('m4',len(rows))
