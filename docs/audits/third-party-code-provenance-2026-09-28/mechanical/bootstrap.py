"""Extract the installed, pinned tree-sitter runtime without installing packages."""
import argparse, hashlib, json, pathlib, shutil, struct
p=argparse.ArgumentParser();p.add_argument('--out',type=pathlib.Path,required=True);a=p.parse_args(); out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
asar=pathlib.Path('/usr/share/code/resources/app/node_modules.asar')
with asar.open('rb') as f:
 h=struct.unpack('<4I',f.read(16)); tree=json.loads(f.read(h[3]));base=8+h[1]
 for name in ['package.json','LICENSE','wasm/tree-sitter.js']:
  entry=tree
  for part in ('@vscode/tree-sitter-wasm/'+name).split('/'):entry=entry['files'][part]
  f.seek(base+int(entry['offset']));(out/pathlib.Path(name).name).write_bytes(f.read(entry['size']))
shutil.copy('/usr/share/code/resources/app/node_modules.asar.unpacked/@vscode/tree-sitter-wasm/wasm/tree-sitter.wasm',out)
for lang in ['rust','python','c','cpp','scala','typescript','javascript','bash','c_sharp','java']:
 shutil.copy('/usr/share/code/resources/app/node_modules.asar.unpacked/@github/copilot-linux-x64/tree-sitter-'+lang+'.wasm',out)
(out/'sha256.json').write_text(json.dumps({p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.iterdir()) if p.is_file()},indent=2)+'\n')
print((out/'package.json').read_text())
