"""End-to-end extraction fixtures specified independently of audited implementations."""
import json,pathlib,subprocess,sys
out=pathlib.Path(sys.argv[1]).resolve();d=out/'validation-fixtures';d.mkdir(exist_ok=True)
if not (d/'tools').exists():(d/'tools').symlink_to(out/'tools',target_is_directory=True)
fixtures={'rust':('case.rs','''const A: [i32; 8] = [3, 5, 7, 11, 13, 17, 19, 23];
const B: [i32; 9] = [7; 9];
fn exercise() { let complement = !0xffu32; let signed = -123i32; println!("not numeric: 999"); }
'''),'python':('case.py','''# not numeric: 999
values = [3, 5, 7, 11, 13, 17, 19, 23]
text = f"{31 + 37}"
'''),'cpp':('case.cpp','''const int values[8] = {3,5,7,11,13,17,19,23};
const char *message = "foo" "bar";
int f() { return ~0xffU + -123; }
'''),'typescript':('case.ts','''let count: number = 37;
'''),'bash':('case.sh','''#!/bin/sh
exit 23
''')}
records=[]
for i,(lang,(name,source)) in enumerate(fixtures.items()):
 p=d/name;p.write_text(source);records.append({'id':i,'language':lang,'source':str(p),'repo':'spec-fixture'})
(d/'inventory.jsonl').write_text(''.join(json.dumps(r)+'\n' for r in records))
subprocess.run(['node',str(out/'extract.cjs'),str(d)],check=True)
f={r['id']:r for r in map(json.loads,(d/'features.jsonl').open())}
assert all(r['status']=='ok' for r in f.values()),{k:v['errors'] for k,v in f.items()}
rust=f[0];rustnums=[n[1] for n in rust['numbers']];assert '0xffu32' in rustnums and '!0xffu32' not in rustnums and '-123i32' in rustnums and '999' not in rustnums
assert any(a[2]=='repeat' and a[3]==['7']*9 for a in rust['arrays'])
assert any(a[3]==['3','5','7','11','13','17','19','23'] for a in rust['arrays'])
assert {'31','37'} <= {n[1] for n in f[1]['numbers']}
assert 'number' not in [n[1] for n in f[3]['numbers']]
assert '23' in [n[1] for n in f[4]['numbers']]
from text_normalization import decode_strings
assert any(decode_strings(t[3],'cpp')=='foobar' for t in f[2]['texts'])
print('8 end-to-end extraction assertions passed across Rust, Python, C++, TypeScript and shell.')
