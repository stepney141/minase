import csv,json,pathlib,sys
from checks import token_values
out=pathlib.Path(sys.argv[1]).resolve();samples=[]
with (out/'m4_hits.tsv').open() as f:
 header=next(csv.reader([next(f)],delimiter='\t'))
 for index,line in enumerate(f):
  if index%5000==0:samples.append(dict(zip(header,next(csv.reader([line],delimiter='\t')))))
ids={int(r[k]) for r in samples for k in ['minase_record','corpus_record']};tokens={}
for line in (out/'features.jsonl').open():
 f=json.loads(line)
 if f['id'] in ids:tokens[f['id']]=token_values(f['tokens'])
for r in samples:
 a=tokens[int(r['minase_record'])];b=tokens[int(r['corpus_record'])];i=int(r['minase_token_start']);j=int(r['corpus_token_start']);n=int(r['length_tokens']);assert a[i:i+n]==b[j:j+n] and n>=20;assert abs(n/len(a)-float(r['minase_file_coverage']))<=1e-8;assert abs(n/len(b)-float(r['corpus_file_coverage']))<=1e-8
(out/'m4-sample-verification.json').write_text(json.dumps({'sampling':'every 5000th data row, deterministic','sample_count':len(samples),'all_token_sequences_equal':True,'all_coverages_correct':True},indent=2)+'\n');print(len(samples),'local matches and coverage ratios verified against final extracted tokens')
