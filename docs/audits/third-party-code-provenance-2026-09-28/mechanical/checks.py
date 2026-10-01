"""Mechanical comparisons. Inputs are immutable Git objects extracted by inventory.py."""
import argparse, ast, collections, csv, difflib, hashlib, itertools, json, math, pathlib, re, sys, unicodedata
from fractions import Fraction
from text_normalization import decode_strings
from functools import lru_cache

def write_tsv(path,header,rows):
 with path.open('w') as f:
  w=csv.writer(f,delimiter='\t');w.writerow(header);w.writerows(rows)
def number(raw,language=None):
 s=re.sub(r'\s+','',raw).replace('_','').replace("'",'')
 s=re.sub(r'(?:[iu](?:8|16|32|64|128|size)|f(?:32|64))$','',s,flags=re.I)
 if re.match(r'^[+-]?0[xX]',s):
  s=re.sub(r'[uUlL]+$','',s)
  if 'p' in s.lower():return Fraction(float.fromhex(s))
  return Fraction(int(s,16))
 s=re.sub(r'[uUlLfFdDmM]+$','',s)
 if re.match(r'^[+-]?0[bB]',s):return Fraction(int(s,2))
 if re.match(r'^[+-]?0[oO]',s):return Fraction(int(s,8))
 if language in ['c','cpp','java','c_sharp'] and re.match(r'^[+-]?0[0-7]+$',s):return Fraction(int(s,8))
 return Fraction(s)
@lru_cache(maxsize=100000)
def norm(raw,expand_escapes=True):
 s=unicodedata.normalize('NFKC',raw)
 def esc(m):
  x=m.group(1)
  if x in {'n','r','t','\\','"',"'",'0'}:return {'n':' ','r':' ','t':' ','\\':'\\','"':'"',"'":"'",'0':'\0'}[x]
  if x.startswith('u{'):return chr(int(x[2:-1],16))
  if x[0] in 'uxU':return chr(int(x[1:],16))
  return x
 # Raw literals keep backslashes; ordinary literals/comments expand known escapes.
 if expand_escapes and not re.match(r'^(?:r|br)#+"',s):s=re.sub(r'\\(u\{[0-9a-fA-F]+\}|u[0-9a-fA-F]{4}|U[0-9a-fA-F]{8}|x[0-9a-fA-F]{2}|[nrt\\"\'0])',esc,s)
 s=re.sub(r'([a-z0-9])([A-Z])',r'\1 \2',s);s=re.sub(r'([A-Z])([A-Z][a-z])',r'\1 \2',s)
 s=''.join(' ' if unicodedata.category(c)[0] in 'PZS' else c for c in s)
 return ' '.join(s.lower().split())
def phrases(s,raw):
 words=re.findall(r'[a-z][a-z0-9]*|[0-9]+',s)
 out={('en5',' '.join(words[i:i+5])) for i in range(len(words)-4)}
 for span in re.findall(r'[\u3040-\u30ff\u3400-\u9fffA-Za-z0-9 ]+',s):
  span=span.replace(' ','')
  if re.search(r'[\u3040-\u30ff\u3400-\u9fff]',span):out.update(('ja12',span[i:i+12]) for i in range(len(span)-11))
 if 0<len(words)<5:out.add(('short',' '.join(words)))
 identifiers=re.findall(r'\b(?:[a-z]+[A-Z][a-zA-Z0-9]*|[A-Za-z]+_[A-Za-z0-9_]+)\b',raw)
 for ident in identifiers:
  z=norm(ident)
  if z and len(z.split())<5:out.add(('identifier',z))
 # Short expressions retained separately regardless of rarity.
 for expr in re.findall(r'\b[A-Za-z_][A-Za-z0-9_]*\s*(?:==|!=|<=|>=|[+*/=<>-])\s*[A-Za-z0-9_]+',raw):out.add(('expression',norm(expr)))
 return out

def location(r,line):
 loc=next((x for x in r['locations'] if x['scope']=='baseline'),r['locations'][0])
 refs=','.join(sorted({x['ref'] for x in r['locations']}))
 return f"{r['repo']}@{refs}:{loc['path']}:{line} [blob={r['blob'][:12]}]"
