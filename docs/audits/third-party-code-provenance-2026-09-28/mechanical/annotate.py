"""Apply documented exclusions and provenance metadata; never adjudicate copying."""
import argparse,collections,csv,json,pathlib,re,sys
from fractions import Fraction
from checks import exclusion,norm,token_values,write_tsv
p=argparse.ArgumentParser();p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--method',choices=['all','M1','M2','M3','M4'],default='all');a=p.parse_args();o=a.out.resolve();R=[json.loads(l) for l in (o/'inventory.jsonl').open()];S=json.loads((o/'summary.json').read_text());changes=list(csv.reader((o/'file-license-overrides.tsv').open(),delimiter='\t'))[1:] if (o/'file-license-overrides.tsv').exists() else []
for r in R:
 if r['side']!='corpus':continue
 text=pathlib.Path(r['source']).read_text(errors='replace')[:12000];m=re.search(r'SPDX-License-Identifier:\s*([^\r\n*]+)',text)
 if m:
  lic=m.group(1).strip().rstrip('"');old=r['license']
  if lic!=old:
   changes.append([r['id'],r['repo'],r['locations'][0]['path'],old,lic,'file SPDX header']);r['license']=lic;r['license_evidence']='SPDX-License-Identifier in extracted source'
   if lic in ['MIT','Apache-2.0','BSD-2-Clause','BSD-3-Clause','ISC']:r['category']='permissive'
   elif 'GPL' in lic:r['category']='copyleft'
with (o/'inventory.jsonl').open('w') as f:
 for r in R:f.write(json.dumps(r,ensure_ascii=False)+'\n')
write_tsv(o/'file-license-overrides.tsv',['record','repo','path','manifest_license','file_license','evidence'],changes)
rules=norm((o/'RULES.md').read_text()).replace(' ','');stats={};arrays={x['table_id']:x for x in map(json.loads,(o/'m3_arrays.jsonl').open())}
def line_number(loc):return int(loc.split(' [blob=')[0].rsplit(':',1)[1])
def dense(values):
 try:vs={Fraction(x) for x in values}
 except (ValueError,ZeroDivisionError):return False
 return len(vs)>=8 and all(v.denominator==1 and 0<=v<=256 for v in vs) and len(vs)/(max(vs)-min(vs)+1)>=.5
array_dense={key:dense(x['values']) for key,x in arrays.items()} if a.method in ['all','M3'] else {}
protocol_origins={(x['record'],x['line']) for x in map(json.loads,(o/'m1_texts.jsonl').open()) if x['normalized'].startswith('setoption ')} if a.method in ['all','M1'] else set()
# Keep only the token facts needed to mark repetitive clone candidates.
T={}
for line in ((o/'features.jsonl').open() if a.method in ['all','M4'] else []):
 f=json.loads(line);r=R[f['id']]
 if r['side']=='minase' and r['language'] in ['rust','python']:T[r['id']]=[t[0] for t in f['tokens']]
for method in ['M1','M2','M3','M4']:
 if a.method not in ['all',method]:continue
 path=o/(method.lower()+'_hits.tsv');tmp=path.with_suffix('.revised.tsv');kept=0;ex=collections.Counter();flags=collections.Counter()
 with path.open() as f,tmp.open('w') as g:
  reader=csv.DictReader(f,delimiter='\t');w=csv.DictWriter(g,fieldnames=reader.fieldnames,delimiter='\t');w.writeheader()
  for row in reader:
   ci=int(row['corpus_record']) if method!='M3' else arrays[int(row['corpus_table'])]['record'];row['license']=R[ci]['license']
   if method=='M1':
    mi=int(row['minase_record']);ml=line_number(row['minase_location']);cl=line_number(row['corpus_location']);key=(row['match_kind'],row['matched_text']);reason=row['exclusion_reason']
    if not reason and ((mi,ml) in protocol_origins or (ci,cl) in protocol_origins):reason='protocol_specification'
    row['exclusion_reason']=reason
    if reason:ex[reason]+=1
    else:kept+=1
    if row['matched_text'] in ['must contain only ascii characters','length mismatch expected expected got','black wins white wins draws']:row['false_positive_note']='一般的な入力検査文または集計項目だけの一致'
   elif method=='M2':
    kept+=1
    if dense(row['common_values'].split(',')):row['false_positive_note']='小さな連番または添字集合の一致。機能固有の係数一致とは区別する'
   elif method=='M3':
    kept+=1
    if array_dense[int(row['minase_table'])] or array_dense[int(row['corpus_table'])]:row['false_positive_note']='少なくとも片側が小さな連番の表。倍率付き部分一致を含むため注意'
   else:
    kept+=1;i=int(row['minase_token_start']);n=int(row['length_tokens']);ts=T[int(row['minase_record'])][i:i+n]
    if ts.count('=>')>=4 and ts.count('S')>=4 and len(set(ts))<=15:row['false_positive_note']='列挙値から文字列への反復対応。識別子と文字列内容を消去したための構造一致'
    elif ts.count(':')>=4 and ts.count('pub')>=4 and '=' not in ts:row['false_positive_note']='同じ型の構造体フィールド宣言の反復。フィールド名の消去による一致'
    elif ts.count('+=')>=4:row['false_positive_note']='フィールドごとの加算代入の反復。フィールド名の消去による一致'
   if row['false_positive_note']:flags[row['false_positive_note']]+=1
   w.writerow(row)
 tmp.replace(path);S[method]['candidates']=kept;stats[method]={'candidates':kept,'exclusions':dict(ex),'flags':dict(flags)}
 if method=='M1':S[method]['excluded']={k:ex[k] for k in ['license_boilerplate','todo_boilerplate','protocol_specification','RULES_quote']}
 print(method,kept,dict(ex),flush=True)
S['annotation']={'file_license_overrides':len(changes),'methods':{**S.get('annotation',{}).get('methods',{}),**stats}};(o/'summary.json').write_text(json.dumps(S,ensure_ascii=False,indent=2)+'\n');(o/'annotation-summary.json').write_text(json.dumps(S['annotation']['methods'],ensure_ascii=False,indent=2)+'\n')
trivial={x['value'] for x in csv.DictReader((o/'m2_trivial_values.tsv').open(),delimiter='\t')};excluded=collections.Counter()
for x in csv.DictReader((o/'m2_literals.tsv').open(),delimiter='\t'):
 if x['value'] in trivial:excluded[R[int(x['record'])]['side']]+=1
write_tsv(o/'exclusion-counts.tsv',['method','rule','side','count'],[['M1',k,'matched_pairs',v] for k,v in S['M1']['excluded'].items()]+[['M2','trivial_literal_occurrence',side,count] for side,count in excluded.items()]+[['M3','candidate_exclusion','all',0],['M4','candidate_exclusion','all',0]])
