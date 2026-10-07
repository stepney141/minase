import os
os.environ['OMP_NUM_THREADS']='1'
os.environ['OPENBLAS_NUM_THREADS']='1'
os.environ['MKL_NUM_THREADS']='1'
import sys, json, hashlib, struct, subprocess, csv, time
from pathlib import Path
import numpy as np
import torch
torch.set_num_threads(1)
from minase_train.data.features import feature_indices, canonical_feature_indices, FEATURE_COUNT
from minase_train.data.mnpt import initial_weights, read_mnpt, read_mnpt_v3, STATE_KIND, PIECE_VALUES
from minase_train.data.mnsd import map_records, read_header, hash64
from minase_train.data.taper import phase_numerators
from minase_train.pst.evaluate import integer_evaluate
from minase_train.fm.train import integer_evaluate as fm_integer, float_evaluate as fm_float
ROOT=Path('/home/stepney141/board-games/minase')
OUT=Path('/tmp/codex-pst-diag')
K=1072.6529541015625
N=FEATURE_COUNT

def save(name,x):
    (OUT/name).write_text(json.dumps(x,indent=2,ensure_ascii=False,allow_nan=False)+'\n')

def historical(name,commit,path):
    raw=subprocess.check_output(['git','show',f'{commit}:{path}'],cwd=ROOT)
    dest=OUT/(name+'.bin'); dest.write_bytes(raw)
    return dest

def read_pst(path):
    raw=Path(path).read_bytes(); magic,ver,n,k=struct.unpack_from('<4sIIf',raw)
    assert magic==b'MNPT' and n==N and hashlib.sha256(raw[80:]).digest()==raw[48:80]
    if ver==1:
        assert len(raw)==80+N*2
        w=np.frombuffer(raw,dtype='<i2',offset=80).copy()
        return w,w,k
    assert ver==2
    mg,eg,_,k=read_mnpt(path)
    return mg,eg,k

def calibrate(x,y,w,intercept=False):
    # Minimize game-balanced BCE; no regularization, fit only diagnostic games.
    a=1.; b=0.
    def loss(a,b):
        z=a*x+b
        return np.dot(w,np.logaddexp(0,z)-y*z)
    old=loss(a,b)
    for step in range(100):
        z=a*x+b; p=np.exp(-np.logaddexp(0,-z)); d=w*(p-y); h=w*p*(1-p)
        if intercept:
            g=np.array([np.dot(d,x),d.sum()]); H=np.array([[np.dot(h,x*x),np.dot(h,x)],[np.dot(h,x),h.sum()]])
            delta=np.linalg.solve(H,g)
        else:
            g=np.array([np.dot(d,x)]); delta=np.array([g[0]/np.dot(h,x*x),0.])
        if np.max(np.abs(g))<1e-11: break
        rate=1.
        while True:
            aa=a-rate*delta[0]; bb=b-rate*delta[1]
            val=loss(aa,bb)
            if aa>0 and val<=old+1e-15: break
            rate/=2
            assert rate>1e-12
        a,b,old=aa,bb,val
    assert np.max(np.abs(g))<1e-8
    return float(a),float(b)