def exclusion(raw,key,rules):
 s=norm(raw);p=key[1]
 if any(t in s for t in ['gnu general public license','gnu affero general','permission is hereby granted','without any warranty','all rights reserved','redistribute it and or modify','copyright c','spdx license identifier']):return 'license_boilerplate'
 if p in {'todo','fixme','xxx','todo fixme','unimplemented','not implemented'}:return 'todo_boilerplate'
 protocol=r'^(?:usi|uci|usiok|uciok|isready|readyok|usinewgame|ucinewgame|bestmove|ponder|ponderhit|sfen|cecp|feature|option name|setoption|info|position|go|btime|wtime|binc|winc|byoyomi|type spin|spin default|hash type spin)(?: |$)'
 if re.match(protocol,p) or re.match(protocol,s):return 'protocol_specification'
 if key[0]=='ja12' and p in rules:return 'RULES_quote'
 return ''

def run_m1(o,R,F,summary):
 rules=norm((o/'RULES.md').read_text()).replace(' ','');mq=collections.defaultdict(list);matches=collections.defaultdict(list);counts=collections.Counter();excluded=collections.Counter();texts_count=collections.Counter()
 with (o/'m1_texts.jsonl').open('w') as out:
  for r in R:
   for t in F[r['id']]['texts']:
    raw=t[3];decoded=decode_strings(raw,r['language']) if t[2]=='string' else raw;normal=norm(decoded,t[2]!='string');obj={'record':r['id'],'line':t[0],'end_line':t[1],'kind':t[2],'raw':raw,'decoded':decoded,'normalized':normal};out.write(json.dumps(obj,ensure_ascii=False)+'\n');texts_count[r['side']]+=1
    if r['side']=='minase':
     for k in phrases(normal,raw):mq[k].append((r['id'],t[0],raw))
  for r in R:
   if r['side']!='corpus':continue
   for t in F[r['id']]['texts']:
    for k in phrases(norm(decode_strings(t[3],r['language']) if t[2]=='string' else t[3],t[2]!='string'),t[3]):
     if k in mq:counts[k]+=1;matches[k].append((r['id'],t[0],t[3]))
 rows=0;kept=0;top=[]
 with (o/'m1_hits.tsv').open('w') as f:
  w=csv.writer(f,delimiter='\t');w.writerow(['minase_location','corpus_location','license','match_kind','matched_text','match_length','corpus_occurrences','rarity','exclusion_reason','minase_record','corpus_record','false_positive_note'])
  for k,cs in matches.items():
   if k[0]=='short' and counts[k]>2:continue
   for mi,ml,mraw in mq[k]:
    for ci,cl,craw in cs:
     reason=exclusion(mraw,k,rules) or exclusion(craw,k,rules);length=len(k[1]) if k[0]=='ja12' else len(k[1].split());note='短い単語または識別子のみ。一般的名称の一致を含む' if k[0] in ('short','identifier') else ''
     row=[location(R[mi],ml),location(R[ci],cl),R[ci]['license'],k[0],k[1],length,counts[k],round(1/counts[k],6),reason,mi,ci,note];w.writerow(row);rows+=1
     if reason:excluded[reason]+=1
     else:
      kept+=1
      if len(top)<1000:top.append(row)
 summary['M1']={'rows':rows,'candidates':kept,'excluded':dict(excluded),'extracted_texts':dict(texts_count),'matched_keys':len(matches),'threshold':'English 5 words; Japanese 12 characters; short whole phrases corpus frequency <=2; identifiers/expressions separate'}
 print('M1',summary['M1'],flush=True)

PARAM_RE=re.compile(r'\b([A-Z][A-Za-z0-9_]*)\s*\(\s*([a-z][a-z0-9_]*)\s*\)\s*:\s*([+-]?[0-9_]+)\s*,\s*([+-]?[0-9_]+)\s*,\s*([+-]?[0-9_]+)\s*;')
def unit_value(value,context):
 c=context.lower()
 denom=re.search(r'([\d,]+)分率',c)
 if denom:return value/Fraction(denom.group(1).replace(',','')),'ratio'
 if any(x in c for x in ['百分率','percent','percentage','パーセント']):return value/100,'ratio'
 if any(x in c for x in ['千分率','permille','per mille']):return value/1000,'ratio'
 if any(x in c for x in ['ミリ秒','milliseconds','millisecond']) or re.search(r'\bms\b',c):return value/1000,'seconds'
 return value,'unspecified'
