import argparse, csv, hashlib, json, pathlib, subprocess, collections, sys, os
EXT={'.rs':'rust','.py':'python','.sh':'bash','.toml':'toml','.patch':'patch','.c':'c','.h':'cpp','.cc':'cpp','.cpp':'cpp','.cxx':'cpp','.hpp':'cpp','.hxx':'cpp','.scala':'scala','.ts':'typescript','.tsx':'typescript','.js':'javascript','.cs':'c_sharp','.java':'java'}
os.environ['GIT_NO_LAZY_FETCH']='1'
os.environ['GIT_OPTIONAL_LOCKS']='0'
VENDOR={'3rdparty','third_party','third-party','external','deps','vendor','incbin','thirdparty'}
def git(repo,*args):return subprocess.check_output(['git','-C',str(repo),*args])
def tree(repo,rev):
 for item in git(repo,'ls-tree','-rz',rev).split(b'\0'):
  if not item:continue
  meta,path=item.split(b'\t',1); mode,typ,oid=meta.decode().split()
  if typ=='blob':yield oid,path.decode('utf8','surrogateescape')
def main():
 p=argparse.ArgumentParser();p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--manifest',type=pathlib.Path,required=True);p.add_argument('--minase',type=pathlib.Path,required=True);p.add_argument('--base',default='a05478a');a=p.parse_args();o=a.out.resolve();(o/'sources').mkdir(exist_ok=True)
 records={}; refs=[];excluded=[];first={};occ=[];missing=[]
 def add(side,repo,local,rev,oid,path,license,category,commit='',scope=''):
  ext=pathlib.Path(path).suffix.lower()
  if ext not in EXT:return
  if side=='corpus' and set(path.lower().split('/'))&VENDOR:
   excluded.append([repo,rev,path,'bundled_library']);return
  key=(side,repo,oid,EXT[ext]); loc={'ref':rev,'path':path,'first_commit':commit,'scope':scope}
  if key in records:
   if loc not in records[key]['locations']:records[key]['locations'].append(loc)
   return
  rid=len(records);dest=o/'sources'/(oid+ext)
  if not dest.exists():
   try: dest.write_bytes(git(local,'show',(commit if side=='minase' and commit else rev)+':'+path) if local else pathlib.Path(rev,path).read_bytes())
   except subprocess.CalledProcessError:
    missing.append([repo,rev,path,oid,'missing_local_blob']);return
  records[key]={'id':rid,'side':side,'repo':repo,'blob':oid,'language':EXT[ext],'source':str(dest),'license':license,'category':category,'locations':[loc]}
 objs=git(a.minase,'rev-list','--objects','--all').decode().splitlines();(o/'minase-rev-list-objects.txt').write_text('\n'.join(objs)+'\n')
 commits=git(a.minase,'rev-list','--reverse','--topo-order','--all').decode().splitlines()
 for i,c in enumerate(commits):
  for oid,path in tree(a.minase,c):
   if pathlib.Path(path).suffix.lower() not in EXT:continue
   if (oid,path) not in first:first[(oid,path)]=c;occ.append([oid,path,c,i])
  if i%100==0:print('history',i,len(first),flush=True)
 for (oid,path),commit in first.items():add('minase','minase',a.minase,'all-refs',oid,path,'unassessed','target',commit,'history')
 for oid,path in tree(a.minase,a.base):add('minase','minase',a.minase,a.base,oid,path,'unassessed','target',first.get((oid,path),a.base),'baseline')
 (o/'RULES.md').write_bytes(git(a.minase,'show',a.base+':RULES.md'))
 rows=list(csv.DictReader(a.manifest.open(),delimiter='\t'))
 for row in rows:
  repo=row['repo'];local=pathlib.Path(row['local_path']); revs=['HEAD']
  if repo=='official-stockfish/Stockfish':revs+=['sf_'+str(i) for i in range(10,20)]
  if repo=='yaneurao/YaneuraOu':revs+=['V5.00','v6.00','v7.00','v7.10','V7.61','v8.00','V8.30','V9.00','v9.10','v9.40','33ccf1f','0a6dd2cb^','v8.00-fukauraou','v9.10-fukauraou']
  if repo=='official-stockfish/fishtest':revs+=['b8eecff220b562a0dc2c4e68d1fa02521e06d72c']
  if not local.exists():refs.append([repo,'HEAD','','missing_path']);continue
  if not (local/'.git').exists():
   refs.append([repo,'snapshot','','snapshot_unversioned'])
   for path in sorted(local.rglob('*')):
    if path.is_file() and path.suffix.lower() in EXT:
     content=path.read_bytes();oid=hashlib.sha256(content).hexdigest();add('corpus',repo,None,str(local),oid,str(path.relative_to(local)),row['license_spdx'],row['category'])
   continue
  for rev in revs:
   cp=subprocess.run(['git','-C',str(local),'rev-parse','--verify',rev+'^{commit}'],capture_output=True,text=True)
   if cp.returncode:refs.append([repo,rev,'','missing_ref']);continue
   sha=cp.stdout.strip();refs.append([repo,rev,sha,'supplementary_release_tag' if rev.endswith('-fukauraou') else 'ok'])
   for oid,path in tree(local,sha):
    lic=row['license_spdx'];cat=row['category']
    if repo=='SH11235/rshogi' and path.startswith('crates/rshogi-csa/'):lic='MIT';cat='permissive'
    add('corpus',repo,local,rev,oid,path,lic,cat)
  print('corpus',repo,len(records),flush=True)
 with (o/'inventory.jsonl').open('w') as f:
  for r in records.values():f.write(json.dumps(r,ensure_ascii=False)+'\n')
 for name,header,data in [('missing-inputs.tsv',['repo','ref','path','blob','reason'],missing),('refs.tsv',['repo','requested_ref','commit','status'],refs),('excluded-files.tsv',['repo','ref','path','reason'],excluded),('blob-first-occurrence.tsv',['blob','path','first_commit','topological_index'],occ)]:
  with (o/name).open('w') as f:w=csv.writer(f,delimiter='\t');w.writerow(header);w.writerows(data)
 (o/'inventory-summary.json').write_text(json.dumps({'commits':len(commits),'records':len(records),'minase_blobs':len({r['blob'] for r in records.values() if r['side']=='minase'}),'manifest_rows':len(rows),'missing_refs':[r for r in refs if r[-1].startswith('missing')],'excluded_files':len(excluded),'missing_blobs':len(missing)},indent=2))
if __name__=='__main__':main()