def d1():
    path=ROOT/'data/strength-stage9/human/human.bin'
    records=map_records(path); games=json.loads(Path(str(path)+'.games.json').read_text()); prov=json.loads(Path(str(path)+'.provenance.json').read_text())
    assert hashlib.sha256(path.read_bytes()).hexdigest()==prov['mnsd_sha256']
    lookup={entry['game']:entry['id'] for entry in prov['games']}
    assert all(lookup[v['game']]==key for key,v in games.items())
    nums,inv,counts=np.unique(records['game'],return_inverse=True,return_counts=True)
    valid=np.array([int.from_bytes(hashlib.sha256(lookup[int(g)].encode()).digest()[:8],'little')%2==0 for g in nums])
    y=records['result'].astype(float)/2
    winner=np.array([0 if games[lookup[int(g)]]['winner']=='sente' else 1 for g in nums])[inv]
    assert np.array_equal(y,(winner==records['stm']).astype(float))
    assert (len(nums),len(records),valid.sum(),counts[valid].sum())==(2035,303592,1020,151630)
    sources=[('G0','588363a','nets/pst.bin'),('G01_single','af200b4','nets/pst.bin'),('G01_tapered','77fe8c0','nets/pst.bin'),('stage7','960c56b','nets/pst.bin'),('G2_removal','240fbfd','nets/pst.bin'),('S0_lookahead','a7e32a8','nets/pst.bin')]
    models={'v0_material':(initial_weights(),initial_weights(),K)}
    meta=[]
    for name,c,p in sources:
        path2=historical(name,c,p); models[name]=read_pst(path2)
        meta.append({'name':name,'commit':c,'path':p,'sha256':hashlib.sha256(path2.read_bytes()).hexdigest()})
    for name,p in [('G23','data/pst-gen3-g23/training/pst.bin'),('Pc','data/pst-longer-training/pst-base.bin'),('L','data/pst-longer-training/training/pst.bin')]:
        models[name]=read_pst(ROOT/p); meta.append({'name':name,'path':p,'sha256':hashlib.sha256((ROOT/p).read_bytes()).hexdigest()})
    mg,eg,pv,mk,u,signs,exponent=read_mnpt_v3(ROOT/'crates/minase/nets/pst.bin')
    assert np.array_equal(mg,models['L'][0]) and np.array_equal(eg,models['L'][1]) and mk==K
    fm=np.load(ROOT/'data/fm-quarter-current-pst-fa/fm-float.npz')
    names=list(models)+['M','L_Fa_full_float']
    scores=np.empty((len(records),len(names)),float)
    human_counts=np.zeros(N,np.int64)
    for start in range(0,len(records),8192):
        r=records[start:start+8192]; f=feature_indices(r['board'],r['stm'],r['lion']); q=phase_numerators(r['board'])
        human_counts+=np.bincount(f.ravel(),minlength=N+1)[:N]
        for j,name in enumerate(models):
            scores[start:start+len(r),j]=integer_evaluate(*models[name][:2],f,q)
        scores[start:start+len(r),-2]=fm_integer(mg,eg,u,signs,exponent,f,q)
        scores[start:start+len(r),-1]=fm_float(mg,eg,K,fm['V'],fm['a'],f,q)
    np.savez(OUT/'human_scores.npz',scores=scores,names=np.array(names),y=y,inv=inv,counts=counts,valid=valid,human_counts=human_counts)
    fixed=[]; calibrated=[]; intercept_losses=[]; table=[]
    train=~valid[inv]; ww=1/counts[inv[train]]/(~valid).sum()
    for j,name in enumerate(names):
        x=scores[:,j]/K; a,b=calibrate(x[train],y[train],ww); ai,bi=calibrate(x[train],y[train],ww,True)
        means=[]
        for aa,bb in [(1,0),(a,b),(ai,bi)]:
            z=aa*x+bb; loss=np.logaddexp(0,z)-y*z
            means.append(np.bincount(inv,weights=loss)/counts)
        fixed.append(means[0]); calibrated.append(means[1]); intercept_losses.append(means[2])
        table.append({'model':name,'fixed_all':means[0].mean(),'fixed_validation':means[0][valid].mean(),'scale':a,'calibrated_validation':means[1][valid].mean(),'scale_with_intercept':ai,'intercept':bi,'intercept_validation':means[2][valid].mean()})
    fixed=np.array(fixed).T; calibrated=np.array(calibrated).T
    pairs=[(i,i+1) for i in range(len(names)-1)]+[(0,names.index('L')),(names.index('L'),names.index('L_Fa_full_float'))]
    pairs=list(dict.fromkeys(pairs)); differences=[]
    for kind,arr in [('fixed_all',fixed),('fixed_validation',fixed[valid]),('calibrated_validation',calibrated[valid])]:
        dif=np.column_stack([arr[:,b]-arr[:,a] for a,b in pairs]); rng=np.random.default_rng(1)
        boot=np.empty((2000,len(pairs)))
        for rep in range(2000): boot[rep]=dif[rng.integers(len(arr),size=len(arr))].mean(axis=0)
        for j,(a,b) in enumerate(pairs):
            differences.append({'kind':kind,'from':names[a],'to':names[b],'delta':dif[:,j].mean(),'se':boot[:,j].std(ddof=1),'ci95':np.quantile(boot[:,j],[.025,.975]).tolist()})
    save('d1.json',{'K':K,'games':len(nums),'positions':len(records),'diagnostic_games':int((~valid).sum()),'validation_games':int(valid.sum()),'sources':meta,'table':table,'differences':differences,'bootstrap':'2000 paired game resamples, seed 1; calibration held fixed'})
    for row in table: print(row,flush=True)