def param_extract(r):
 if not any(x['path'].endswith('params.rs') for x in r['locations']):return []
 text=pathlib.Path(r['source']).read_text(errors='replace');out=[]
 # Syntax is a macro token-tree declaration; match its literal production, never numeric text in comments.
 for m in PARAM_RE.finditer(text):
  before=text[:m.start()];line=before.count('\n')+1
  if not re.search(r'parameters!\s*\{',before):continue
  context='\n'.join(before.splitlines()[-3:]);name,access,*vals=m.groups();out.append({'name':name,'accessor':access,'values':[str(number(v)) for v in vals],'line':line,'context':context})
 return out

def run_m2(o,R,F,summary):
 sets=[];freq=collections.Counter();bad=[];params=[];first={};occ=list(csv.DictReader((o/'blob-first-occurrence.tsv').open(),delimiter='\t'));rank={(x['blob'],x['path']):int(x['topological_index']) for x in occ}
 with (o/'m2_literals.tsv').open('w') as f:
  w=csv.writer(f,delimiter='\t');w.writerow(['record','line','function','raw','value','unit_normalized','unit'])
  for r in R:
   groups=collections.defaultdict(set);lines={}
   for line,raw,fn,ctx in F[r['id']]['numbers']:
    try:v=number(raw,r['language'])
    except (ValueError,ZeroDivisionError,OverflowError):bad.append([r['id'],line,raw]);continue
    uv,unit=unit_value(v,ctx);w.writerow([r['id'],line,fn,raw,str(v),str(uv),unit]);groups[fn].add(str(v));lines[fn]=min(line,lines.get(fn,line))
    if unit!='unspecified':groups[fn+'#unit-normalized'].add(str(uv));lines[fn+'#unit-normalized']=line
    if r['side']=='corpus':freq[str(v)]+=1
   for p in param_extract(r):
    prefix=re.findall(r'[A-Z]+(?=[A-Z][a-z]|$)|[A-Z]?[a-z]+|\d+',p['name'])[0].lower();fn='params:'+prefix
    groups[fn].update(p['values']);lines[fn]=p['line'];params.append([r['id'],p['line'],p['name'],p['accessor'],*p['values'],prefix])
    for loc in r['locations']:
     key=(loc['path'],p['name']);rr=rank.get((r['blob'],loc['path']),10**9)
     if key not in first or rr<first[key][0]:first[key]=(rr,r,p,loc)
   for fn,vs in groups.items():sets.append({'record':r['id'],'function':fn,'line':lines[fn],'values':vs,'origin':'blob'})
 intro=collections.defaultdict(lambda:{'values':set(),'names':[]});intro_rows=[]
 for (_,name),(rankval,r,p,loc) in first.items():
  prefix=re.findall(r'[A-Z]+(?=[A-Z][a-z]|$)|[A-Z]?[a-z]+|\d+',name)[0].lower();key=(loc['path'],prefix);z=intro[key];z['values'].update(p['values']);z['record']=r['id'];z['line']=p['line'];z['names'].append(name)
  intro_rows.append([loc['path'],name,loc['first_commit'],r['blob'],p['line'],*p['values'],prefix])
 for (path,prefix),z in intro.items():sets.append({'record':z['record'],'function':'introduced:'+path+':'+prefix,'line':z['line'],'values':z['values'],'origin':'coefficient_introduction'})
 trivial={'0','1','2','-1','10','100','1000'}|{str(2**i) for i in range(0,129)}|{v for v,n in freq.most_common(20)}
 write_tsv(o/'m2_trivial_values.tsv',['value','corpus_frequency'],[(v,freq[v]) for v in sorted(trivial,key=Fraction)])
 write_tsv(o/'m2_params.tsv',['record','line','name','accessor','default','min','max','prefix'],params)
 write_tsv(o/'m2_params_introductions.tsv',['path','name','first_commit','blob','line','default','min','max','prefix'],intro_rows)
 write_tsv(o/'m2_numeric_failures.tsv',['record','line','literal'],bad)
 with (o/'m2_sets.jsonl').open('w') as f:
  for s in sets:f.write(json.dumps({**s,'values':sorted(s['values'],key=Fraction)})+'\n')
 index=collections.defaultdict(set)
 for i,s in enumerate(sets):
  s['values']-=trivial
  if R[s['record']]['side']=='corpus':
   for v in s['values']:index[v].add(i)
 single=0;hits=0
 with (o/'m2_hits.tsv').open('w') as f, (o/'m2_common_values.tsv').open('w') as g:
  w=csv.writer(f,delimiter='\t');vout=csv.writer(g,delimiter='\t');w.writerow(['pair_id','minase_location','corpus_location','license','minase_function','corpus_function','common_count','common_values','origin','minase_record','corpus_record','false_positive_note']);vout.writerow(['pair_id','value'])
  for s in sets:
   if R[s['record']]['side']!='minase':continue
   ct=collections.Counter(j for v in s['values'] for j in index[v]);single+=sum(n==1 for n in ct.values())
   for j,n in ct.items():
    if n<2:continue
    c=sets[j];common=sorted(s['values']&c['values'],key=Fraction);hits+=1;note='関数外の定数集合。機能の対応は未確認' if s['function']=='<module>' or c['function']=='<module>' else ''
    w.writerow([hits,location(R[s['record']],s['line']),location(R[c['record']],c['line']),R[c['record']]['license'],s['function'],c['function'],n,','.join(common),s['origin'],s['record'],c['record'],note]);vout.writerows((hits,x) for x in common)
 summary['M2']={'candidates':hits,'single_matches':single,'numeric_failures':len(bad),'trivial_values':len(trivial),'parameter_rows':len(params),'introduced_coefficients':len(intro_rows),'sets':len(sets),'threshold':'at least 2 nontrivial common values; top 20 frequency values and explicit trivial values removed'}
 print('M2',summary['M2'],flush=True)

