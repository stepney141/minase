import sys
sys.path.insert(0,'/tmp/codex-pst-diag')
from diagnose import *
from minase_train.data.features import ROYAL_STATES, BOARD_FEATURE_COUNT
mg,eg,_=read_pst(ROOT/'data/pst-longer-training/pst-base.bin')
ext=np.zeros((N+1,2),np.int64); ext[:N]=np.stack([mg,eg],axis=1)
# Exact integer reference operations from removal.py. v0 removal differences equal material values,
# except if its before/after evaluation is clipped. Check that separately on all sample positions.
v0=np.zeros(N+1,np.int64); v0[:N]=initial_weights().astype(np.int64)
entries=json.loads((ROOT/'data/pst-longer-training/training/inputs.json').read_text())['data']
for i in map(int,sys.argv[1:]):
 p=Path(entries[i]['path']); h=read_header(p); records=map_records(p)
 totals=dict(positions=0,nonroyal_occurrences=0,reference_zero=0,reference_wrong=0,reference_submargin=0,v0_clipped_positions=0,different_positions=0)
 for start in range(0,len(records),16384):
  r=records[start:start+16384]; r=r[hash64(h.seed,r['game'])%20!=0]
  f=feature_indices(r['board'],r['stm'],r['lion']); c=np.count_nonzero(r['board'],axis=1); q=np.clip(c-2,0,90); aq=np.clip(c-3,0,90)
  fw=ext[f]; sums=fw.sum(axis=1); before=q*sums[:,0]+(90-q)*sums[:,1]
  after=aq[:,None]*(sums[:,None,0]-fw[:,:144,0])+(90-aq[:,None])*(sums[:,None,1]-fw[:,:144,1])
  ib=np.clip(np.sign(before)*(np.abs(before)//720),-28999,28999)
  ia=np.clip(np.sign(after)*(np.abs(after)//720),-28999,28999)
  sg=np.sign(ia-ib[:,None]); mag=np.abs(after-before[:,None])/720
  bf=f[:,:144]; states=(bf//144)%47; eligible=(bf<BOARD_FEATURE_COUNT)&(states!=11)&(states!=21)
  expected=np.where(bf//(47*144)==0,-1,1)
  zero=eligible&(sg==0); wrong=eligible&(sg!=0)&(sg!=expected); small=eligible&(sg!=0)&(mag<.25)
  v0sum=v0[f].sum(axis=1)
  # Count any position that could clip before or after removal using maximal material change 2600 cp.
  vc=(np.abs(v0sum)> (28999-2600)*8)
  totals['positions']+=len(r); totals['nonroyal_occurrences']+=int(eligible.sum()); totals['reference_zero']+=int(zero.sum()); totals['reference_wrong']+=int(wrong.sum()); totals['reference_submargin']+=int(small.sum()); totals['v0_clipped_positions']+=int(vc.sum()); totals['different_positions']+=int((zero|wrong|small).any(axis=1).sum())
 save(f'reference-{i}.json',totals); print(i,totals,flush=True)
