import json,re
# Run from the M1-M4 work directory; inventory.jsonl is read from the current directory.
INVENTORY_PATH='inventory.jsonl'
INV={}
with open(INVENTORY_PATH) as f:
    for l in f:
        d=json.loads(l); INV[d['id']]=d
def paths(rid): return sorted({x['path'] for x in INV[rid]['locations']})
def at_base(rid): return any('a05478a' in x['ref'] or x.get('scope')=='base' for x in INV[rid]['locations'])