def scale_interval(a,b):
 low=Fraction(0);high=None
 for x,y in zip(a,b):
  if y==0:
   if abs(x)>1:return None
  else:
   lo,hi=sorted(((x-1)/y,(x+1)/y));low=max(low,lo);high=hi if high is None else min(high,hi)
   if high<low or high<=0:return None
 if high is None:return None
 return (low+high)/2 if low>0 else high/2

def scaled_overlap(a,b,ratio):
 aa=sorted((float(v),i) for i,v in enumerate(a));bb=sorted((float(v*ratio),i) for i,v in enumerate(b));i=j=0;matches=[]
 while i<len(aa) and j<len(bb):
  x,ix=aa[i];y,iy=bb[j]
  if abs(x-y)<=1+1e-9:matches.append((ix,iy));i+=1;j+=1
  elif x<y:i+=1
  else:j+=1
 return matches

def run_m3(o,R,F,summary):
 tables=[];numeric_failed=0;array_limits=[]
 for r in R:
  array_limits.extend([r['id'],*x] for x in F[r['id']].get('arrayLimitations',[]))
  for a in F[r['id']]['arrays']:
   try:vs=tuple(x if x.startswith('@expr:') else number(x,r['language']) for x in a[3])
   except (ValueError,ZeroDivisionError,OverflowError):numeric_failed+=1;continue
   tables.append({'record':r['id'],'line':a[0],'end':a[1],'kind':a[2],'values':vs,'element_lines':a[4]})
 write_tsv(o/'m3_array_limitations.tsv',['record','line','reason','expression'],array_limits)
 with (o/'m3_arrays.jsonl').open('w') as f:
  for i,t in enumerate(tables):f.write(json.dumps({'table_id':i,**t,'values':[str(x) for x in t['values']]})+'\n')
 unique={'minase':collections.defaultdict(list),'corpus':collections.defaultdict(list)}
 for i,t in enumerate(tables):unique[R[t['record']]['side']][t['values']].append(i)
 exact=collections.defaultdict(list);multi=collections.defaultdict(list);length=collections.defaultdict(list);seq=collections.defaultdict(list)
 for b in unique['corpus']:
  exact[b].append(b);multi[tuple(sorted(b,key=str))].append(b);length[len(b)].append(b)
 wanted={a[i:i+8] for a in unique['minase'] for i in range(len(a)-7)}
 cvalues=list(unique['corpus'])
 for ci,b in enumerate(cvalues):
  for k in set(b[j:j+8] for j in range(len(b)-7)):
   if k in wanted:seq[k].append(ci)
 hits=0;types=collections.Counter();cal=[];pair_unique=set()
 with (o/'m3_hits.tsv').open('w') as f:
  w=csv.writer(f,delimiter='\t');w.writerow(['minase_location','corpus_location','license','match_type','length','scale','minase_offset','corpus_offset','minase_values_preview_first16','corpus_values_preview_first16','minase_table','corpus_table','false_positive_note'])
  def emit(a,b,typ,n,ratio=1,ai=0,bi=0):
   nonlocal hits
   key=(a,b,typ,ai,bi)
   if key in pair_unique:return
   pair_unique.add(key)
   for mi in unique['minase'][a]:
    for ci in unique['corpus'][b]:
     mt,ct=tables[mi],tables[ci];note='定数が2種類以下の反復表' if len(set(a))<=2 or len(set(b))<=2 else ''
     w.writerow([location(R[mt['record']],mt['element_lines'][ai]),location(R[ct['record']],ct['element_lines'][bi]),R[ct['record']]['license'],typ,n,str(ratio),ai,bi,','.join(map(str,a[:16])),','.join(map(str,b[:16])),mi,ci,note]);hits+=1;types[typ]+=1
     if 'HaChu' in R[ct['record']]['repo'] and any(x['path']=='src/eval/handcrafted.rs' for x in R[mt['record']]['locations']) and mt['line']<60 and typ.startswith('scaled'):
      cal.append({'minase_table':mi,'corpus_table':ci,'scale':str(ratio),'matched':n,'minase_count':len(a),'corpus_count':len(b),'type':typ})
  for ix,a in enumerate(unique['minase']):
   for b in exact[a]:emit(a,b,'exact',len(a))
   for b in multi[tuple(sorted(a,key=str))]:
    if b!=a:emit(a,b,'multiset',len(a))
   for b in length[len(a)]:
    if b==a or not all(isinstance(x,Fraction) for x in a+b):continue
    k=scale_interval(a,b)
    if k is not None:emit(a,b,'scaled',len(a),k)
   candidate_arrays={ci for key in set(a[i:i+8] for i in range(len(a)-7)) for ci in seq[key]}
   for ci in candidate_arrays:
    b=cvalues[ci]
    if a==b:continue
    if len(set(a))==1 or len(set(b))==1:
     short,long,swapped=(a,b,False) if len(set(a))==1 else (b,a,True);value=short[0];best=(0,0);offset=0
     for val,g in itertools.groupby(long):
      count=sum(1 for _ in g)
      if val==value and count>best[1]:best=(offset,count)
      offset+=count
     n=min(len(short),best[1]);ai,bi=(best[0],0) if swapped else (0,best[0])
    else:
     match=difflib.SequenceMatcher(None,a,b,autojunk=False).find_longest_match();ai,bi,n=match.a,match.b,match.size
    if n>=8:emit(a,b,'subsequence',n,1,ai,bi)
   # Composite calibration extension: reordered, scaled subsets of record columns.
   # At least 8 matches AND 70% of the shorter table; applies to every table, not named HaChu values.
   if 8<=len(a)<=128 and len(set(a))>=8 and all(isinstance(x,Fraction) for x in a):
    for b in unique['corpus']:
     if not (8<=len(b)<=128 and len(set(b))>=8 and all(isinstance(x,Fraction) for x in b)):continue
     ratios=collections.Counter(round(float(x/y),2) for x in set(a) if x>0 for y in set(b) if y>0)
     for k,count in ratios.items():
      if count<4 or k<=0:continue
      ratio=Fraction(str(k));matches=scaled_overlap(a,b,ratio)
      if len(matches)>=max(8,math.ceil(.7*min(len(a),len(b)))):
       emit(a,b,'scaled_reordered_subset',len(matches),ratio);break
   if ix%100==0:print('M3 arrays',ix,'/',len(unique['minase']),flush=True)
 write_tsv(o/'m3_calibration.tsv',['minase_table','corpus_table','scale','matched','minase_count','corpus_count','type'],[[c[k] for k in ['minase_table','corpus_table','scale','matched','minase_count','corpus_count','type']] for c in cal])
 summary['M3']={'candidates':hits,'types':dict(types),'arrays':len(tables),'unique_minase_arrays':len(unique['minase']),'unique_corpus_arrays':len(unique['corpus']),'numeric_failures':numeric_failed,'unresolved_repeat_lengths':len(array_limits),'calibration_pass':any(abs(float(Fraction(c['scale']))-2.5)<.01 and c['matched']>=8 for c in cal),'calibration':cal[:10],'threshold':'8 elements; positive scaling absolute error <=1; composite unordered scaled subset >=8 and >=70% of shorter table'}
 print('M3',summary['M3'],flush=True)

