import os
os.environ['OMP_NUM_THREADS']='1'
os.environ['OPENBLAS_NUM_THREADS']='1'
import sys
sys.path.insert(0,'/tmp/codex-pst-diag')
from diagnose import *
from minase_train.data.features import PIECE_STATE_BY_BYTE, COLOR_BY_BYTE
from minase_train.pst.removal import make_removal_reference, removal_loss
r=map_records(ROOT/'data/strength-stage9/human/human.bin')
x=np.load(OUT/'human_scores.npz'); direct=np.zeros(len(r),np.int64)
for i in range(0,len(r),8192):
    rr=r[i:i+8192]; states=PIECE_STATE_BY_BYTE[rr['board']]; occupied=rr['board']!=0
    vals=PIECE_VALUES[STATE_KIND[states]]; vals=np.where(COLOR_BY_BYTE[rr['board']]==rr['stm'][:,None],vals,-vals)
    direct[i:i+len(rr)]=np.clip(np.where(occupied,vals,0).sum(axis=1),-28999,28999)
assert np.array_equal(x['scores'][:,0],direct)
assert (ROOT/'crates/minase/nets/pst.bin').read_bytes()==(ROOT/'data/fm-quarter-current-pst-fa/fm-quarter.bin').read_bytes()
a=read_pst(OUT/'G01_single.bin'); b=read_pst(OUT/'taper_transition.bin')
assert np.array_equal(a[0],b[0]) and np.array_equal(a[0],b[1])
# Compare reference-dependent auxiliary losses on an identical fixed training sample.
p=ROOT/'data/strength-stage7/gen2/generated-600000.bin'; rr=map_records(p); h=read_header(p)
ix=np.flatnonzero(hash64(h.seed,rr['game'])%20!=0); rng=np.random.default_rng(1); rr=rr[np.sort(rng.choice(ix,4096,replace=False))]
f=feature_indices(rr['board'],rr['stm'],rr['lion']); ft=torch.as_tensor(f); ct=torch.as_tensor(np.count_nonzero(rr['board'],axis=1),dtype=torch.int64)
weights={'v0':(initial_weights(),initial_weights()),'Pc':read_pst(ROOT/'data/pst-longer-training/pst-base.bin')[:2],'L':read_pst(ROOT/'data/pst-longer-training/training/pst.bin')[:2]}
losses=[]
for ref in ['v0','Pc']:
    rt=make_removal_reference(*weights[ref],torch.device('cpu'))
    for candidate in weights:
        ext=np.zeros((N+1,2)); ext[:N]=np.stack(weights[candidate],axis=1)/8
        loss=float(removal_loss(torch.as_tensor(ext[f]),ft,rt,ct,K))
        losses.append({'reference':ref,'candidate':candidate,'removal_loss':loss,'1000x_loss':1000*loss})
save('checks.json',{'v0_direct_material_exact':True,'M_equal_Fa_quarter_file':True,'b687022_duplicates_previous_single_endpoints':True,'removal_sample_size':4096,'removal_losses':losses})
print(json.dumps(losses,indent=2))
