import re
LOC=re.compile(r'^(.*?)@([^:]*):(.*):(\d+) \[blob=(\w+)\]$')
def parse(s):
    m=LOC.match(s)
    if not m: return None
    return dict(repo=m.group(1),refs=m.group(2),path=m.group(3),line=int(m.group(4)),blob=m.group(5))
K=[
 ('K1', lambda mp: 'time_management' in mp, lambda r,p: 'Stockfish' in r and re.search(r'(search|timeman)\.(cpp|h)$',p)),
 ('K2', lambda mp: mp.endswith('stats.rs') and mp.startswith('src/stats') , lambda r,p: 'fishtest' in r and 'LLRcalc' in p),
 ('K3', lambda mp: 'magic-bitboard-prototype' in mp, lambda r,p: 'Stockfish' in r and re.search(r'(bitboard|misc)\.(cpp|h)$',p)),
 ('K4', lambda mp: 'handcrafted' in mp or 'train_pst' in mp, lambda r,p: 'HaChu' in r),
 ('K5', lambda mp: mp.endswith('params.rs') and 'search' in mp, lambda r,p: re.search(r'akimbo|hobbes|mixed SPSA',r)),
]