MASK=(1<<64)-1

def fingerprint(tokens,threshold,k=10):
 if len(tokens)<threshold:return []
 vocab=[int.from_bytes(hashlib.blake2b(x.encode(),digest_size=8).digest(),'little') for x in tokens];base=1000003;power=pow(base,k-1,1<<64);h=0;hs=[]
 for v in vocab[:k]:h=(h*base+v)&MASK
 hs.append(h)
 for i in range(k,len(vocab)):h=(((h-vocab[i-k]*power)&MASK)*base+vocab[i])&MASK;hs.append(h)
 w=threshold-k+1;dq=collections.deque();out=[];last=-1
 for i,h in enumerate(hs):
  while dq and dq[0]<=i-w:dq.popleft()
  while dq and hs[dq[-1]]>=h:dq.pop()
  dq.append(i)
  if i>=w-1 and dq[0]!=last:last=dq[0];out.append((hs[last],last))
 return out

def local_matches(a,b,threshold,a_fp=None,b_index=None):
 idx=collections.defaultdict(list) if b_index is None else b_index
 if b_index is None:
  for h,j in fingerprint(b,threshold):idx[h].append(j)
 matches=[];covered=collections.defaultdict(list)
 for h,i in (fingerprint(a,threshold) if a_fp is None else a_fp):
  for j in idx[h]:
   if a[i:i+10]!=b[j:j+10]:continue
   d=i-j
   if any(s<=i<e for s,e in covered[d]):continue
   l=0
   while i-l>0 and j-l>0 and a[i-l-1]==b[j-l-1]:l+=1
   n=10
   while i+n<len(a) and j+n<len(b) and a[i+n]==b[j+n]:n+=1
   x,y,z=i-l,j-l,n+l;covered[d].append((x,x+z))
   if z>=threshold:matches.append((x,y,z))
 return sorted(set(matches))

