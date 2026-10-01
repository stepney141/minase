const fs=require('fs'),path=require('path');
const out=path.resolve(process.argv[2]),ts=require(path.join(out,'tools/tree-sitter/tree-sitter.js'));
const FN=new Set(['function_item','function_definition','function_declaration','method_definition','method_declaration','lambda_expression','arrow_function']);
const NUM=new Set(['integer_literal','float_literal','number_literal','integer','float','number','real_literal']);
const STR=new Set(['string_literal','raw_string_literal','string','concatenated_string','template_string','character_literal','char_literal','interpreted_string_literal']);
const ARR=new Set(['array_expression','initializer_list','list','tuple','array','array_initializer']);
function children(n){return n.namedChildren.filter(c=>!c.type.includes('comment'));}
function scalar(n,constants){if((NUM.has(n.type)&&/^[0-9.]/.test(n.text))||(n.type==='word'&&/^[+-]?(?:[0-9]+(?:\.[0-9]+)?|0x[0-9a-fA-F]+)$/.test(n.text)))return n.text;if(['unary_expression','unary_operator','negative_literal'].includes(n.type)){const cc=children(n);if(cc.length===1&&NUM.has(cc[0].type)&&/^[+-]/.test(n.text))return n.text;}if(constants.has(n.text))return constants.get(n.text);return null;}
function features(parser,text,language){
 const tree=parser.parse(text);const root=tree.rootNode;const lines=text.split("\n");
 let texts=[],numbers=[],arrays=[],tokens=[],errors=[],functions=[],constants=new Map(),arrayLimitations=[];
 function decl(n){if(['const_item','static_item','preproc_def','init_declarator','assignment'].includes(n.type)){let name=n.childForFieldName('name')||n.childForFieldName('left')||n.childForFieldName('declarator'),v=n.childForFieldName('value')||n.childForFieldName('right');if(name&&v){let val=scalar(v,constants);if(val!==null)constants.set(name.text,val);else if(n.type==='preproc_def'&&/^[+-]?\d+$/.test(v.text.trim()))constants.set(name.text,v.text.trim());}}for(const c of n.namedChildren)decl(c);}
 decl(root);
 function walk(n,fn){
  if(n.type==='ERROR'||n.isMissing)errors.push([n.startPosition.row+1,n.endPosition.row+1,n.type]);
  if(FN.has(n.type)){let name=n.childForFieldName('name')||n.childForFieldName('declarator');fn=(name?name.text.split('\n')[0]:'<lambda>')+'@'+(n.startPosition.row+1);functions.push([fn,n.startPosition.row+1,n.endPosition.row+1,n.startIndex,n.endIndex]);}
  if(n.type.includes('comment')){texts.push([n.startPosition.row+1,n.endPosition.row+1,'comment',n.text,n.startIndex,n.endIndex]);return;}
  if(STR.has(n.type)){texts.push([n.startPosition.row+1,n.endPosition.row+1,'string',n.text,n.startIndex,n.endIndex]);if(language==='rust'||language==='python')tokens.push(['S',n.startPosition.row+1,n.endPosition.row+1,n.startIndex,n.endIndex,n.text]);const nt=tokens.length;function embedded(c){if(c.type.includes('interpolation'))walk(c,fn);else for(const q of c.namedChildren)embedded(q);}for(const c of n.namedChildren)embedded(c);tokens.length=nt;return;}
  let sv=scalar(n,constants);
  if(sv!==null && (NUM.has(n.type)||n.type==='word'||['unary_expression','unary_operator','negative_literal'].includes(n.type))){let pos=n.startPosition.row;numbers.push([pos+1,sv,fn,lines.slice(Math.max(0,pos-2),pos+1).join('\n')]);if(language==='rust'||language==='python')tokens.push(['N',pos+1,n.endPosition.row+1,n.startIndex,n.endIndex,n.text]);return;}
  if(ARR.has(n.type)){
   const cs=children(n);const values=cs.map(c=>scalar(c,constants));
   if(cs.length>=8&&values.every(x=>x!==null))arrays.push([n.startPosition.row+1,n.endPosition.row+1,'literal',values,cs.map(c=>c.startPosition.row+1)]);
   if(cs.length>=8&&values.some(x=>x===null))arrays.push([n.startPosition.row+1,n.endPosition.row+1,'literal_symbolic',cs.map((c,i)=>values[i]===null?'@expr:'+c.text.replace(/\s+/g,' '):values[i]),cs.map(c=>c.startPosition.row+1)]);
   const repeat=n.childForFieldName('length');
   if(n.type==='array_expression'&&repeat&&cs.length===2){const size=scalar(repeat,constants),value=scalar(cs[0],constants);if(size!==null&&/^\d[\d_]*$/.test(size)){const count=Number(size.replace(/_/g,''));if(count>=8)arrays.push([n.startPosition.row+1,n.endPosition.row+1,'repeat',Array(count).fill(value===null?'@expr:'+cs[0].text:value),Array(count).fill(cs[0].startPosition.row+1)]);}else arrayLimitations.push([n.startPosition.row+1,'repeat_length_not_resolved',repeat.text]);}

   if(cs.length>=8&&cs.every(c=>ARR.has(c.type)||c.type==='tuple_expression')){
    const rows=cs.map(c=>children(c));let width=Math.max(...rows.map(r=>r.length));
    for(let k=0;k<width;k++){let val=rows.map(r=>r[k]?scalar(r[k],constants):null);let kept=val.map((v,i)=>[v,i]).filter(x=>x[0]!==null);if(kept.length>=8)arrays.push([n.startPosition.row+1,n.endPosition.row+1,'column:'+k,kept.map(x=>x[0]),kept.map(x=>rows[x[1]][k].startPosition.row+1)]);}
   }
  }
  if(n.childCount===0){if((language==='rust'||language==='python')&&n.text.trim()){let t=/identifier|name$/.test(n.type)?'I':n.text;tokens.push([t,n.startPosition.row+1,n.endPosition.row+1,n.startIndex,n.endIndex,n.text]);}return;}
  for(const c of n.children)walk(c,fn);
 }
 walk(root,'<module>');tree.delete();
 // Join consecutive comment nodes and adjacent string literals using parsed boundaries.
 let merged=[];
 for(const t of texts){let last=merged[merged.length-1];if(last&&last[2]===t[2]&&/^\s*$/.test(text.slice(last[5],t[4]))&& (t[2]==='string'||t[0]<=last[1]+1)){last[1]=t[1];last[3]+=(t[2]==='comment'?'\n':'')+t[3];last[5]=t[5];}else merged.push(t.slice());}
 return {texts:merged,numbers,arrays,tokens,errors,functions,constants:Object.fromEntries(constants),arrayLimitations};
}
(async()=>{
 await ts.Parser.init({locateFile:n=>path.join(out,'tools/tree-sitter',n)});
 let parsers=new Map();for(const lang of ['rust','python','c','cpp','scala','typescript','javascript','bash','c_sharp','java']){let p=new ts.Parser();p.setLanguage(await ts.Language.load(path.join(out,'tools/tree-sitter/tree-sitter-'+lang+'.wasm')));parsers.set(lang,p);}
 let records=fs.readFileSync(path.join(out,'inventory.jsonl'),'utf8').trim().split('\n').map(JSON.parse);if(process.argv[3])records=records.filter(r=>r.language===process.argv[3]);let fd=fs.openSync(path.join(out,process.argv[3]?'features-'+process.argv[3]+'.jsonl':'features.jsonl'),'w');let i=0;
 for(const r of records){let text=fs.readFileSync(r.source,'utf8'),f;
  if(r.language==='patch'){
   f={texts:[],numbers:[],arrays:[],tokens:[],errors:[],functions:[],constants:{},patch_fragments:[]};let lines=text.split('\n');
   for(const side of ['old','new']){let target='',chunk=[],map=[];
    function flush(){if(!chunk.length)return;let lang=target.endsWith('.py')?'python':'rust';let z=features(parsers.get(lang),chunk.join('\n'),lang);for(const key of ['texts','numbers','arrays','errors'])for(let x of z[key]){x[0]=map[x[0]-1];if(['texts','arrays','errors'].includes(key))x[1]=map[x[1]-1];if(key==='arrays')x[4]=x[4].map(n=>map[n-1]);if(key==='numbers')x[2]+=':'+side;f[key].push(x);}f.patch_fragments.push({target,side,lines:map.length,parse_errors:z.errors.length});chunk=[];map=[];}
    for(let j=0;j<lines.length;j++){let l=lines[j];if(l.startsWith('+++ ')){flush();target=l.slice(4);}else if(l.startsWith('@@')||l.startsWith('diff --git'))flush();else if(l.startsWith(' ')||(side==='new'&&l.startsWith('+')&&!l.startsWith('+++'))||(side==='old'&&l.startsWith('-')&&!l.startsWith('---'))){chunk.push(l.slice(1));map.push(j+1);}}flush();
   }f.status='patch_old_and_new_fragments';
  }else if(r.language==='toml'){f={status:'lexical_pending',texts:[],numbers:[],arrays:[],tokens:[],errors:[],functions:[],constants:{}};}
  else {try{f=features(parsers.get(r.language),text,r.language);f.status=f.errors.length?'partial_parse':'ok';}catch(e){f={status:'failed',error:String(e),texts:[],numbers:[],arrays:[],tokens:[],errors:[],functions:[],constants:{}};}}
  fs.writeSync(fd,JSON.stringify({id:r.id,...f})+'\n');if(++i%100===0)console.log('parsed',i,'/',records.length,r.repo);
 }
 fs.closeSync(fd);
})().catch(e=>{console.error(e);process.exit(1)});
