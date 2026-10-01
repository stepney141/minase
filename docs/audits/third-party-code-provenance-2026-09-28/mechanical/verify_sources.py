import argparse,hashlib,json,os,pathlib,subprocess
p=argparse.ArgumentParser();p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--minase',required=True);a=p.parse_args();o=a.out.resolve();os.environ['GIT_NO_LAZY_FETCH']='1';os.environ['GIT_OPTIONAL_LOCKS']='0';records=[json.loads(l) for l in (o/'inventory.jsonl').open()];checked=0;baseline=0
for r in records:
 data=pathlib.Path(r['source']).read_bytes();actual=hashlib.sha1(b'blob '+str(len(data)).encode()+b'\0'+data).hexdigest() if len(r['blob'])==40 else hashlib.sha256(data).hexdigest();assert actual==r['blob'],r['source'];checked+=1
 if r['side']=='minase':
  for loc in [r['locations'][0]]+[x for x in r['locations'] if x['scope']=='baseline']:
   rev='a05478a' if loc['scope']=='baseline' else loc['first_commit'];content=subprocess.check_output(['git','-C',a.minase,'show',rev+':'+loc['path']]);assert content==data,(r['id'],loc);baseline+=loc['scope']=='baseline'
(o/'source-verification.json').write_text(json.dumps({'records_hash_verified':checked,'baseline_paths_verified_with_git_show_a05478a':baseline,'history_blobs_verified_with_git_show_first_commit':sum(r['side']=='minase' for r in records)},indent=2)+'\n');print('Verified all source hashes and minase baseline/history commit:path reads.')