def token_values(tokens):
 return ['N:'+str(number(t[5],'rust')) if t[0]=='N' else t[0] for t in tokens]

def calibration(o,R,F):
 selectors={
  'see_left':('codedeliveryservice/Reckless','src/board/see.rs','see'),
  'see_right':('cosmobobak/viridithas','src/search.rs','static_exchange_eval'),
  'evaluate_left':('codedeliveryservice/Reckless','src/nnue.rs','evaluate'),
  'evaluate_right':('cosmobobak/viridithas','src/nnue/network.rs','evaluate'),
  'popcount_left':('codedeliveryservice/Reckless','src/types/bitboard.rs','popcount'),
  'popcount_right':('cosmobobak/viridithas','src/chess/squareset.rs','count')}
 selected={}
 for key,(repo,path,name) in selectors.items():
  for r in R:
   if r['repo']!=repo or not any(x['path']==path for x in r['locations']):continue
   for fn in F[r['id']]['functions']:
    if fn[0].split('@')[0]==name:
     ts=[t for t in F[r['id']]['tokens'] if t[3]>=fn[3] and t[4]<=fn[4]];selected[key]=(r,fn,ts)
 if len(selected)!=len(selectors):raise RuntimeError('Missing functional calibration controls')
 source,fn,ts=selected['see_left'];text=pathlib.Path(source['source']).read_text(errors='replace');snippet=text[fn[3]:fn[4]]
 replacements=[(t[3]-fn[3],t[4]-fn[3],'minase_'+t[5]) for t in ts if t[0]=='I'];transformed=snippet
 for start,end,replacement in sorted(replacements,reverse=True):transformed=transformed[:start]+replacement+transformed[end:]
 lines=transformed.splitlines();di=next(i for i,l in enumerate(lines) if 'let minase_diagonal =' in l);oi=next(i for i,l in enumerate(lines) if 'let minase_orthogonal =' in l);lines[di],lines[oi]=lines[oi],lines[di];transformed='\n'.join(lines)
 d=o/'calibration';d.mkdir(exist_ok=True);(d/'original.rs').write_text(snippet+'\n');(d/'renamed_reordered.rs').write_text(transformed+'\n')
 import subprocess
 subprocess.run(['node',str(o/'fixture_tokens.cjs'),str(o)],check=True)
 fixture=json.loads((d/'tokens.json').read_text())
 if fixture['original_errors'] or fixture['renamed_reordered_errors']:raise RuntimeError('Calibration fixture parse failure')
 def canon(ts):return ['N:'+str(number(x[2:],'rust')) if x.startswith('N:') else x for x in ts]
 a=canon(fixture['original']);b=canon(fixture['renamed_reordered']);rows=[]
 for threshold in [20,40,80]:
  clones=local_matches(a,b,threshold);covered=set(i for x,y,n in clones for i in range(x,x+n));rows.append(['renamed_reordered_clone',threshold,int(bool(clones)),len(clones),len(covered)/len(a),location(source,fn[1]),'identifier renaming and independent diagonal/orthogonal declarations reordered'])
  for name in ['see','evaluate','popcount']:
   l=selected[name+'_left'];r=selected[name+'_right'];aa=token_values(l[2]);bb=token_values(r[2]);mm=local_matches(aa,bb,threshold);cov=set(i for x,y,n in mm for i in range(x,x+n));rows.append(['independent:'+name,threshold,int(bool(mm)),len(mm),len(cov)/len(aa),location(l[0],l[1][1]),location(r[0],r[1][1])])
 write_tsv(o/'m4_calibration.tsv',['case','minimum_tokens','detected','local_matches','source_coverage','source','comparison'],rows)
 viable=[t for t in [20,40,80] if any(row[0]=='renamed_reordered_clone' and row[1]==t and row[2] for row in rows)]
 if not viable:raise RuntimeError('Artificial clone was not detected')
 chosen=min(viable,key=lambda t:(sum(row[2] for row in rows if row[0].startswith('independent:') and row[1]==t),t))
 return chosen,rows

