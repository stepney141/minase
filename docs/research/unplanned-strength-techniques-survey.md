# minaseに未導入の棋力向上手法の調査

## 結論

チェスと将棋のエンジンで効果が測定され、minaseの`src/`にも`docs/plans/`の設計書にもまだ現れない手法は、本書の照合で50件残った。
最も有望なのは、既存の機構を小さく変えるだけで試せる手法である。
具体的には、aspiration windowsがfail-highした後の減深、静的交換評価（SEE、static exchange evaluation）が負の捕獲手を静かな手の後へ回す段、捕獲履歴、静止探索の手数制限、history表の持ち越し、および静的評価の補正表の鍵の追加がこれに当たる。
証拠の量が最大の特異延長は、Stockfishの除去試験で60〜76 Eloの寄与を示す一方、発動が深さに依存し、YaneuraOuは将棋向けに余裕値を作り直した。
minaseの採否判定は検出すべき差を10 Eloに置くので、成熟したStockfishで+1〜3 Eloの改良は検出できず、若いエンジンで+5 Elo以上を記録した手法を先に試すのが合理的である。
照合の副産物として、minaseのnull move pruningは静的評価がβ以上であることもPVノード（最善応手列上の節点）でないことも条件にせず、置換表の打ち切りもPVノードで行うことが分かった。
どちらもチェスエンジンとHaChuでは標準の前提であり、安価に試せる。
一方で、中将棋に固有の証拠はほぼ存在せず、どの候補も発動率の診断とminase自身の自己対局検定を経ずには採否を言えない。
以下では、優先候補、照合の方法、領域別の一覧、証拠の限界、および土台と測定順序への含意の順に述べる。

## 優先候補は既存機構の小改良に集まる

優先順位は、複数エンジンでの実測、中将棋への移しやすさ、およびminaseの判定基準で検出できる効果量の3点で決めた。
minaseの採否判定には一般化逐次確率比検定（GSPRT）を使う。
帰無仮説H0を0 Elo、対立仮説H1を10 Eloに置くので、真の差が5 Eloなら判定まで約4,200ペアを要する（[測定の手引き](../guides/sprt.md)）。
このため、Stockfishの成熟した木で+1〜2 Eloの改良よりも、Reckless、Alexandria、Obsidian、Caissaのような若いエンジンで+5 Elo以上を示した改良を上位に置いた。
加えて、minaseの到達深さは標準の時間制御でSTCが4〜6、LTCが6〜7と浅く（[棋力向上段階4](../plans/strength-stage4.md)）、深さで発動が決まる手法は既に何度も発動率の不足で見送られている。
深さに依存しない順序付けと再探索の改良を上位に、深さ依存の延長を中位に置いたのはこの理由による。

本書では、各エンジンの短時間条件をSTC、長時間条件をLTCと呼ぶ。
それぞれshort time controlとlong time controlの略である。
StockfishのSTCとLTCは10+0.1秒と60+0.6秒、RecklessとAlexandriaは8+0.08秒と40+0.4秒、minaseは10+0.1秒と60+0.2秒である。
Stockfishの値に付けた「≈」は、コミット文の勝敗数から算出したロジスティックEloを表し、fishtestが表示する正規化Eloではない。
本書に引いたStockfishのコミットは、2026年9月19日に取得した複製（コミット`17a6c8f`）で件名と勝敗数を照合し、Eloを再計算した。
その他のエンジンの「±」は、OpenBenchがコミット文に記す95%信頼区間である。

