import argparse,collections,csv,hashlib,heapq,json,pathlib,re,sys,math

def main():
 p=argparse.ArgumentParser();p.add_argument('--out',type=pathlib.Path,required=True);a=p.parse_args();o=a.out.resolve();S=json.loads((o/'summary.json').read_text());I=json.loads((o/'inventory-summary.json').read_text());R=[json.loads(x) for x in (o/'inventory.jsonl').open()];tables={}
 for line in (o/'m3_arrays.jsonl').open():
  x=json.loads(line);tables[x['table_id']]=x
 def rpath(rid):
  r=R[int(rid)];return next((x['path'] for x in r['locations'] if x['scope']=='baseline'),r['locations'][0]['path'])
 def baseline(rid):return any(x['scope']=='baseline' for x in R[int(rid)]['locations'])
 def canonical(loc):return re.sub(r'@.*?:','@:',re.sub(r' \[blob=.*?\]','',loc),count=1)
 tops={};aggregates={};all_counts={}
 for m in ['M1','M2','M3','M4']:
  heap=[];count=0;accepted=0;flagged=0;byrepo=collections.Counter();seen=set();maxn=60
  with (o/(m.lower()+'_hits.tsv')).open() as f:
   reader=csv.DictReader(f,delimiter='\t');header=reader.fieldnames
   for row in reader:
    count+=1
    if row.get('exclusion_reason'):continue
    accepted+=1;note=row['false_positive_note'];flagged+=bool(note)
    if m=='M3':mi=tables[int(row['minase_table'])]['record'];ci=tables[int(row['corpus_table'])]['record']
    else:mi=int(row['minase_record']);ci=int(row['corpus_record'])
    byrepo[R[ci]['repo']]+=1
    path=rpath(mi);isp=baseline(mi);production=path.startswith('src/') and '/test' not in path
    if m=='M1':score=(not bool(note),row['match_kind'] in ['en5','ja12'],isp,production,-int(row['corpus_occurrences']),len(row['matched_text']));content=row['matched_text']
    elif m=='M2':score=(not bool(note),isp,production,int(row['common_count']));content=row['common_values']
    elif m=='M3':score=('HaChu' in R[ci]['repo'] and 'handcrafted.rs' in path and row['scale']=='5/2',not bool(note),isp,production,int(row['length']));content=row['match_type']+':'+row['scale']+':'+row['minase_values_preview_first16']
    else:score=(not bool(note),isp,production,int(row['length_tokens']));content=row['length_tokens']
    key=(isp,canonical(row['minase_location']),canonical(row['corpus_location']),content)
    if key in seen:continue
    if len(heap)<maxn or score>heap[0][0]:
     seen.add(key);heapq.heappush(heap,(score,count,row))
     if len(heap)>maxn:heapq.heappop(heap)
  tops[m]=[x[2] for x in sorted(heap,reverse=True)];all_counts[m]={'rows':count,'candidates':accepted,'flagged_false_positive_rows':flagged};aggregates[m]=byrepo
  with (o/(m.lower()+'_top.tsv')).open('w') as f:w=csv.DictWriter(f,fieldnames=header,delimiter='\t');w.writeheader();w.writerows(tops[m])
  print(m,all_counts[m],flush=True)
 (o/'output-counts.json').write_text(json.dumps(all_counts,ensure_ascii=False,indent=2)+'\n')
 with (o/'candidate-counts-by-repo.tsv').open('w') as f:
  w=csv.writer(f,delimiter='\t');w.writerow(['method','repo','candidates']);w.writerows((m,k,v) for m,d in aggregates.items() for k,v in d.most_common())
 corpus={}
 for r in R:
  if r['side']!='corpus':continue
  corpus[(r['repo'],r['license'],r['category'])]=True
 with (o/'attribution.tsv').open('w') as f:
  w=csv.writer(f,delimiter='\t');w.writerow(['repo','manifest_license','category','attribution_review','evidence_scope'])
  for repo,lic,cat in corpus:
   note={'permissive':'取り込みを認定した場合、著作権表示とライセンス本文の保持条件を原文で確認する。Apache-2.0ではNOTICEと変更表示も確認対象。','public-domain':'manifestはpublic-domainと記録。ライセンスによる帰属表示条件は記録されていない。由来はHaChuとして保持。','none':'許諾を確認できない。帰属表示だけで利用条件が満たされるとは判断しない。','copyleft':'取り込みを認定した場合、参照版の著作権表示とライセンス条件を確認する。','mixed':'出典ごとのライセンスに分けて確認する。'}[cat]
   w.writerow([repo,lic,cat,note,'manifest.tsv; rshogi-csa exception applied; historical per-file license not independently adjudicated'])
 def esc(x):return str(x).replace('|','\\|').replace('\n',' ')
 lines=['# M1〜M4の機械的照合結果','',f"基準コミット `a05478a` と全 ref の805コミットを調べ、minase の{I['minase_blobs']:,}個のコード系 blob を解析した。",f"manifest の{I['manifest_rows']}行を処理し、比較用入力を{I['records']:,}件保存した。",'履歴版と出現箇所を別件として数えるため、以下の件数は独立した複製の数ではない。','', '## 実行結果','', '| 手法 | 候補件数 | 除外または単独一致 | 状態 |','|---|---:|---|---|']
 lines += [f"| M1 | {S['M1']['candidates']:,} | 除外 {sum(S['M1']['excluded'].values()):,} | 取得できた入力の注釈と文字列を照合した。 |",f"| M2 | {S['M2']['candidates']:,} | 単独一致 {S['M2']['single_matches']:,} | 数値集合と係数導入時の値を照合した。 |",f"| M3 | {S['M3']['candidates']:,} | 反復表を注記した。 | 配列照合とHaChuの較正を実行した。 |",f"| M4 | {S['M4']['candidates']:,} | 単純構造を注記した。 | tree-sitter補助検査を実行。指定ツールの検証は未完了。 |",'',f"M1 の除外内訳は {json.dumps(S['M1']['excluded'],ensure_ascii=False)} である。",f"M2 の数値正規化失敗は{S['M2']['numeric_failures']}件、係数マクロの記録は{S['M2']['parameter_rows']}件、初出係数は{S['M2']['introduced_coefficients']}件である。",f"M3 の内訳は {json.dumps(S['M3']['types'],ensure_ascii=False)} である。",'M2とM3では候補の意味による除外を行わず、自明値の除外または明らかな偽陽性の注記を行った。','', '## 較正と被覆','', 'M1は英語5語、日本語12文字を最小単位とし、短い希少句、識別子、式を別枠に残した。','M2は自明値を除いた共通値が2個以上の関数または係数接頭辞の組を候補とした。','M3は8要素以上を比較し、正の倍率では各要素の絶対誤差1以内を許容した。','HaChu `hachu.c:194` の `chuPieces[]` と `src/eval/handcrafted.rs:19` の表は、倍率2.5で29要素中27要素が一致した。','表の並べ替えと王駒2要素の変更があるため、単純な全配列一致に加え、倍率付きの多重集合部分一致で再検出した。','',f"M4は10トークンのハッシュを使い、較正で{S['M4']['threshold']}トークンを採用した。",'改名と独立した宣言の並べ替えを行った人工複製の検出率と、独立対照3組の一致率を次に示す。','', '| 最小トークン長 | 人工複製検出 | 人工例の被覆率 | 独立対照の一致 |','|---:|---:|---:|---:|']
 for t in [20,40,80]:
  rows=[x for x in S['M4']['calibration'] if x[1]==t];cl=next(x for x in rows if x[0]=='renamed_reordered_clone');ne=[x for x in rows if x[0].startswith('independent:')];lines.append(f'| {t} | {cl[2]}/1 | {100*cl[4]:.1f}% | {sum(x[2] for x in ne)}/{len(ne)} |')
 c=S['coverage'];lines+=['',f"minase の Rust は基準版{c['baseline_rust_success']}/{c['baseline_rust_files']}ファイル、履歴{c['minase_rust_success']}/{c['minase_rust_blobs']} blob で構文解析に成功した。",f"コーパスでは{c['parse_failures_by_side'].get('corpus',0):,}件の入力に構文エラーがあり、回復した構文木からの抽出を partial_parse と明示した。",'パッチは旧側と新側の各ハンクを解析し、断片ゆえの構文エラーを別に残した。',f"UTF-8として読めない入力は{S['encoding_failures']}件あり、元のバイト列を保持したまま `encoding-errors.tsv` と被覆表に抽出上の制限を記録した。",'完全な構文木を得られない入力について、候補がないことから非複製とは判断できない。','独立対照には閾値未満の短い関数も含まれるため、この3組の結果から一般の偽陽性率を推定しない。','', '## 上位候補','', '候補は基準版を優先し、同一内容と位置の履歴重複を抑えて表示した。','各手法の上位60行は `m1_top.tsv` から `m4_top.tsv` に保存し、全候補は指定の `m1_hits.tsv` から `m4_hits.tsv` に残した。','ライセンス欄は manifest を基本とし、ファイル冒頭のSPDX表示で補正した。各参照版における法的評価は行っていない。','']
 for m in tops:
  lines += [f'### {m}の候補','', '| minase側位置 | コーパス側位置 | ライセンス | 一致内容 | 注記 |','|---|---|---|---|---|']
  for row in tops[m][:10]:
   if m=='M1':detail=row['matched_text']+'; '+row['match_kind']+'; 出現'+row['corpus_occurrences']+'回'
   elif m=='M2':detail=row['minase_function']+' ↔ '+row['corpus_function']+'; '+row['common_values']
   elif m=='M3':detail=row['match_type']+'; '+row['length']+'要素; 倍率'+row['scale']
   else:detail=row['length_tokens']+'トークン; minase被覆率'+str(round(100*float(row['minase_file_coverage']),2))+'%'
   lines.append('| '+' | '.join(esc(x) for x in [row['minase_location'],row['corpus_location'],row['license'],detail,row['false_positive_note'] or '未判定'])+' |')
  lines.append('')
 lines += ['## 未完了部分と限界','',f"YaneuraOu の `v8.00` と `v9.10` は存在せず、要求どおりのタグによる照合は実行できなかった。",'存在する `v8.00-fukauraou` と `v9.10-fukauraou` は補足版として明示して解析した。',f"fishtest の指定コミットにはローカルにない blob が{I['missing_blobs']}個あり、ネットワーク取得もできなかった。",'該当パスは `missing-inputs.tsv` と `coverage.tsv` に記録した。','これらの欠落により、指定入力の全件についてM1〜M4を完遂したとはいえない。','', 'JPlag 6.2.0とDolosは取得できず、指定ツールによるM4は未完了である。','tree-sitterによる代替の解析と照合は実行したが、JPlagの構文解析失敗を確認したという条件は満たしていない。','C++との言語をまたぐ比較は、翻訳例で検出力を確認していないため実施していない。','', '帰属表示の確認対象は `attribution.tsv` に記録した。','候補の複製／偶然の判定、コーパス外の実装の検査、重みファイルの来歴調査はこの仕様のM1〜M4では行っていない。',f"M3では反復配列の長さを解決できなかった箇所が{S['M3']['unresolved_repeat_lengths']:,}件あり、式と位置を `m3_array_limitations.tsv` に記録した。",'部分構文木、マクロ内の式、動的に生成される配列、単位の明記がない数値は検出力の制限になる。','M3の追加検査は比を小数第2位に丸めて候補化するため、任意の倍率による部分的な並べ替えすべてを保証しない。','', '## 再実行と検証','', '各手法のコマンド、版、正規化、閾値、出力の説明は `README.md` に記録した。','`logs/inventory.log`、`logs/extract.log`、`logs/m1.log`、`logs/m2.log`、`logs/m3.log`、`logs/m4.log` は実行記録である。','仕様に基づく単体検証は `logs/tests.log`、構文解析の記録は `parse-errors.tsv` にある。','成果物の行数は `output-counts.json`、ファイルの整合性確認は `verification.json` に保存する。','']
 (o/'report.md').write_text('\n'.join(lines))
if __name__=='__main__':main()