def run_m4(o,R,F,summary):
 threshold,cal=calibration(o,R,F);total=0;hist=collections.Counter();comparisons=0
 with (o/'m4_hits.tsv').open('w') as f:
  w=csv.writer(f,delimiter='\t');w.writerow(['minase_location','minase_end_line','corpus_location','corpus_end_line','license','language','length_tokens','minase_file_coverage','corpus_file_coverage','minase_token_start','corpus_token_start','minase_record','corpus_record','false_positive_note'])
  for language in ['rust','python']:
   corpus=[r for r in R if r['side']=='corpus' and r['language']==language];mins=[r for r in R if r['side']=='minase' and r['language']==language];index=collections.defaultdict(set);ct={};positions={}
   for c in corpus:
    ts=token_values(F[c['id']]['tokens']);ct[c['id']]=ts
    positions[c['id']]=collections.defaultdict(list)
    for h,j in fingerprint(ts,threshold):index[h].add(c['id']);positions[c['id']][h].append(j)
   for ri,m in enumerate(mins):
    mt=token_values(F[m['id']]['tokens']);mfp=fingerprint(mt,threshold);candidates=set(c for h,i in mfp for c in index[h])
    for ci in sorted(candidates):
     c=R[ci];cs=ct[ci];comparisons+=1
     for i,j,n in local_matches(mt,cs,threshold,mfp,positions[ci]):
      ml=F[m['id']]['tokens'];cl=F[ci]['tokens'];note='単純な配列、宣言または反復構造の一致' if len(set(mt[i:i+n]))<=8 else ''
      w.writerow([location(m,ml[i][1]),ml[i+n-1][2],location(c,cl[j][1]),cl[j+n-1][2],c['license'],language,n,round(n/len(mt),8),round(n/len(cs),8),i,j,m['id'],ci,note]);total+=1;hist[language]+=1
    if ri%100==0:print('M4',language,ri,'/',len(mins),'hits',total,flush=True)
 summary['M4']={'candidates':total,'languages':dict(hist),'threshold':threshold,'k':10,'window':threshold-9,'candidate_file_pairs':comparisons,'calibration':cal,'tool_status':'JPlag/Dolos unavailable; supplementary tree-sitter winnowing executed; strict specified-tool validation incomplete','cross_language':'not executed: no independently validated translation fixture'}
 print('M4',total,'threshold',threshold,flush=True)

def lexical_toml(r,f):
 from pygments.lexers import get_lexer_by_name
 from pygments.token import Comment, String, Number, Error
 text=pathlib.Path(r['source']).read_text(errors='replace');lexer=get_lexer_by_name('toml');line=1;errs=[]
 for offset,typ,value in lexer.get_tokens_unprocessed(text):
  start=text.count('\n',0,offset)+1
  if typ in Comment or typ in String:f['texts'].append([start,start+value.count('\n'),'comment' if typ in Comment else 'string',value,offset,offset+len(value)])
  if typ in Number:f['numbers'].append([start,value,'<module>',''])
  if typ in Error:errs.append([start,start,'lexical_error'])
 f['errors']=errs;f['status']='lexical_partial' if errs else 'lexical_ok'