| 順位 | 手法 | 選んだ根拠 | 中将棋で先に確かめること |
|---|---|---|---|
| 1 | aspiration windowsのfail-high時の減深 | Stockfishの導入はSTCで≈+10.72、LTCで≈+6.54 Elo（[081af908](https://github.com/official-stockfish/Stockfish/commit/081af908)）。Etherealも12+0.12秒で+10.62±6.46、60+0.6秒で+4.76±3.57 Eloを得た（[710f64e](https://github.com/AndyGrant/Ethereal/commit/710f64e)）。minaseの窓の制御へ数行を足すだけで済む。 | 根の探索がfail-highを起こす頻度。 |
| 2 | 静的交換評価が負の捕獲手を静かな手の後へ回す段 | YaneuraOuは、負の捕獲手を静かな手より前に置くと自己対局のレーティング差がR−25.46とR−30.95になると測った（[movepick.cpp](https://github.com/yaneurao/YaneuraOu/blob/master/source/movepick.cpp)）。将棋での直接の証拠を持つ数少ない候補である。 | 獅子の2枚取りなど判定不能の捕獲手をどちらの段に置くか。 |
| 3 | 捕獲履歴 | Stockfishの導入はSTCで≈+5.25、LTCで≈+2.73 Elo（[4bc11984](https://github.com/official-stockfish/Stockfish/commit/4bc11984)）。Alexandria、Berserk、Etherealなど多くのエンジンが持ち、捕獲履歴による戦術手の減深など後続の手法の前提にもなる。 | 2枚取りと成りを表の鍵へ写す方法。 |
| 4 | 静止探索の手数制限 | Stockfishの導入はSTCで≈+0.88、LTCで≈+3.67 Elo（[d5f86b63](https://github.com/official-stockfish/Stockfish/commit/d5f86b63)）とLTCで効き、YaneuraOuも将棋で同じ制限を使う。minaseでは深さ5の探索で静止探索と深さ0の節点が全ノードの約98%を占めた記録があり（[静止探索の教訓](../lessons/qsearch-dag-without-tt.md)）、静止探索の1ノードあたりの手数を削る効果はチェスより大きく出る可能性がある。 | 最後の王駒の捕獲と獅子の2枚取りを除外したときの発動率。 |
| 5 | history表と補正表の持ち越し | Stockfishは新しい根で表を3/4倍して持ち越し、STCで≈+1.29、LTCで≈+2.00 Eloを得た（[ced9f698](https://github.com/official-stockfish/Stockfish/commit/ced9f698)）。minaseがcounter move historyとcontinuation historyを見送った理由、すなわち1回の探索で表が埋まらないことに直接効く。 | 持ち越し後の非ゼロ参照率。 |
| 6 | 補正表の鍵の追加 | 色別の非歩兵の鍵はAlexandriaで+6.98と+12.28 Elo、Obsidianで+9.96 Eloを示した（[Alexandria 1f26efc](https://github.com/PGG106/Alexandria/commit/1f26efc)、[Obsidian fe0637c](https://github.com/gab8192/Obsidian/commit/fe0637c)）。minaseの鍵は駒種別の枚数であり、Stockfishが中立として外した材料の鍵に当たる。 | 色別の駒配置の鍵の再利用率と予測改善率。 |
| 7 | 特異延長と複数手打ち切り | Stockfishの除去試験は、2018年のSTC 60,000局で約60 Elo（[e408fd7b](https://github.com/official-stockfish/Stockfish/commit/e408fd7b)）、Stockfish 17の注記で約76 Eloとした（[Stockfish 17のsearch.cpp](https://github.com/official-stockfish/Stockfish/blob/sf_17/src/search.cpp)）。複数手打ち切りの導入はSTCで≈+2.48、LTCで≈+3.20 Elo（[f69106f7](https://github.com/official-stockfish/Stockfish/commit/f69106f7)）。 | LTCの到達深さ6〜7での発動率。 |
| 8 | null moveと置換表の打ち切りの前提条件 | 静的評価がβ以上でPVノードでないことをnull moveの条件にする形は、StockfishとHaChuの双方が持つ（[Stockfishのsearch.cpp](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/search.cpp)、[hachu.c](https://salsa.debian.org/debian/hachu)）。分離した測定は見つからなかった。 | 条件を加えた後のnull moveの発動率と打ち切り率。 |

8件のうち1から6は、深さに依存せず発動するか、既存の機構の使い方を変えるだけの手法である。
4はStockfishのSTCでは小さいが、minaseの木の構成が静止探索に偏っているので上位に残した。
7は証拠が最も厚いが、STCの到達深さでは発動しにくく、YaneuraOuは将棋でほぼ全ての手が特異と判定されるため係数を作り直した（[yaneuraou-search.cpp](https://github.com/yaneurao/YaneuraOu/blob/master/source/engine/yaneuraou-engine/yaneuraou-search.cpp)）。
8は単独の測定値を欠くが、minaseの現行条件が標準から外れている点が照合で判明したので、診断の対象として挙げた。
次節では、これらの候補をどの基準で既存の計画と照合したかを述べる。

## 照合では計画済みと棄却済みの手法を除いた

本書は、独立に行った2つの調査を統合したものである。
一方はチェス、将棋、および大盤変種のエンジンを領域別に調べた調査であり、もう一方はチェスエンジンの探索と順序付けに絞って、minaseで試すための前提条件を詳しく検討した調査である。
後者が挙げた10件のうち8件は前者の候補と重なり、残る2件（脅威を避ける手の順序補正と、捕獲履歴による戦術手の減深）を本書の一覧に加えた。
重なった候補については、後者が示した測定値と、minaseの着手表現や探索実装に照らした前提条件を該当する行へ統合した。

除外の基準には、`src/`、`docs/plans/`、および未統合ブランチの設計書を2026年9月23日時点で調べた在庫の記録を用いた。
照合の基準コミットは`cb4de91`である。
`src/`に実装済みの手法、設計書が採用、`H0`で棄却、見送り、辞退、または予定した手法は、一覧から除いた。
除外した主な手法は、王手延長、ProbCut、mate distance pruning、置換表のクラスタ化、静的評価の置換表保存、counter move history、continuation history、history pruning、および獅子の2段階の非捕獲移動の減深であり、いずれも段階4から段階8で見送られた（[棋力向上の段階計画](../plans/strength-stages.md)）。
reverse futility pruning、late move pruning、razoring、improvingフラグ、駒種と到達升のhistory、およびmalusは、GSPRTの`H0`またはSTCの振分けで不採用となった。
詰み専用探索と定跡は探索部の設計書が辞退し、補助ワーカーの投票は、複数スレッドが置換表だけを共有して同じ根を探索するLazy SMPの[設計書](../plans/lazy-smp.md)が辞退した。
先読み中の目標時間の25%増しは[USI先読みの設計書](../plans/ponder.md)が本方式には当てはまらないと判断し、相手番局面の先読みは同書が見送った。
秒単位の切り上げは[時間効率の設計書](../plans/time-management-efficiency.md)が不要と判断し、Stockfishの動的な時間係数は同書で束ごと`H0`となった。
Texel tuningは探索部の設計書が評価関数の次期マイルストーンへ送り、その後に導入した学習済みの駒升表（PST、piece-square table）が同じ役割を果たした。
反復の結果と置換表の干渉は[反復負け回避の設計書](../plans/search-repetition.md)が扱い、NNUE（差分更新型のニューラルネット評価関数）の第1層拡幅、出力バケット、および相対王駒特徴は段階10の予定に入っている。
調整と測定の領域では候補が残らなかった。
同時摂動確率近似（SPSA）による係数調整とGSPRTの段階ゲートは実装済みであり、LTCでの調整は[SPSAの設計書](../plans/spsa.md)が最初のセッションの採否を見てから判断すると定めているからである。

改良形の扱いには判断を要した。
実装済みまたは棄却済みの手法の改良は、改良の内容そのものがどの設計書にも現れない場合に限って残し、一覧の最終列に基礎の手法と相違点を書いた。
捕獲履歴とhistory表の持ち越しは、[棋力向上段階5](../plans/strength-stage5.md)が「本書の対象外」と明記したものの、採否も見送りも判断していない。
補正表の対局内の持ち越しも、[棋力向上段階8](../plans/strength-stage8.md)が「別の主張なので扱わない」と記しただけである。
本書はこれらを未計画として残し、該当する行にその旨を記した。
設計書がなく`docs/research/`の調査メモだけに現れる手法は、「調査メモで言及済み、計画なし」として残し、ファイルを示した。
次節は、残った50件を領域ごとに示す。

## 候補は8つの領域に分かれる

候補は、探索の選択性、手の順序、置換表、評価、時間管理、並列探索、実行環境、および将棋系に固有の手法の8領域に分かれ、時間管理から実行環境までの3領域は1つの表にまとめた。
一覧の各表は同じ列を持つ。
測定値の列は時間制御と出典を添え、最終列は中将棋への適用上の論点と、既存手法との関係を示す。
中将棋への適用の記述は、特記のない限り調査者の推論であり、中将棋での測定ではない。
英語名に現れるTTはtransposition table、すなわち置換表の略である。
また、β打ち切りが予想される節点をcut node、全手がα以下と予想される節点をall nodeと呼ぶ。

### 探索の選択性では延長と減深の細部が残る

探索の選択性の未導入手法は、延長の系統と、LMR（late move reductions、後方の手の減深）の細部の2群に分かれる。
minaseのLMRは対数表とhistoryによる±1の調整だけを持ち、ノードの種別や置換表の情報を減深に使わない。
このため、チェスエンジンが2018年以降に積み上げた細部の大半が未導入のまま残る。

| 手法 | 仕組み | 採用エンジン | 測定値 | 中将棋への適用と照合 |
|---|---|---|---|---|
| 特異延長と多段、負の延長（singular extension） | 置換表の手を除いた浅い探索が置換表値から余裕を引いた値に届かなければ、置換表の手を延長する。差が大きければ2〜3手延長し、他の手も届けば逆に減深する。 | Stockfish、Reckless、Berserk、Obsidian、Weiss、YaneuraOu | Stockfishの除去試験は2018年にSTC 60,000局で約60 Elo（[e408fd7b](https://github.com/official-stockfish/Stockfish/commit/e408fd7b)）、Stockfish 17の注記で約76 Elo（[Stockfish 17](https://github.com/official-stockfish/Stockfish/blob/sf_17/src/search.cpp)）。3段延長はSTCで≈+0.94、LTCで≈+2.02 Elo（[f2b6b5cf](https://github.com/official-stockfish/Stockfish/commit/f2b6b5cf)）。Obsidianの2段延長は8+0.08秒で+14.80±7.46 Elo（[25a7db3](https://github.com/gab8192/Obsidian/commit/25a7db3)）。Weissのcut nodeでの負の延長は10+0.1秒で+3.25±2.36、60+0.6秒で+3.55±2.42 Elo（[60c5137](https://github.com/TerjeKir/weiss/commit/60c5137)）。 | 置換表の記録手、保存値の下限、および保存深さを前提とし、Stockfishは深さ6以上で発動させる。除外探索を置換表への保存と反復検出と矛盾なく行う必要がある。YaneuraOuは将棋向けに約10%の手が該当するよう余裕値を調整した。LTCで効果が大きい系統なので、STCだけで棄却しない。 |
| 複数手打ち切り（multi-cut） | 特異延長の除外探索が元のβも超えた場合、少なくとも2手がβを超えるとみなしてノードを打ち切る。 | Stockfish、Reckless、Ethereal、Berserk、Weiss、Caissa | Stockfishの導入はSTCで≈+2.48、LTCで≈+3.20 Elo（[f69106f7](https://github.com/official-stockfish/Stockfish/commit/f69106f7)）。Etherealは10+0.1秒で+5.74±4.14、60+0.6秒で+3.63±2.80 Elo（[d397a7b](https://github.com/AndyGrant/Ethereal/commit/d397a7b)）。Recklessの返り値の調整は8+0.08秒で+4.29±2.62、40+0.4秒で+4.54±3.23 Elo（[0d9ac30](https://github.com/codedeliveryservice/Reckless/commit/0d9ac30)）。 | 特異延長の除外探索を前提とする。置換表の手の合法性と、特殊移動後の規則状態が局面の鍵に含まれることを保つ。 |
| 再捕獲延長（recapture extension） | 直前に駒を取られた升で取り返す手を1手延長する。 | Stockfish（現行版では削除）、Reckless | StockfishはPVノードで捕獲履歴の条件も課し、STCで≈+1.63、LTCで≈+1.24 Elo（[863a1f2b](https://github.com/official-stockfish/Stockfish/commit/863a1f2b)）。2026年9月の版には独立した条件が見当たらない（[確認版のsearch.cpp](https://github.com/official-stockfish/Stockfish/blob/0a215d6c9e48856ef630013b8ab8312941a59057/src/search.cpp)）。Recklessは8+0.08秒で+6.41±3.34、40+0.4秒で+6.08±3.61 Elo（[8d37736](https://github.com/codedeliveryservice/Reckless/commit/8d37736)）。 | 獅子は1手で経由升と到達升の2升で取れるので、直前手の到達升だけを比べず、実際の捕獲升を定義する必要がある（[着手表現](../../src/core/mv.rs)、[捕獲升の取得](../../src/core/position.rs)）。見送り済みの王手延長と違い、王駒への利きの判定を要しない。 |
| PV末端での置換表の手の延長（TT-move extension at PV leaves） | PVノードで子が静止探索へ落ちる場合でも、置換表の手の保存深さが2以上なら深さ1で読む。 | Stockfish、Reckless | STCで≈+1.62、LTCで≈+1.34 Elo（[836154ac](https://github.com/official-stockfish/Stockfish/commit/836154ac)）。 | 規則上の障害はない。現行の`negamax`は深さ0で直ちに静止探索へ移るので、ノード種別と記録手を渡して延長の位置を定める必要がある（[探索実装](../../src/search/alphabeta/negamax.rs)）。成りと獅子の2段階移動を同じ扱いにするかを決める。 |
| ノード種別によるLMRの調整（cut/all-node LMR） | β打ち切りが予想されるcut nodeで減深を増やし、全手がα以下と予想されるall nodeにも係数を掛ける。 | Stockfish、Reckless | Stockfish 17の注記でcut nodeの項は約4 Eloとされる。all nodeの係数はSTCで≈+1.16、LTCで≈+1.97 Elo（[1780c1fd](https://github.com/official-stockfish/Stockfish/commit/1780c1fd)）。Recklessは8+0.08秒で+2.31±1.76 Elo（[7a0a812](https://github.com/codedeliveryservice/Reckless/commit/7a0a812)）。 | 実装済みLMRの改良である。現行の`negamax`はノード種別を引数に持たない。 |
| 子ノードの打ち切り回数による減深（cutoffCnt） | 1手先のノードでβ打ち切りが多く起きていれば、残りの手を深く減深する。 | Stockfish、Reckless、Caissa | StockfishはSTCで≈+1.14、LTCで≈+1.69 Elo（[a32d2086](https://github.com/official-stockfish/Stockfish/commit/a32d2086)）。Recklessは8+0.08秒で+6.00±4.14 Elo（[0f25f89](https://github.com/codedeliveryservice/Reckless/commit/0f25f89)）。 | 実装済みLMRの改良である。plyごとの計数を持つだけで済む。 |
| 再探索の深さ調整（doDeeper/doShallower） | 減深した探索がαを大きく超えれば1手深く、わずかに超えただけなら1手浅く読み直す。 | Stockfish、Ethereal、Alexandria、Caissa、Reckless | StockfishはSTCで≈+0.90、LTCで≈+1.85 Elo（[65e21505](https://github.com/official-stockfish/Stockfish/commit/65e21505)）。Etherealは10+0.1秒で+2.66±2.54、60+0.6秒で+2.31±2.39 Elo（[15f1e12](https://github.com/AndyGrant/Ethereal/commit/15f1e12)）。Caissaは10+0.1秒で+6.93±3.88、40+0.4秒で+5.14±3.35 Elo（[434bbda](https://github.com/Witek902/Caissa/commit/434bbda)）。 | 実装済みLMRの改良である。現行は減深探索がαを超えると通常の深さで読み直す。 |
| 事後の深さ調整（hindsight reductions） | 親が大きく減深した子ノードで、相手の局面が悪化していなければ1手延ばし、両者の静的評価の和が大きければ1手縮める。 | Stockfish、Alexandria、Reckless、Berserk、Viridithas | StockfishはSTCで≈+2.18、LTCで≈+1.17 Elo（[a944f082](https://github.com/official-stockfish/Stockfish/commit/a944f082)）。Alexandriaは8+0.08秒で+1.93±1.54、40+0.4秒で+1.36±1.10 Elo（[915558d](https://github.com/PGG106/Alexandria/commit/915558d)）。 | 棄却済みのimprovingフラグは枝刈りの余裕値を変えたが、本手法はノードの深さを変える点が異なる。親の減深量と静的評価をplyごとに保持する必要がある。 |
| 補正量による減深の調整（corrplexity in LMR） | 静的評価の補正量の絶対値が大きい局面は評価が不確かとみなし、減深を減らす。 | Stockfish、Reckless、Alexandria、Obsidian、PlentyChess | Recklessは8+0.08秒で+9.17±5.31 Elo（[3edc87b](https://github.com/codedeliveryservice/Reckless/commit/3edc87b)）。Alexandriaは8+0.08秒で+2.14±1.67、40+0.4秒で+2.63±1.92 Elo（[7de97e7](https://github.com/PGG106/Alexandria/commit/7de97e7)）。Obsidianは10+0.1秒で+2.51±1.94 Elo（[e144325](https://github.com/gab8192/Obsidian/commit/e144325)）。 | 実装済みcorrection historyの新しい用途である。現行の補正値はfutilityとSEE枝刈りの境界にだけ使われる。 |
| 捕獲履歴による戦術手の減深（capture-history LMR for tactical moves） | 捕獲履歴が負の戦術手にも減深を掛け、見込みの薄い捕獲手を浅く読む。 | Ethereal | 捕獲手の減深を1から2へ増やし、10+0.1秒で+7.22±4.72、60+0.6秒で+2.37±1.90 Elo（[dcb8560](https://github.com/AndyGrant/Ethereal/commit/dcb8560)）。 | 捕獲履歴の導入が前提になる。現行のLMRは捕獲手を対象外とするので、減深した捕獲手がαを超えたときの再探索と、最後の王駒の捕獲手を減深しない条件が要る（[探索実装](../../src/search/alphabeta/negamax.rs)）。 |
| αを上げた後の減深（alpha-raise reduction） | PVノードでαを更新した後の残りの手を浅く読む。 | Stockfish、Reckless、Stormphrax、PlentyChess | StockfishはSTCで≈+1.63、LTCで≈+1.37 Elo（[9eb7b607](https://github.com/official-stockfish/Stockfish/commit/9eb7b607)）。Recklessは8+0.08秒で+1.55±1.25、40+0.4秒で+3.15±2.15 Elo（[ec56d74](https://github.com/codedeliveryservice/Reckless/commit/ec56d74)）。 | 規則上の障害はない。 |
| fail-high時の値の混合（fail-high score blending） | β打ち切りの返り値や、静止探索で着手せずに静的評価を採るstand-patの返り値を、βとの加重平均へ寄せる。 | Stockfish、Reckless、Caissa、integral | 主探索はSTCで≈+1.36、LTCで≈+1.37 Elo（[91a4cea4](https://github.com/official-stockfish/Stockfish/commit/91a4cea4)）。Recklessの係数の制限は8+0.08秒で+1.27±1.01、40+0.4秒で+3.00±2.06 Elo（[666cef5](https://github.com/codedeliveryservice/Reckless/commit/666cef5)）。Recklessのstand-patは8+0.08秒で+1.17±0.90、40+0.4秒で+1.71±1.37 Elo（[614b666](https://github.com/codedeliveryservice/Reckless/commit/614b666)）。 | 詰み帯と、反復負け回避の設計書が定める専用帯の値は混合から除く。 |
| aspiration windowsのfail-high時の減深と平均値中心 | 根がfail-highしたとき同じ深さでなく1手浅く読み直す。窓の中心を過去の反復の平均値に置く。 | Stockfish、Ethereal、Reckless | 減深はSTCで≈+10.72、LTCで≈+6.54 Elo（[081af908](https://github.com/official-stockfish/Stockfish/commit/081af908)）。Etherealは12+0.12秒で+10.62±6.46、60+0.6秒で+4.76±3.57 Elo（[710f64e](https://github.com/AndyGrant/Ethereal/commit/710f64e)）。平均値中心はSTCで≈+1.82、LTCで≈+2.23 Elo（[a0259d8a](https://github.com/official-stockfish/Stockfish/commit/a0259d8a)）。 | 段階6で採用したaspiration windowsの改良である。現行はfail-high後に同じ深さで上側を広げる。 |
| 置換表だけによるProbCut（small ProbCut） | 置換表に「β＋余裕」以上の下限値が深さ−4以上で残っていれば、探索せずに返す。 | Stockfish | Stockfish 17の注記で約4 Elo（[Stockfish 17](https://github.com/official-stockfish/Stockfish/blob/sf_17/src/search.cpp)）。 | 見送り済みのProbCutは捕獲手の探索を前提とし、適格ノードが0.4%だった。本手法は捕獲手を読まないので発動条件が異なり、発動率を別に測る必要がある。 |
| 置換表の探索値による評価の置換（TT value as better eval） | 置換表の値の種類が静的評価との大小に合う場合、stand-patや枝刈りの基準値をその値で置き換える。 | Stockfish、Reckless、PlentyChess | Stockfish 16の注記で静止探索約13 Elo、主探索約7 Elo（[Stockfish 16](https://github.com/official-stockfish/Stockfish/blob/sf_16/src/search.cpp)）。 | 見送り済みの静的評価の保存とは別で、保存済みの探索値を使う。現行の静止探索は置換表を打ち切りにだけ使う。 |
| null moveの前提条件（null move preconditions） | 静的評価がβ以上で、PVノードでない場合に限り手番を渡す探索を試す。 | Stockfish、HaChu | 分離した測定は見つからなかった。Stockfishはcut nodeで静的評価がβ付近以上のときだけ試す（[Stockfishのsearch.cpp](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/search.cpp)）。HaChuも`curEval >= beta`を条件にする（[hachu.c](https://salsa.debian.org/debian/hachu)）。 | 実装済みnull move pruningの改良である。現行の条件は深さ、直前のnull move、詰み帯、および非王駒の有無だけである（[探索実装](../../src/search/alphabeta/negamax.rs)）。棄却済みの減深量の評価差依存とは別の変更である。 |
| 静止探索の手数制限（qsearch move-count pruning） | 王手でも再捕獲でもない捕獲手を、各ノードの2〜3手目より後では読まない。 | Stockfish、Reckless、Obsidian、Caissa、YaneuraOu | Stockfishの導入はSTCで≈+0.88、LTCで≈+3.67 Elo（[d5f86b63](https://github.com/official-stockfish/Stockfish/commit/d5f86b63)）。2026年9月の版も、王手を掛けず、直前の捕獲升へ取り返さず、成らない手を3手目以降で省く（[確認版のsearch.cpp](https://github.com/official-stockfish/Stockfish/blob/0a215d6c9e48856ef630013b8ab8312941a59057/src/search.cpp#L1806-L1833)）。Obsidianは10+0.1秒で+1.56±1.24 Elo（[e0c2b3a](https://github.com/gab8192/Obsidian/commit/e0c2b3a)）。Caissaは+1.79±1.31 Elo（[fbebb14](https://github.com/Witek902/Caissa/commit/fbebb14)）。 | 見送り済みの深さ上限は深い節点が0.82%で不発だったが、本手法はノード内の手数を切る点が異なる。現行の静止探索はdelta pruningとSEEによる枝刈りの後に全捕獲手を展開するので、その後段に置く。最後の王駒の捕獲と獅子の2枚取りを除外する条件が要る。 |
| 反復の先読み（upcoming repetition detection） | 可逆な1手で既出局面へ戻れるなら、αを引き分け値まで上げる。 | Stockfish、Alexandria、Berserk、Weiss、Obsidian | StockfishはSTCで≈+2.82、LTCで≈+4.52 Elo（[91a76331](https://github.com/official-stockfish/Stockfish/commit/91a76331)）。YaneuraOuは将棋で強くならないとして使わない。 | 調査メモで言及済み、計画なし（[search-repetition-prior-art.md](search-repetition-prior-art.md)）。RULES.md第31条の反復規則のうちR2とR3では反復が禁じ手、R1では攻撃側の負けなので、引き分け値でαを上げる前提が成り立たない。 |
| reverse futility pruningとrazoringの再設計 | reverse futility pruningをPVフラグの立ったノードで行わず、返り値をβと静的評価の加重平均にする。razoringを予想all nodeに限る。 | Stockfish、Reckless、Stormphrax、PlentyChess | 返り値の混合はSTCで≈+6.49、LTCで≈+1.15 Elo（[08cdbca5](https://github.com/official-stockfish/Stockfish/commit/08cdbca5)）。Recklessは8+0.08秒で+9.71±5.47 Elo（[c5b6133](https://github.com/codedeliveryservice/Reckless/commit/c5b6133)）。razoringのall node限定はSTCで≈+1.35、LTCで≈+2.61 Elo（[9c11e231](https://github.com/official-stockfish/Stockfish/commit/9c11e231)）。 | 両手法は段階4で`H0`となった棄却済みの手法である。段階4の実装は静的評価をそのまま返し、ノード種別を区別しなかったので、改良形は別の主張として再挑戦になる。 |

延長の系統は、Stockfishの注記がLTCほど効くと記す系統であり（[Stockfishのsearch.cpp](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/search.cpp)）、minaseの浅い到達深さでは発動率が先に問題になる。
LMRの細部はどれも1件あたり数Eloだが、ノード種別とPVフラグを持てば複数の細部が同じ土台に乗る。
その土台の一部は置換表と手の順序付けにあるので、次に順序付けの表を扱う。

### 手の順序とhistoryでは捕獲手と表の寿命が空白になっている

手の順序付けの未導入手法は、捕獲手の順序、表の寿命、および表の添字の3点に集中する。
minaseの手選択は置換表の手、MVV-LVA（取られる駒の価値の降順、取る駒の価値の昇順）で並べた全捕獲手、2つのkiller、historyで並べた静かな手の順で進み、history表は根探索ごとに初期化される。
捕獲手の側に学習する表がなく、静かな手の側では表が1回の探索の中でしか育たない点が、チェスエンジンとの最大の差である。

| 手法 | 仕組み | 採用エンジン | 測定値 | 中将棋への適用と照合 |
|---|---|---|---|---|
| 捕獲履歴（capture history） | 動かす駒、到達升、取られる駒種を鍵に捕獲手の成績を学習し、捕獲手の順序と枝刈りの閾値に使う。 | Stockfish、Alexandria、Berserk、Ethereal、Viridithas | 導入はSTCで≈+5.25、LTCで≈+2.73 Elo（[4bc11984](https://github.com/official-stockfish/Stockfish/commit/4bc11984)）。重みの倍増はSTCで≈+1.13、LTCで≈+2.36 Elo（[881cab25](https://github.com/official-stockfish/Stockfish/commit/881cab25)）。 | 段階5が対象外と明記したが採否の判断はない。58駒種×144升×29駒種で約24万要素である。獅子の2枚取りでは鍵に使う取られる駒を決める必要がある。加点と減点の形、および静止探索の捕獲順序にも使うかを併せて決める。 |
| 負の捕獲手を後回しにする段（bad-capture stage） | 静的交換評価が負の捕獲手を、静かな手の後の段へ回す。 | Stockfish、YaneuraOu | YaneuraOuは、負の捕獲手を静かな手より前に置く版が固定ノード30万でR−25.46、1手1秒でR−30.95だったと記す（[movepick.cpp](https://github.com/yaneurao/YaneuraOu/blob/master/source/movepick.cpp)）。 | SEEと段階的な手選択は実装済みで、段の構成だけが新しい。現行は全捕獲手をkillerより前に置く（[探索実装](../../src/search/alphabeta/ordering.rs)）。判定不能のSEEを持つ獅子の捕獲の扱いを決める。 |
| history表と補正表の持ち越し（history aging） | 新しい根で表を初期化せず、一定倍して前の探索の値を残す。 | Stockfish、PlentyChess、Stormphrax | Stockfishの3/4倍はSTCで≈+1.29、LTCで≈+2.00 Elo（[ced9f698](https://github.com/official-stockfish/Stockfish/commit/ced9f698)）。PlentyChessはSTCで+1.12±0.91 Elo（[#451](https://github.com/Yoshie2000/PlentyChess/pull/451)）。Viridithasは除去しても中立だった（[#323](https://github.com/cosmobobak/viridithas/pull/323)）。 | 段階5と段階8が扱わないと明記したが、判断はしていない。counter move historyとcontinuation historyの見送り理由は非ゼロ参照率0.73%と0.62%であり、持ち越しはこの理由に直接効くので、両表の再開条件になり得る。 |
| 浅い手数専用の履歴（low-ply history） | 根とその近くのplyだけに別のhistory表を持ち、深い位置の更新に埋もれない順序を根付近で使う。 | Stockfish、Alexandria、YaneuraOu | Stockfishは2020年にply 0〜3の表として導入し、STCで≈+1.05、LTCで≈+1.73 Eloを得た（[b8c00efa](https://github.com/official-stockfish/Stockfish/commit/b8c00efa)）。根の表はSTCで≈+1.2、LTCで≈+0.9 Elo（[240a5b1c](https://github.com/official-stockfish/Stockfish/commit/240a5b1c)）。根の表を低plyの表へ組み替えた版はSTCで≈+1.02、LTCで≈+2.71 Elo（[7ac745a7](https://github.com/official-stockfish/Stockfish/commit/7ac745a7)）。Alexandriaは8+0.08秒で+6.72±3.46、40+0.4秒で+1.30±1.05 Elo（[24a2484](https://github.com/PGG106/Alexandria/commit/24a2484)）。YaneuraOuは他の変更と一括でR+70と記す（[01d74a8](https://github.com/yaneurao/YaneuraOu/commit/01d74a8)）。 | 規則上の障害はない。144×144の升の組で表を持てば大きさは問題にならない。根からの距離をnull moveの後も一貫して数える必要がある。 |
| 根の手の並べ替え（root move ordering） | 根の手を前回の反復の値や探索量で並べ直す。 | Stockfish、Obsidian、Koivisto | Obsidianは+18.7±16.0 Eloと+10.39±6.25 Eloを記録した（[88d2214](https://github.com/gab8192/Obsidian/commit/88d2214)、[ae1ca23](https://github.com/gab8192/Obsidian/commit/ae1ca23)）。ただしコミット文は仕組みを詳述しない。 | 現行の根は各反復で置換表の手と通常の順序付けで並ぶ（[探索実装](../../src/search/alphabeta/root.rs)）。 |
| 静的評価の変化による履歴（static-eval-difference history） | 直前の手が引き起こした静的評価の変化の符号で、その手のhistoryを更新する。 | Stockfish、YaneuraOu、PlentyChess | 導入はSTCで≈+1.06、LTCで≈+1.39 Elo（[be7a03a9](https://github.com/official-stockfish/Stockfish/commit/be7a03a9)）。非対称化はSTCで≈+7.80、LTCで≈+3.08 Elo（[3cfaef74](https://github.com/official-stockfish/Stockfish/commit/3cfaef74)）。Stockfish 17の注記で約9 Elo。 | 実装済みbutterfly historyの改良である。静的評価の計算費用は処理時間の0.24%と小さい。 |
| 脅威を添字にした履歴（threat-indexed history） | 移動元と移動先が相手の利きの下にあるかをhistoryの添字に加える。 | Berserk、Ethereal、Caissa、PlentyChess、Koivisto | Berserkは10+0.1秒で+8.82±5.94、60+0.6秒で+4.72±3.44 Elo（[6df0179](https://github.com/jhonnold/berserk/commit/6df0179)）。Etherealは10+0.1秒で+2.33±1.86、60+0.6秒で+2.88±1.93 Elo（[341c43f](https://github.com/AndyGrant/Ethereal/commit/341c43f)）。Caissaは+5.9±4.9 Elo（[ecba3d3](https://github.com/Witek902/Caissa/commit/ecba3d3)）。 | 相手の利き集合を毎ノード要する。YaneuraOuは将棋で費用過大として無効化し、minaseの利き数の表は毎秒探索局面数（NPS）が0.816倍に落ちて不採用になった。利き情報の土台ができるまで優先度は低い。 |
| 脅威を避ける手の順序補正（threat-aware move ordering） | 価値の低い駒に攻められている駒を逃がす静かな手を順序で上げ、そうした升へ駒を移す手を下げる。 | Stockfish、YaneuraOu（無効化） | STCで≈+1.47、LTCで≈+1.15 Elo（[65ece7d9](https://github.com/official-stockfish/Stockfish/commit/65ece7d9)）。 | 脅威を添字にした履歴と同じく利き集合を要する。[棋力向上段階9](../plans/strength-stage9.md)の脅威特徴は評価関数用であり、順序付けには使っていない。獅子の2段階移動の移動先と、王駒が2枚あり得る場合の例外を定め、利き情報の計算費用を先に測る。 |
| 補正表の鍵の追加（multi-key correction history） | 駒配置の部分集合や直前の手順を鍵にした補正表を複数持ち、補正値を加重和する。 | Stockfish、Alexandria、Obsidian、Berserk、Caissa、YaneuraOu | 主要駒、小駒、非歩兵の鍵はSTCで≈+4.41、LTCで≈+8.70 Elo（[60351b9d](https://github.com/official-stockfish/Stockfish/commit/60351b9d)）。非歩兵の鍵はAlexandriaで+6.98±3.56と+12.28±4.75 Elo（[1f26efc](https://github.com/PGG106/Alexandria/commit/1f26efc)）。Obsidianでは+9.96±3.88 Elo（[fe0637c](https://github.com/gab8192/Obsidian/commit/fe0637c)）。手順の補正はSTCで≈+0.82、LTCで≈+1.59 Elo（[6592b13d](https://github.com/official-stockfish/Stockfish/commit/6592b13d)）。 | 調査メモで言及済み、計画なし（[forward-pruning-prior-art.md](forward-pruning-prior-art.md)）。段階8は王駒の升、駒種別の枚数、およびその組合せだけを診断した。Stockfishは材料の鍵を導入後に中立として外した（[7f386d10](https://github.com/official-stockfish/Stockfish/commit/7f386d10)）。 |
| history表の重力更新（history gravity） | 加点と減点の双方で`v += b − v·abs(b)/D`と更新し、値を上限の内側へ滑らかに保つ。 | Stockfish、Viridithas、Stormphrax | 原型の導入に数値はない（[7f300a76](https://github.com/official-stockfish/Stockfish/commit/7f300a76)）。Viridithasの集計値による重力は40+0.4秒で+2.84±2.00 Elo（[#370](https://github.com/cosmobobak/viridithas/pull/370)）。 | 実装済みbutterfly historyの改良である。現行は上限超過時に表全体を半減する。malusはSTCの振分けで進まなかったので、再挑戦するなら減点の形も併せて変える別の主張になる。 |
| fail-low時の直前手への加点（prior-move bonus） | ノードがfail-lowしたとき、相手の直前の静かな手を良い手として加点する。 | Stockfish | STCで≈+1.15、LTCで≈+1.38 Elo（[7395d568](https://github.com/official-stockfish/Stockfish/commit/7395d568)）。主historyへの加点はSTCで≈+0.70、LTCで≈+3.33 Elo（[d97a02ea](https://github.com/official-stockfish/Stockfish/commit/d97a02ea)）。 | 自分の失敗手を減点するmalusとは向きも対象も異なる。 |
| 歩兵配置の履歴（pawn history） | 歩兵の配置のハッシュ、動かす駒、到達升を鍵にしたhistoryを加える。 | Stockfish、PlentyChess、Viridithas、YaneuraOu | StockfishはSTCで≈+1.11、LTCで≈+1.95 Elo（[b0658f09](https://github.com/official-stockfish/Stockfish/commit/b0658f09)）。YaneuraOuは移植時に弱くなり既定で無効化した（[99b20a7](https://github.com/yaneurao/YaneuraOu/commit/99b20a7)）。 | 歩兵配置専用の局面の鍵が必要になる。中将棋の歩兵と仲人は前線を作るが、将棋の結果は利得が自明でないことを示すので、表の非ゼロ参照率と衝突率を先に測る。優先度は低い。 |
| 深さ依存の部分整列（partial insertion sort） | 閾値以上の静かな手だけを整列し、閾値を深さに比例させる。 | Stockfish、YaneuraOu | 深さ依存化はSTCで≈+2.22、LTCで≈+3.29 Elo（[6b9a22b4](https://github.com/official-stockfish/Stockfish/commit/6b9a22b4)）。 | 現行は静かな手を全件整列する。静かな手が200を超える中将棋では、整列費用の比重がチェスより大きい。 |
| 王駒を攻める手の加点（check bonus in ordering） | 相手の王駒に利きを付ける静かな手を順序で上げる。 | Stockfish、YaneuraOu | STCで≈+4.52、LTCで≈+2.15 Elo（[9f5b31c2](https://github.com/official-stockfish/Stockfish/commit/9f5b31c2)）。 | 王手延長と同じく利きの判定が高価であり、王駒が2枚あり得る点も同じである。優先度は低い。 |

表の寿命と捕獲手の学習は、どちらも「1回の探索で表が育たない」というminase自身の診断結果と結び付く。
この2点を先に変えれば、見送り済みのcontinuation historyやhistory pruningの再開条件も同時に満たされる可能性がある。
順序付けの表と並んで、探索の結果を持ち越すもう1つの機構が置換表である。

### 置換表ではPVフラグと打ち切りの条件が残る

置換表の未導入手法は少なく、PVフラグと打ち切りの条件の2点が中心である。
構造面のクラスタ化、静的評価の保存、およびプリフェッチは既に棄却または見送られているので、残るのは項目の意味と使い方に関する改良である。

| 手法 | 仕組み | 採用エンジン | 測定値 | 中将棋への適用と照合 |
|---|---|---|---|---|
| PVフラグ（ttPv） | 局面がPV上にあったかを置換表の1ビットに残し、減深、特異延長の余裕、置換の優先度に使う。 | Stockfish、Reckless、Alexandria、PlentyChess | 導入はSTCで≈+2.66、LTCで≈+3.21 Elo（[70880b8e](https://github.com/official-stockfish/Stockfish/commit/70880b8e)）。 | 実装済み置換表の改良である。多くのLMRの細部と特異延長の余裕値がこの印を前提にする。 |
| 置換表の打ち切り条件（TT cutoff conditions） | 打ち切りを非PVノードに限り、浅い深さではcut nodeの予想と値の向きが一致する場合に限る。 | Stockfish、Reckless、Viridithas | cut nodeとの一致はSTCで≈+2.72、LTCで≈+1.02 Elo（[43e100ae](https://github.com/official-stockfish/Stockfish/commit/43e100ae)）。同じ深さのfail-low値の許容はSTCで≈+3.91、LTCで≈+1.31 Elo（[b280d2f0](https://github.com/official-stockfish/Stockfish/commit/b280d2f0)）。 | 現行は根以外の全ノードで深さが足りれば打ち切る（[探索実装](../../src/search/alphabeta/negamax.rs)）。PVの品質にも関わる。 |
| 二次エージング（secondary aging） | 打ち切りに失敗した深い非正確値の項目の深さを下げ、置換されやすくする。 | Stockfish、PlentyChess | Stockfishの導入はSTCで≈+1.21、LTCで≈+2.04 Elo（[c9af7674](https://github.com/official-stockfish/Stockfish/commit/c9af7674)）。その後に一度削除され、限定形で戻された（[495296fc](https://github.com/official-stockfish/Stockfish/commit/495296fc)、[94beadff](https://github.com/official-stockfish/Stockfish/commit/94beadff)）。PlentyChessは+1.07±0.85と+1.19±0.97 Elo（[#359](https://github.com/Yoshie2000/PlentyChess/pull/359)）。 | 実装済み置換方針の改良である。採否の揺れが大きく、優先度は低い。 |

ttPvは単独でも効果を示したが、真価は探索側の細部がこの印を読むときに出る。
置換表と探索の改良が木の形を変えるのに対し、次の評価関数の改良は木の葉の値そのものを変える。

### 評価と学習では出力の後処理が未着手である

評価関数の未導入手法は、学習済みの評価値に掛ける後処理と、段階10の列挙に入っていないNNUEの構成要素に分かれる。
minaseの学習PSTは静かな局面の選別、引き分けの保持、細かい量子化を既に備えるので、将棋の学習工程に由来する多くの工夫は実装済みに当たる。

| 手法 | 仕組み | 採用エンジン | 測定値 | 中将棋への適用と照合 |
|---|---|---|---|---|
| 駒量に応じた評価出力の縮尺（material-dependent output scaling） | 評価関数の出力全体に、盤上の駒量で変わる倍率を掛ける。優勢側が駒を残し、劣勢側が交換を避ける方向に働く。 | Stockfish | 駒量による倍率はSTCで≈+4.05、LTCで≈+4.64 Elo（[1dbd2a1ad5](https://github.com/official-stockfish/Stockfish/commit/1dbd2a1ad5)）。歩を含めた版はSTCで≈+3.71、LTCで≈+1.96 Elo（[5af09cfda5](https://github.com/official-stockfish/Stockfish/commit/5af09cfda5)）。 | 実装済みの序中盤と終盤の補間は2つの表を混ぜるが、出力全体への倍率は持たない。倍率の係数は既存のSPSAの対象に加えられる。 |
| 楽観値（optimism） | 根の評価に応じた小さな楽観値を加えてから駒量の倍率を掛け、劣勢側に緊張を保たせる。 | Stockfish | 導入はSTCで≈+0.20、LTCで≈+1.36 Elo（[a5a89b27c8](https://github.com/official-stockfish/Stockfish/commit/a5a89b27c8)）。非対称版はSTCで≈+1.46、LTCで≈+1.76 Elo（[908811c24a](https://github.com/official-stockfish/Stockfish/commit/908811c24a)）。 | 反復負け回避の設計書が辞退したcontemptは引き分け値を動かすが、本手法は全局面の評価の縮尺を動かす点が異なる。効果が小さく優先度は低い。 |
| NNUEの構成要素のうち段階10の列挙外（pairwise multiplication、SCReLU、factoriser、threat inputs） | 第1層の出力の対ごとの積、二乗した切り詰め活性化、学習時だけの仮想特徴、および「駒Xが駒Yに利く」組の入力特徴。 | Stockfish、bullet、Viridithas | 対ごとの積はSTCで≈+5.21、LTCで≈+5.37 Elo（[cb9c2594fc](https://github.com/official-stockfish/Stockfish/commit/cb9c2594fc)）。二乗活性化はSTCで≈+3.40、LTCで≈+3.86 Elo（[c079acc26f](https://github.com/official-stockfish/Stockfish/commit/c079acc26f)）。脅威入力を含む版はStockfish 17比で累積+32.87 Eloだが単独の値ではない（[Regression Tests](https://github.com/official-stockfish/Stockfish/wiki/Regression-Tests)）。仮想特徴は学習初期に効くと説明される（[nnue-pytorch docs](https://github.com/official-stockfish/nnue-pytorch/blob/master/docs/nnue.md)）。 | 段階10はNNUEの再挑戦を予定するが、列挙は拡幅、出力バケット、相対王駒特徴、並列演算命令（SIMD）の方針、正則化に限られる。仮想特徴は、第1層の初期化の飽和というminaseの教訓と同じ学習初期の問題を扱う。脅威入力は利き集合を要し、利き数の表の不採用と同じ費用の壁に当たる。 |
| 歩兵と仲人の閉塞、並進する小駒（pawn block、tandem） | 自駒が歩兵や仲人の前進を塞ぐことを減点し、敵陣の境で支え合う小駒の組を加点する。 | HaChu | 個別の測定は見つからなかった（[hachu.c](https://salsa.debian.org/debian/hachu)）。 | 調査メモで言及済み、計画なし（[evaluation-terms-survey.md](evaluation-terms-survey.md)）。段階9の7項目には含まれない。 |
| 知識蒸留（knowledge distillation） | 強い評価器の値で自己対局の局面を付け直し、軽い評価関数を学習する。 | INUGAMI | 構成の記述だけで、単独の効果量は示されていない（[INUGAMIの第35回世界コンピュータ将棋選手権アピール文書](https://www.apply.computer-shogi.org/wcsc35/appeal/appeal_round2_250503b.pdf)）。 | 調査メモで言及済み、計画なし（[teacher-generation-prior-art.md](teacher-generation-prior-art.md)）。同メモは中将棋に強い教師がないため適用不能と判断した。 |

評価の後処理は、学習をやり直さずにSPSAで係数を合わせられる点で、NNUEの構成要素より安い。
評価と探索が決まった後に残るのは、同じ計算量をどの時間と計算資源へ割り当てるかという問題である。

### 時間管理、並列探索、実行環境では相手の時計と多様化が残る

この3領域は、既存の設計書が大半の標準手法を扱い済みであり、未導入手法は5件に限られる。
並列探索の3件は既定の`Threads=1`では効かず、段階11で並列度を上げる判断をした後に意味を持つ。

| 手法 | 仕組み | 採用エンジン | 測定値 | 中将棋への適用と照合 |
|---|---|---|---|---|
| 相手の残り時間を考慮した配分（opponent-clock-aware allocation） | 自分の残り時間が相手より少ないとき、目標時間を縮める。 | Stockfish | STCで≈+1.13、LTCで≈+1.08 Eloであり、切れ負けと周期制の非劣性試験も通過した（[5ca4bfe7](https://github.com/official-stockfish/Stockfish/commit/5ca4bfe72b9c0b1ba565c1da081d3531cce98a64)）。 | 将棋の通信規約USIの`go`は双方の残り時間を渡すので、追加の通信は要らない。lishogiの対人戦では時間切れの回避にも効く。 |
| スレッドごとの減深の揺らぎ（per-thread LMR noise） | 補助スレッドごとに減深量や探索窓を少しずつ変え、探索木を多様にする。 | Reckless、Stockfish | Recklessは1スレッドで−0.10±1.03、複数スレッドの5+0.05秒で+3.19±2.37 Elo（[0bf14a3](https://github.com/codedeliveryservice/Reckless/commit/0bf14a3)）。Stockfishのスレッド依存の窓と減深は8スレッドのLTCで≈+1.31と≈+2.04 Elo（[8ecfc3c8](https://github.com/official-stockfish/Stockfish/commit/8ecfc3c89d48418f84864fb235635d69e6ea37b8)、[39c077f1](https://github.com/official-stockfish/Stockfish/commit/39c077f15a88ff1e563971c396eb9a27b0ac6ac5)）。 | 実装済みLazy SMPの改良である。現行の多様化は補助ワーカーの深さの飛ばしで行う。 |
| 補正表とhistory表のスレッド間共有 | 補正表などを全スレッドで共有する。 | Stockfish、Caissa、PlentyChess | Stockfishは8スレッドのLTCで≈+3.59 Elo（[1a67ccc7](https://github.com/official-stockfish/Stockfish/commit/1a67ccc72ef2e3c06e9c905a793a14416d53643f)）。Caissaは+6.12±3.34 Elo（[24fde49](https://github.com/Witek902/Caissa/commit/24fde49)）。 | 現行の補正表は探索ワーカー内の配列である。1スレッドでは効果がない。 |
| ABDADA（Weillが提案した分散αβ探索） | 他のスレッドが探索中の手を後回しにし、同じ部分木の重複探索を減らす。 | texel、Stockfish（のちに削除） | texelではLazy SMP比で4コア+8.2、16コア+18.4 Eloだが1コアでは−8.1 Elo（[TalkChess](https://talkchess.com/forum3/viewtopic.php?f=7&t=64824)）。Stockfishの類似機構は≈+2.67 Eloで導入後に削除された（[217840a6](https://github.com/official-stockfish/Stockfish/commit/217840a6a5a40b516cab59a450a9f36352997240)、[a0e2debe](https://github.com/official-stockfish/Stockfish/commit/a0e2debe3f1d14f84984a9a2c1482dc41f695548)）。 | 4スレッド以上で意味を持つ。優先度は低い。 |
| 大きなページの使用（large pages） | 置換表と作業領域を大きなメモリページへ置き、アドレス変換の失敗を減らす。 | Stockfish | 典型的にNPS 5〜10%増と記される（[Advanced topics](https://github.com/official-stockfish/Stockfish/wiki/Advanced-topics)）。作業領域への適用はSTCで≈+1.02 Elo（[75edbee0](https://github.com/official-stockfish/Stockfish/commit/75edbee01e6f8cb53a2555499192ccaddb883577)）。 | minaseは`unsafe_code`を禁じており、実現できるかは割り当て器とオペレーティングシステムの設定に依存し未確認である。持ち時間2倍が+216 Eloという感度から見て、効果は数Eloにとどまる。 |

時間と資源の割り当ての改良は、どれも中将棋の規則に触れないので移植は容易だが、効果量も小さい。
規則に触れる改良は、将棋と大盤変種のエンジンが独自に積み上げてきた。

### 将棋と大盤変種に固有の手法は王駒捕獲と静止探索に集まる

将棋系と中将棋のエンジンに固有の未導入手法は、詰みや王駒捕獲の安価な判定、静止探索の打ち切り、および余裕値や順序の学習に分かれる。
Hoki and Muramatsuは、将棋の生の分岐因子が約80であるため前向き枝刈りがチェスより有効に探索空間を減らすと報告しており（[ScienceDirect](https://www.sciencedirect.com/science/article/abs/pii/S1875952111000450)）、分岐因子がさらに大きい中将棋でもこの傾向は保たれると推測できる。

| 手法 | 仕組み | 採用エンジン | 測定値 | 中将棋への適用と照合 |
|---|---|---|---|---|
| 1手詰め相当の静的判定（mate-in-1 detection） | 探索や静止探索の節点で、1手で詰む手を専用の静的判定で探し、見つかれば勝ち値を返す。 | YaneuraOu、Shokidoki、HaChu | YaneuraOuの静止探索での1手詰め判定は1手1秒でR+37.54、6秒でR+35.33（[yaneuraou-search.cpp](https://github.com/yaneurao/YaneuraOu/blob/master/source/engine/yaneuraou-engine/yaneuraou-search.cpp)）。Shokidokiは王の安全度と打ち詰めの判定を合わせて、20分の対局の得点率が約28%から約40%へ上がった（[Shokidoki](https://home.hccnet.nl/h.g.muller/shokidoki.html)）。 | 中将棋では「相手のどの応手でも最後の王駒が取られる手」の判定になる。将棋の利得は利きの差分更新に支えられた安価な判定に由来し、minaseの利き数の表は不採用になった。辞退済みの詰み専用探索とは別の、探索内の静的判定である。 |
| 静止探索の自動fail-high（HaChu auto-fail-high） | 静止探索の末端で子を探索する前に捕獲手の損得を調べ、親の捕獲の利得を上回る取り返しがあれば直ちにfail-highとする。 | HaChu | 個別の測定は見つからなかった（[hachu.c](https://salsa.debian.org/debian/hachu)）。HaChuの静止探索全体は+190 Eloと記される（[TalkChess](https://talkchess.com/viewtopic.php?t=48305)）。 | 中将棋のエンジンでの実装例である。獅子の2枚取りを含む損得の計算を静止探索の中で安価に行えるかが論点になる。 |
| 動的なfutility余裕値（dynamic futility margin） | 探索中に観測した位置評価の変化量の分布から、約90%を覆う余裕値を推定して使う。 | Bonanza系の研究 | 固定の余裕値に対して175勝138敗1分（[ゲームプログラミングワークショップ2007の論文](http://www.jaist.ac.jp/~t-hashi/papers/GPW2007-FP.pdf)）。 | 実装済みfutility pruningの改良である。現行の余裕値は固定値で、SPSAの対象である。探索中に推定する点が新しい。 |
| 手の分類確率の学習（realization probability、move-category learning） | 棋譜や自己対局から手の種類ごとの選択確率を学び、深さの消費や順序付けに使う。 | 激指、技巧 | 公開された効果量は見つからなかった（[激指の探索](https://www.logos.t.u-tokyo.ac.jp/~gekisashi/algorithm/search.html)、[技巧の紹介資料](https://denou.jp/tournament2015/img/PR/Gikou.pdf)）。 | 中将棋では王手や歩の脅威のようなチェスの順序信号が弱いので、学習した分類が代わりになり得る。学習データと費用の見積りが先に要る。 |

これらの手法は中将棋に近い領域の証拠を持つが、効果量の公表は少なく、1手詰め判定のように土台の費用が利得を左右するものが多い。
証拠の性質そのものにも限界があるので、次にそれを述べる。

## 証拠には4つの偏りがある

第1の偏りは、逐次検定で止めた推定値が上振れすることである。
逐次確率比検定は有利な揺らぎで境界を越えた時点で止まるので、合格した改良の点推定は真の値より大きく出やすい。
Stockfishの値はコミット文の勝敗数から算出したロジスティックEloであり、fishtestの正規化Eloとも一致しない。
Obsidianの+18.7±16.0 Eloのように、信頼区間が点推定と同程度に広い値も含まれる。

第2の偏りは、効果が加算的でないことである。
一覧の値はいずれも、その時点のエンジン全体に対する限界効果である。
Stockfishは浅い手数の履歴を2021年に外し2024年に戻し、ABDADA類似の機構や材料の補正の鍵を導入後に中立として外した（[dc5d9bdf](https://github.com/official-stockfish/Stockfish/commit/dc5d9bdf)、[a0e2debe](https://github.com/official-stockfish/Stockfish/commit/a0e2debe3f1d14f84984a9a2c1482dc41f695548)、[7f386d10](https://github.com/official-stockfish/Stockfish/commit/7f386d10)）。
再捕獲延長も導入後に独立した条件が消えている。
複数の候補を順に採用すれば、後の候補ほど効果は小さく測られる見込みが高い。

第3の偏りは、チェスでの効果が中将棋での効果を予測しないことである。
minase自身の記録がその実例であり、reverse futility pruning、late move pruning、razoring、improvingフラグ、駒種と到達升のhistoryは、チェスで確立した手法でありながらminaseのSTCで`H0`となった（[棋力向上の段階計画](../plans/strength-stages.md)）。
YaneuraOuも、Stockfishの探索を移した当初は係数が将棋に合わず、SPSAで調整し直して1手1秒でR+50、4〜8秒でR+90を得たと記す（[やねうら王V9.00](https://yaneuraou.yaneu.com/2025/08/20/yaneuraouv900-got-super-strong/)）。
144升、200を超える合法手、獅子の2枚取り、王駒捕獲による勝敗、および複数の王駒は、どれもチェスの定数と前提をそのまま使えない理由になる。

第4の偏りは、中将棋に固有の証拠がほぼないことである。
公開された中将棋の測定は、HaChuの初期版が100〜200局で記録した静止探索+190、置換表+170、補間+100、killer+80 Eloにとどまり、誤差は大きい（[TalkChess](https://talkchess.com/viewtopic.php?t=48305)）。
現代の選択的探索の手法を中将棋で測った公開結果は見つからず、Fairy-Stockfishも12×12の盤を扱わない。
YaneuraOuのR値も数百から数千局の自己対局であり、誤差は10〜30 Eloに及ぶ。
したがって、本書の順位は検証すべき仮説の順序であり、期待効果の予測ではない。
試す候補は、発動率と計算費用を診断した後、実装した候補コミットと基準コミットの自己対局を[測定の手引き](../guides/sprt.md)に従って測る。

## 土台と測定順序への含意

照合から見えた最大の構造は、minaseが見送りや棄却を重ねた手法の多くが、チェスエンジンでは別の土台の上で効いているという点である。
現行のStockfishでは、reverse futility pruningはPVフラグの立ったノードを除き、razoringは予想all nodeに限られる。
また、Stockfishは主historyを根探索のたびに消さずに3/4倍して持ち越しており、minaseのように根探索ごとに表を消す前提では、continuation historyのような大きな表は埋まりにくい。
minaseはこれらの土台を持たないまま個々の手法を測ったので、`H0`や発動率の不足は手法そのものの限界ではなく土台の欠如を反映している可能性がある。
そうであれば、ノード種別の追跡、置換表のPVフラグ、およびhistory表の持ち越しという3つの土台を先に整えることが、複数の候補と見送り済みの手法の再開を同時に可能にする。
捕獲履歴も同じ位置にあり、捕獲手の順序付けに加えて、捕獲履歴による戦術手の減深のような後続の手法の前提になる。

もう1つの含意は、測定の設計に関わる。
延長の系統やLMRの細部はLTCほど効き、1件あたりの効果は数Eloにとどまるので、H1を10 Eloに置いたSTC先行の段階ゲートでは取りこぼしやすい。
深さに依存しない順序付けの改良をSTCで先に測り、延長の系統は発動率をLTCの到達深さで診断してから測る、という2本立ての順序が、限られた測定時間で最も多くを学べる進め方である。
