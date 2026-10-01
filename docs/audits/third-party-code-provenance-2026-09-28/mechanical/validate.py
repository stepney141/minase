"""Final artifact verification against the concrete requirements in spec.md."""
import argparse,csv,hashlib,json,pathlib,subprocess,sys
p=argparse.ArgumentParser();p.add_argument('--out',type=pathlib.Path,required=True);a=p.parse_args();o=a.out.resolve();s=json.loads((o/'summary.json').read_text());i=json.loads((o/'inventory-summary.json').read_text());counts=json.loads((o/'output-counts.json').read_text());required=['README.md','report.md','coverage.tsv','refs.tsv','inventory.jsonl','blob-first-occurrence.tsv','m1_hits.tsv','m1_texts.jsonl','m2_hits.tsv','m2_common_values.tsv','m2_literals.tsv','m2_params.tsv','m2_params_introductions.tsv','m3_hits.tsv','m3_arrays.jsonl','m3_calibration.tsv','m4_hits.tsv','m4_calibration.tsv','attribution.tsv','missing-inputs.tsv','m3_array_limitations.tsv','tool-versions.json','source-verification.json','exclusion-counts.tsv','file-license-overrides.tsv','inventory.py','extract.cjs','checks.py','annotate.py','report.py','text_normalization.py','fixture_tokens.cjs','test_checks.py','extraction_tests.py','encoding-errors.tsv','m4-sample-verification.json'];checks=[]
for name in required:
 p=o/name
 if not p.is_file() or not p.stat().st_size:raise RuntimeError('Missing/empty artifact: '+name)
 h=hashlib.sha256()
 with p.open('rb') as f:
  while block:=f.read(1024*1024):h.update(block)
 checks.append({'path':name,'bytes':p.stat().st_size,'sha256':h.hexdigest()})
for m in ['M1','M2','M3','M4']:
 if counts[m]['candidates']!=s[m]['candidates']:raise RuntimeError('Candidate count mismatch: '+m)
assert s['M3']['calibration_pass']
assert s['coverage']['baseline_rust_success']==s['coverage']['baseline_rust_files']
assert s['coverage']['minase_rust_success']==s['coverage']['minase_rust_blobs']
coverage=list(csv.DictReader((o/'coverage.tsv').open(),delimiter='\t'));recs=[json.loads(l) for l in (o/'inventory.jsonl').open()];cov={(r['record'],r['method']) for r in coverage}
assert all((str(r['id']),m) in cov for r in recs for m in ['M1','M2','M3','M4'])
ph=json.loads((o/'tools/tree-sitter/sha256.json').read_text());assert all(hashlib.sha256((o/'tools/tree-sitter'/name).read_bytes()).hexdigest()==digest for name,digest in ph.items())
result={'spec_sha256':hashlib.sha256((o/'spec.md').read_bytes()).hexdigest(),'artifacts':checks,'implemented_and_executed':['M1','M2','M3','M4 supplementary tree-sitter winnowing'],'strict_spec_complete':False,'blocked_requirements':{'all_methods':{'missing_refs':i['missing_refs'],'missing_blobs':i['missing_blobs']},'M4':'JPlag/Dolos unavailable; no actual JPlag parse-failure assessment; supplementary route only'},'all_inventory_records_have_four_coverage_rows':True,'counts_match':True,'tree_sitter_binary_checksums_match':True,'m3_calibration_pass':True,'summary':{k:s[k] for k in ['M1','M2','M3','M4']}}
(o/'verification.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n');print(json.dumps({k:v for k,v in result.items() if k not in ['summary','artifacts']},ensure_ascii=False,indent=2))