def coverage(o,R,F,summary):
 rows=[];stats=collections.Counter();encoding_errors=[]
 for r in R:
  encoding_bad=False
  try:pathlib.Path(r['source']).read_bytes().decode('utf8')
  except UnicodeDecodeError as e:
   encoding_bad=True;encoding_errors.append([r['id'],r['repo'],r['locations'][0]['path'],e.start,'invalid_utf8: original source bytes preserved; extraction used replacement characters'])
  f=F[r['id']]
  for loc in r['locations']:
   for m in ['M1','M2','M3','M4']:
    reason='';status=f['status']
    if m=='M4' and r['language'] not in ['rust','python']:status='not_applicable';reason='same-language Rust/Python only; patch fragments not standalone modules'
    elif m=='M3' and r['language'] not in ['rust','c','cpp','python','patch']:status='not_applicable';reason='specified array languages: Rust/C/C++/Python'
    elif m=='M4':reason='supplementary tree-sitter winnowing; required JPlag/Dolos unavailable'
    elif r['language']=='patch':reason='both old/new hunk fragments; syntax errors retained; hunk line numbers refer to patch file'
    if m=='M3' and f.get('arrayLimitations'):
     reason+='; unresolved repeat-array lengths: '+str(len(f['arrayLimitations']))
     if status=='ok':status='partial_extraction'
    if encoding_bad and status!='not_applicable':
     reason+='; invalid UTF-8: text extraction used replacement characters'
     if status=='ok':status='partial_decode'
    count=len(f[{'M1':'texts','M2':'numbers','M3':'arrays','M4':'tokens'}[m]])
    rows.append([r['id'],r['side'],r['repo'],loc['ref'],loc['path'],r['blob'],loc['first_commit'],loc['scope'],r['language'],m,status,len(f['errors']),count,reason])
    stats[(r['side'],m,status)]+=1
 for x in csv.DictReader((o/'missing-inputs.tsv').open(),delimiter='\t'):
  for m in ['M1','M2','M3','M4']:rows.append(['','corpus',x['repo'],x['ref'],x['path'],x['blob'],'','','',m,'missing_input',0,0,x['reason']])
 write_tsv(o/'encoding-errors.tsv',['record','repo','path','byte_offset','limitation'],encoding_errors)
 summary['encoding_failures']=len(encoding_errors)
 write_tsv(o/'coverage.tsv',['record','side','repo','ref','path','blob','first_commit','scope','language','method','status','parse_error_nodes','extracted_items','exclusion_or_limitation'],rows)
 rust=[r for r in R if r['side']=='minase' and r['language']=='rust'];base=[r for r in rust if any(x['scope']=='baseline' for x in r['locations'])]
 summary['coverage']={'rows':len(rows),'statuses':{'|'.join(k):v for k,v in stats.items()},'minase_rust_blobs':len(rust),'minase_rust_success':sum(F[r['id']]['status']=='ok' for r in rust),'baseline_rust_files':sum(sum(x['scope']=='baseline' for x in r['locations']) for r in base),'baseline_rust_success':sum(sum(x['scope']=='baseline' for x in r['locations']) for r in base if F[r['id']]['status']=='ok'),'parse_failures_by_side':dict(collections.Counter(r['side'] for r in R if F[r['id']]['status'] in ['partial_parse','failed','lexical_partial']))}
 write_tsv(o/'parse-errors.tsv',['record','location','status','errors'],[(r['id'],location(r,1),F[r['id']]['status'],json.dumps(F[r['id']]['errors'])) for r in R if F[r['id']]['errors'] or F[r['id']]['status']=='failed'])

def main():
 p=argparse.ArgumentParser();p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--method',choices=['M1','M2','M3','M4','coverage','all'],default='all');a=p.parse_args();o=a.out.resolve();R=[json.loads(l) for l in (o/'inventory.jsonl').open()];F={}
 for line in (o/'features.jsonl').open():
  f=json.loads(line);F[f['id']]=f
 if len(F)!=len(R):raise RuntimeError(f'Incomplete feature extraction: {len(F)} / {len(R)}')
 for r in R:
  if r['language']=='toml':lexical_toml(r,F[r['id']])
 summary=json.loads((o/'summary.json').read_text()) if (o/'summary.json').exists() else {}
 coverage(o,R,F,summary)
 for method in ['M1','M2','M3','M4']:
  if a.method not in [method,'all']:continue
  print('starting',method,flush=True);globals()['run_'+method.lower()](o,R,F,summary);(o/'summary.json').write_text(json.dumps(summary,ensure_ascii=False,indent=2)+'\n')
 (o/'summary.json').write_text(json.dumps(summary,ensure_ascii=False,indent=2)+'\n')
if __name__=='__main__':main()