def d2_file(index):
    entries=json.loads((ROOT/'data/pst-longer-training/training/inputs.json').read_text())['data']
    path=Path(entries[index]['path']); header=read_header(path); r=map_records(path)
    assert hashlib.sha256(path.read_bytes()).hexdigest()==entries[index]['sha256']
    raw=np.zeros(N,np.int64); counts=np.zeros(N//2,np.int64); sums=np.zeros(N//2); sums2=sums.copy(); nr=0; ng=set()
    for start in range(0,len(r),32768):
        rr=r[start:start+32768]; use=hash64(header.seed,rr['game'])%np.uint64(20)!=0; rr=rr[use]; nr+=len(rr); ng.update(np.unique(rr['game']).tolist())
        f=feature_indices(rr['board'],rr['stm'],rr['lion']); raw+=np.bincount(f.ravel(),minlength=N+1)[:N]
        c=canonical_feature_indices(f); phi=phase_numerators(rr['board'])/90
        counts+=np.bincount(c.ravel(),minlength=N//2+1)[:N//2]
        pp=np.broadcast_to(phi[:,None],c.shape).ravel()
        sums+=np.bincount(c.ravel(),weights=pp,minlength=N//2+1)[:N//2]
        sums2+=np.bincount(c.ravel(),weights=pp*pp,minlength=N//2+1)[:N//2]
    np.savez(OUT/f'counts-{index}.npz',raw=raw,counts=counts,sums=sums,sums2=sums2,nr=nr,ng=len(ng))
    print(index,path.name,nr,len(ng),flush=True)

KINDS=['歩兵','仲人','香車','反車','横行','竪行','角行','飛車','龍馬','龍王','奔王','王将','醉象','猛豹','盲虎','銅将','銀将','金将','麒麟','鳳凰','獅子','太子','白駒','鯨鯢','飛牛','奔猪','飛鹿','角鷹','飛鷲']
def label(c):
    group=c//72; rank=(c%72)//6; file=c%6
    if group==94: return f'先獅子 r{rank} f{file}/{11-file}'
    state=group%47; return f'{"自" if group<47 else "敵"}{KINDS[int(STATE_KIND[state])]}({"未成" if state>=29 else "成/非成"},s{state}) r{rank} f{file}/{11-file}'

def d2_summary():
    parts=[np.load(OUT/f'counts-{i}.npz') for i in range(10)]
    total={k:sum(x[k] for x in parts) for k in ['raw','counts','sums','sums2','nr','ng']}
    assert total['nr']==27745843
    c=total['counts']; means=np.divide(total['sums'],c,out=np.zeros(len(c)),where=c>0); ssd=np.maximum(0,total['sums2']-c*means*means)
    pc=np.stack(read_pst(ROOT/'data/pst-longer-training/pst-base.bin')[:2],axis=1)/8
    l=np.stack(read_pst(ROOT/'data/pst-longer-training/training/pst.bin')[:2],axis=1)/8
    canonical=canonical_feature_indices(np.arange(N)); inds=np.array([np.flatnonzero(canonical==i)[0] for i in range(N//2)])
    assert np.array_equal(pc,pc[inds][canonical]) and np.array_equal(l,l[inds][canonical])
    pc,l=pc[inds],l[inds]; delta=np.abs(l-pc)
    rows=[]; top=[]
    ranges=[(0,0),(1,10),(11,100),(101,1000),(1001,10000),(10001,10**12)]
    for end in range(2):
        for lo,hi in ranges:
            sel=(c>=lo)&(c<=hi)
            rows.append({'endpoint':['MG','EG'][end],'range':f'{lo}-{hi}','features':int(sel.sum()),'delta_mean':float(delta[sel,end].mean()),'delta_max':float(delta[sel,end].max()),'L_abs_mean':float(np.abs(l[sel,end]).mean()),'L_abs_max':float(np.abs(l[sel,end]).max())})
        for rank,i in enumerate(np.argsort(-delta[:,end],kind='stable')[:50],1):
            top.append({'endpoint':['MG','EG'][end],'rank':rank,'canonical':int(i),'label':label(i),'count':int(c[i]),'Pc':float(pc[i,end]),'L':float(l[i,end]),'abs_delta':float(delta[i,end]),'mean_phi':float(means[i]),'phi_ssd':float(ssd[i])})
    hc=np.load(OUT/'human_scores.npz')['human_counts']; human=np.bincount(canonical,weights=hc,minlength=N//2)
    boundary=[]
    for end in range(2):
        for value in [-4096.,4095.875]: boundary.append({'endpoint':['MG','EG'][end],'value':value,'count':int((l[:,end]==value).sum())})
    allrows=[]
    for i in range(len(c)):
        allrows.append({'canonical':i,'label':label(i),'count':int(c[i]),'mean_phi':float(means[i]),'phi_ssd':float(ssd[i]),'human_count':int(human[i]),'Pc_MG':pc[i,0],'L_MG':l[i,0],'delta_MG':delta[i,0],'Pc_EG':pc[i,1],'L_EG':l[i,1],'delta_EG':delta[i,1]})
    for name,data in [('features.csv',allrows),('top50.csv',top),('bands.csv',rows)]:
        with (OUT/name).open('w') as f:
            writer=csv.DictWriter(f,fieldnames=list(data[0])); writer.writeheader(); writer.writerows(data)
    summary={'training_records':int(total['nr']),'training_games':int(total['ng']),'raw_unobserved':int((total['raw']==0).sum()),'canonical_unobserved':int((c==0).sum()),'zero_count_max_delta':delta[c==0].max(axis=0).tolist(),'boundary':boundary,'bands':rows,'top50':top,'low_ssd_features':int(((c>0)&(ssd<100)).sum()),'human_unseen_features':int(((human>0)&(c==0)).sum()),'human_unseen_occurrences':int(human[c==0].sum()),'human_total_occurrences':int(human.sum()),'delta_mean':delta.mean(axis=0).tolist(),'delta_max':delta.max(axis=0).tolist()}
    floating=np.load(ROOT/'data/pst-longer-training/training/pst-float.npz')
    summary['float_exact_boundary']=[{'endpoint':end,'lower':int((floating[key][inds]==-4096.).sum()),'upper':int((floating[key][inds]==4095.875).sum())} for end,key in [('MG','middlegame'),('EG','endgame')]]
    save('d2.json',summary); np.savez(OUT/'counts-total.npz',**total,mean_phi=means,ssd=ssd)
    print(json.dumps({k:v for k,v in summary.items() if k!='top50'},ensure_ascii=False,indent=2))

if __name__=='__main__':
    if sys.argv[1]=='d1': d1()
    elif sys.argv[1]=='d2': d2_file(int(sys.argv[2]))
    elif sys.argv[1]=='summary': d2_summary()
    else: raise ValueError(sys.argv[1])
