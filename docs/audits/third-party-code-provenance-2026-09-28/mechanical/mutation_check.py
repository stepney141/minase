import json,pathlib,subprocess,sys
out=pathlib.Path(sys.argv[1]).resolve();src=(out/'checks.py').read_text();root=out/'mutations';root.mkdir(exist_ok=True);results=[]
mutations=[('drop_negative_sign',"s=re.sub(r'\\s+','',raw).replace('_','').replace(\"'\",'')","s=re.sub(r'\\s+','',raw).replace('_','').replace(\"'\",'').lstrip('-')"),('reject_threshold_boundary','if z>=threshold:matches.append((x,y,z))','if z>threshold:matches.append((x,y,z))'),('wrong_percentage_denominator',"return value/100,'ratio'","return value/1000,'ratio'")]
for name,before,after in mutations:
 assert before in src,name
 path=root/(name+'.py');path.write_text(src.replace(before,after,1))
 runner="import importlib.util,sys,unittest;sys.path.insert(0,sys.argv[1]);s=importlib.util.spec_from_file_location('checks',sys.argv[2]);m=importlib.util.module_from_spec(s);sys.modules['checks']=m;s.loader.exec_module(m);import test_checks;r=unittest.TextTestRunner().run(unittest.defaultTestLoader.loadTestsFromModule(test_checks));sys.exit(not r.wasSuccessful())"
 p=subprocess.run([sys.executable,'-B','-c',runner,str(out),str(path)],capture_output=True,text=True);(root/(name+'.log')).write_text(p.stdout+p.stderr);results.append({'mutation':name,'detected':p.returncode!=0,'returncode':p.returncode})
(out/'mutation-results.json').write_text(json.dumps(results,indent=2)+'\n');assert all(r['detected'] for r in results);print('All 3 requirement-relevant semantic mutations were detected.')
