# df-pn系アルゴリズムの一次文献調査（中将棋詰め探索の設計資料）

作成日: 2026-10-11。対象読者は、中将棋（12×12、持ち駒なし、獅子の2段階移動、王駒が片側最大2枚）の詰め探索をRustで実装する開発者である。各項目には、書誌、URL（読み込みの確認状況を併記）、および実装者にとって重要な点を列挙した。式は典拠の表記をそのまま写し、本文を直接読めなかった文献はその旨を明記した。

URLの確認状況の表記は次のとおりである。「確認済み」は本調査でPDFまたはHTMLを取得して本文を読んだことを、「書誌のみ」は書誌ページだけを読めたことを、「未確認」はリダイレクトまたは403で本文を取得できなかったことを表す。

## 用語

- 証明数 pn(n) は、節点 n を証明（攻め方の勝ち）するために証明しなければならない先端節点の最小個数であり、反証数 dn(n) は反証のための同じ量である。
- OR節点は攻め方手番、AND節点は玉方手番を表す。OR節点では pn が最小の子、AND節点では dn が最小の子を選ぶと最証明節点（most-proving node、MPN）に到達する。
- df-pn（depth-first proof-number search）は、pn と dn の閾値を各節点に与えて深さ優先で反復深化する探索であり、探索木では最良優先の pn-search と同じ節点を展開する。
- GHI問題（graph history interaction problem）は、同一局面の値が到達経路に依存するにもかかわらず置換表が経路を無視するために誤った証明または反証が生じる問題である。
- TCA（threshold controlling algorithm）とSNDA（source node detection algorithm）はKishimoto（2010）が提案した、無限ループと二重カウントへの対策である。

## 1. Allis らの Proof-Number Search と Allis の学位論文

### 1.1 Allis, van der Meulen, van den Herik, "Proof-Number Search"

- 書誌: L. V. Allis, M. van der Meulen, H. J. van den Herik, "Proof-Number Search," Artificial Intelligence, Vol. 66, No. 1, pp. 91–124, 1994. 原型は技術報告 CS 91-01（University of Limburg, 1991）である。
- URL: https://doi.org/10.1016/0004-3702(94)90004-1 （ScienceDirectは403で未確認。内容は下記の学位論文第2章と ICGA 2012 サーベイで確認した）。
- pn と dn の定義と更新則は次のとおりである。終端節点は勝ちなら (pn, dn) = (0, ∞)、負けなら (∞, 0)、未展開の先端節点は (1, 1) とする。OR節点では pn(n) = min_s pn(s)、dn(n) = Σ_s dn(s)、AND節点では pn(n) = Σ_s pn(s)、dn(n) = min_s dn(s) である。
- 1回の反復は選択、展開、評価、逆伝播の4段階からなり、根の pn または dn が0になるまで繰り返す。逆伝播は、ある祖先の pn と dn が変化しなくなった時点で打ち切り、次の選択をその祖先から始めてよい。
- 先端節点の初期化に分岐数を使う規則（OR節点で pn = 1, dn = 分岐数、AND節点で pn = 分岐数, dn = 1）は、1手分の情報を追加するだけで性能を改善すると報告されている（サーベイ第7.1節）。
- 本論文自体は探索空間を木と仮定している。合流（DAG）や閉路（DCG）では、子の値の和が最小証明集合の濃度の上界にしかならないことを、学位論文第2.3.4節が明記する。

### 1.2 Allis, "Searching for Solutions in Games and Artificial Intelligence"

- 書誌: L. V. Allis, Searching for Solutions in Games and Artificial Intelligence, Ph.D. thesis, Rijksuniversiteit Limburg, Maastricht, 1994（1994年9月23日公開審査）。
- URL: https://project.dke.maastrichtuniversity.nl/games/files/phd/SearchingForSolutions.pdf （確認済み、223ページ）。
- 第2.2.3節の定義2.5は、最証明節点を「true になると木の pn を1減らし、false になると dn を1減らす先端節点」と定義し、最小証明集合と最小反証集合が常に交わることを帰納法で証明する。この性質が、証明と反証のどちらを狙うかを選ばずに1つの節点を選べる理由である。
- 第2.3.1節はメモリ削減として、証明済みまたは反証済みの部分木の削除（DeleteSolvedSubtree）と、最も見込みの薄い子の削除（DeleteLeastProving）、および2段階探索 pn²-search を示す。pn² は第1段の最証明節点を根として節点数上限つきの第2段 pn-search を走らせ、終了後は根の子だけを残す。
- 第2.3.4節「Transpositions」は、DAG では和による pn が上界になるため、実用上は木として扱いつつ更新時にすべての親を再帰的に更新する方法を採ると述べ、図2.4の例で選択が誤り得ることも認めている。DCG については繰り返し規則がゲーム固有であるとして一般解を示していない。
- 初期化へのドメイン知識の導入は Othello では多数の実験による調整を要したと述べており、「初期化の効果は十分には理解されていない」と結論している。

## 2. Nagai の df-pn（学位論文、Nagai & Imai の等価性証明、Othello 応用、詰将棋応用）

### 2.1 Nagai, "Df-pn Algorithm for Searching AND/OR Trees and Its Applications"

- 書誌: A. Nagai, Df-pn Algorithm for Searching AND/OR Trees and Its Applications, Ph.D. thesis, Department of Information Science, University of Tokyo, 2002.
- URL: 公開PDFは本調査では見つからなかった。chessprogramming.org の著者ページ（https://www.chessprogramming.org/Ayumu_Nagai 、確認済み）と ICGA 2012 サーベイ、および KomoringHeights の参考文献（https://raw.githubusercontent.com/komori-n/KomoringHeights/main/source/engine/user-engine/docs/refs.md 、確認済み）が書誌を裏づける。内容は本調査では直接読めていないため、以下はサーベイと Kishimoto の論文が同論文に帰している内容である。
- サーベイ第4節は、df-pn が常に最証明節点を選ぶことの証明を本論文に帰している。
- サーベイ第5.1節は、SmallTreeGC（置換表が満杯になったら部分木サイズの小さい順に一定割合 R を捨てる）と置換戦略の併用を本論文に帰している。
- サーベイ第6.1節は、置換表エントリに親へのポインタを1つ持たせて合流を検出し、合流元の節点で和の代わりに最大値をとる二重カウント対策を本論文に帰している。
- サーベイ第6.2節は、df-pn 固有の GHI 対策（繰り返しの結果を単一の固定値として扱う特殊ケースの解）を本論文に帰し、Kishimoto & Müller（2004）は「根の閾値を ∞ − 1 にして探索し、閾値超過で戻ったら繰り返しを反証扱いにして再探索する」方法と要約している（第2.4節に詳述）。
- Kishimoto（2010）の脚注2は、本論文が無限ループ問題を記述していないこと、および長井のソルバの結果が再現不能であることを指摘している。

### 2.2 Nagai & Imai, "Proof for the Equivalence between Some Best-First Algorithms and Depth-First Algorithms for AND/OR Trees"

- 書誌: A. Nagai, H. Imai, "Proof for the Equivalence between Some Best-First Algorithms and Depth-First Algorithms for AND/OR Trees," IEICE Transactions on Information and Systems, Vol. E85-D, No. 10, pp. 1645–1653, 2002. 予備版は Korea-Japan Joint Workshop on Algorithms and Computation, pp. 163–170, 1999 である。
- URL: https://repository.dl.itc.u-tokyo.ac.jp/records/48966 （東京大学リポジトリ、PDFを確認済み）。
- 本論文が df-pn の原典に当たる。手続きは次のとおりである。根 r で thpn(r) = ∞、thdn(r) = ∞ とし、各節点 n では pn(n) ≥ thpn(n) または dn(n) ≥ thdn(n) が成立するまで（終了条件）配下を探索する。
- OR節点 n で pn 最小の子 nc と2番目に小さい子 n2 を選び、「thpn(nc) = min(thpn(n), pn(n2) + 1)」（式13）、「thdn(nc) = thdn(n) + dn(nc) − dn(n)」（式14、原文は dn(nchild) と表記）を与える。AND節点では「thpn(nc) = thpn(n) + pn(nc) − pn(n)」（式15）、「thdn(nc) = min(thdn(n), dn(n2) + 1)」（式16）である。
- 付録の negamax 形式では φ(n) を「OR節点の pn、AND節点の dn」、δ(n) をその双対と定義し、子の閾値を「nc.φ = n.δ + φc − ΦSum(n)」「nc.δ = min(n.φ, δ2 + 1)」で与える。ΦSum は子の φ の和、∆Min は子の δ の最小である。
- 置換表の使い方は「Look Up Transposition Table(n, φ, δ); if (n.φ ≤ φ || n.δ ≤ δ) return;」であり、閾値がすでに記録値以下なら探索せずに戻る。子の値は置換表から読み、未登録なら (1, 1) とする。
- 定理9は、十分なメモリがあれば df-pn が pn-search と同じく常に最証明節点を展開することを証明する。PDS は「漸近的にしか等価でない」と区別される。
- Seo のアルゴリズムは第2.3節に手続きとして再構成されている。根で thpn(r) = 2 とし、OR節点では子に親の閾値をそのまま渡し、AND節点では pn が非零の任意の子に thpn(child) = pn(child) + 1 を与えて反復深化する。これが「多重反復深化」である。

### 2.3 Nagai & Imai, "Application of df-pn+ to Othello endgames"

- 書誌: A. Nagai, H. Imai, "Application of df-pn+ to Othello endgames," Proceedings of the 5th Game Programming Workshop, IPSJ Symposium Series Vol. 99(14), pp. 16–23, 1999.
- URL: 公開PDFは見つからず、書誌は ICGA 2012 サーベイの参考文献と chessprogramming.org で確認した（書誌のみ）。
- df-pn+ は先端節点の pn, dn を評価関数 Hpn, Hdn で初期化し、さらに辺のコスト Costpn, Costdn を子の値に加える版である。Kaneko（2010）図2は「pn(c) += Costpn(n, c)」および子閾値「np = min(thpn, pn(n2) + 1) − Costpn(n, n1)」として差分を示す。
- サーベイ第7.3節は、Nagai が閾値増分 δ を定数 1 より大きくとった（pt(c) = min(pt(n), pn2 + δ)）と記す。
- サーベイ脚注9は、Nagai の Othello 実装が置換表エントリに子のエントリへのポインタを保持して pn, dn の再計算を速めたと記す。

### 2.4 長井歩・今井浩「df-pn アルゴリズムの詰将棋を解くプログラムへの応用」

- 書誌: 長井歩, 今井浩, 「df-pn アルゴリズムの詰将棋を解くプログラムへの応用」, 情報処理学会論文誌, Vol. 43, No. 6, pp. 1769–1777, 2002年6月. 先行する研究報告は 情報処理学会研究報告 AL, Vol. 2000, No. 103 (2000-AL-075), pp. 9–16, 2000年11月である。
- URL: https://ipsj.ixsq.nii.ac.jp/records/11597 （PDF https://ipsj.ixsq.nii.ac.jp/record/11597/files/IPSJ-JNL4306020.pdf を確認済み）。研究報告版は https://ipsj.ixsq.nii.ac.jp/records/32069 （PDFはスキャン画像で本文抽出不可）。
- 証明数の直観として「証明数は玉の逃げ方の総数を，反証数は攻め方の王手の総数を表す」と述べる。これは王が1枚で OR 節点の手が王手に限られる詰将棋の前提に依存する。
- 置換表の管理は、開番地法のハッシュで「占有率が 80%に達したら，全ハッシュの少なくとも 30%を SmallTreeGC で捨てる」とし、それ以外は SmallTreeReplacement（置き換え候補のうち部分木が最小のエントリを捨てる）を使う。部分木サイズとは、その節点の下に存在する節点の個数である。
- 優越関係は「盤上の駒の配置はまったく同じで，A の攻め方の持駒が，B の攻め方の持駒を真に含む」場合に限って成立し、B が詰めば A も詰み、A が不詰なら B も不詰である。df-pn では pn の上界だけでなく dn の上界にも優越関係を使える点が脊尾のアルゴリズムとの差であると論じる。
- GHI 対策は、ループに遭遇しても即座に不詰としないために、(1) 根の閾値を ∞ − 1 として探索し、(2) 根の (pn, dn) が (0, ∞) または (∞, 0) でなければ、閾値を ∞ に設定し直してループを直ちに不詰扱いにする探索を行う。付録の擬似コードでは「if (φc = ∞ − 1) φ(nc) = ∞; else if (δ(n) ≥ ∞ − 1) φ(nc) = ∞ − 1; else φ(nc) = δ(n) + φc − ΦSum(n);」として子の閾値に反映し、根以外のすべての節点でも同じ処理を行う。
- 証明駒に加えて反証駒（不詰を示すのに必要な玉方の最小持駒集合）を導入した。
- 二重カウント対策は、各局面に親局面へのポインタを1つ持たせ、記録と異なる親から到達したときに親をたどって合流点を確認し、合流点では子の証明数の和の代わりに最大をとる。たどる条件は「OR 局面ではつねに親をたどり，AND 局面ではその AND 局面の証明数と，子局面の証明数が一致する場合に限り親をたどる」である。著者自身が、長編詰将棋に多い「1本道に近い手順が先で同一局面に合流する」DAG にだけ効く ad hoc な方法だと限定している。
- 結果として300手以上の詰将棋をすべて解き、脊尾詰で解けなかった12題も解いた。

## 3. Seo, Iida, Uiterwijk, "The PN*-search algorithm: Application to tsume-shogi"

- 書誌: M. Seo, H. Iida, J. W. H. M. Uiterwijk, "The PN*-search algorithm: Application to tsume-shogi," Artificial Intelligence, Vol. 129, Nos. 1–2, pp. 253–277, 2001.
- URL: https://doi.org/10.1016/S0004-3702(01)00084-4 （DOIはマーストリヒト大学の書誌ページ https://cris.maastrichtuniversity.nl/en/publications/the-pn-search-algorithm-application-to-tsume-shogi で確認。ScienceDirect本体は403で本文未確認）。
- 本文は読めていないため、以下は Nagai & Imai（IEICE 2002）第2.3節、Winands らの章、ICGA 2012 サーベイに基づく。
- PN* は証明数だけを閾値に使う深さ優先反復深化である。根で thpn(r) = 2 から始め、AND節点で thpn(child) = pn(child) + 1 として反復深化する多重反復深化により、AO*（証明数だけの最良優先探索）と同じ節点を展開する。Nagai & Imai の定理5がこの等価性を証明した。
- 書誌ページの要約によれば、再帰的反復深化、動的評価、後続節点の順序付け、依存関係による枝刈りを組み合わせ、295題の難問のうち1題を除いて解いた。
- 反証数を対等に扱わないため反証（不詰の証明）に弱いことが、Winands らの章第2.4節と長井・今井（2002）第3節で指摘されている。長井・今井は、この歪みが300手以上の問題で解けないものを残す一因だと論じる。
- 脊尾詰の置換表は、約100個のエントリを調べて置き換え候補を選ぶとサーベイ第5.1節が記す（Seo 1998『コンピュータ将棋の進歩2』の記述を参照している）。
- 脊尾は正の相関の強い子（無駄合）のうち1つだけを pn, dn の計算に数える手法と、局面の優越関係の検出（Seo 1999）も導入した。優越関係の論文は 脊尾昌宏「詰将棋を解くアルゴリズムにおける優越関係の効率的な利用について」ゲームプログラミングワークショップ'99 であるが、ページ範囲は長井・今井（2002）と KomoringHeights が pp. 129–136、サーベイが pp. 137–144 と食い違っており、本調査では原典を確認できなかった。

## 4. Kishimoto と Müller の一連の研究

### 4.1 "Df-pn in Go: An Application to the One-Eye Problem"

- 書誌: A. Kishimoto, M. Müller, "Df-pn in Go: An Application to the One-Eye Problem," Advances in Computer Games 10: Many Games, Many Challenges (ACG'03), pp. 125–141, Kluwer, 2004（会議は2003年）。
- URL: https://doi.org/10.1007/978-0-387-35706-5_9 （書誌のみ。著者ページ https://webdocs.cs.ualberta.ca/~mmueller/dfpn.html が示すPDF ps/acg_kishimoto_mueller.pdf は404であった）。
- サーベイ第6.3節と Kishimoto（2010）によれば、本論文が df-pn(r) の初出である。df-pn(r) は根からの最小距離 md を置換表に記録し、md(n) < md(c) の子を normal、そうでない子を old と分類する。
- 三値（勝ち・負け・不明）を求めるための二相探索は、第1相の置換表を第2相で再利用して再探索を減らす（サーベイ第9節）。
- 発見的初期化を使うと閾値増分1では内部節点の再展開が増えるため、増分 δ を子の発見的証明数の平均に動的に設定した（Kishimoto 2005, Kishimoto & Müller 2005b。サーベイ第7.3節）。

### 4.2 "A General Solution to the Graph History Interaction Problem"

- 書誌: A. Kishimoto, M. Müller, "A General Solution to the Graph History Interaction Problem," Proceedings of the 19th National Conference on Artificial Intelligence (AAAI-04), pp. 644–649, 2004. 雑誌版は A. Kishimoto, M. Müller, "A solution to the GHI problem for depth-first proof-number search," Information Sciences, Vol. 175, No. 4, pp. 296–314, 2005（https://webdocs.cs.ualberta.ca/~mmueller/ps/kishimoto-mueller-infsci-ghi.pdf 、確認済み）。
- URL: https://cdn.aaai.org/AAAI/2004/AAAI04-102.pdf （確認済み）。日本語の詰将棋版は 岸本章宏「不詰を正しく証明するアルゴリズム」ゲームプログラミングワークショップ2004論文集（https://ipsj.ixsq.nii.ac.jp/record/97561/files/IPSJ-GPWS2004001.pdf 、確認済み）。
- GHI には2つのシナリオがある。first-player-loss では繰り返しが先手（攻め方）の負けであり、詰将棋とチェスの詰め問題がこれに当たる。このとき誤りは偽の反証としてのみ生じる。current-player-loss では繰り返した側の手が不合法（囲碁のSSK規則）であり、偽の証明と偽の反証の両方が生じる。
- Breuker らの Base-Twin Algorithm と Nagai の方法は first-player-loss の特殊ケースだけを扱い、current-player-loss では機能しないと論じる。Nagai の方法については「大きな閾値を超えるまで待つ必要があるため不詰の証明に時間がかかる」ことと「経路情報を持たないので1節点に2つの経路依存の結果を保存できない」ことを欠点として挙げる。
- 解法は、置換表エントリを base と twin に分ける。繰り返しに依存しない結果と未解決の pn, dn は base に、経路 p に依存する証明または反証は「経路 p のとき有効」として twin に保存する。別経路から同じ局面に到達したときは、twin の証明木を Kawano のシミュレーションで検証し、通れば再利用し、通らなければ base の情報で通常探索する。
- 経路は Zobrist 流に「code(p) = R[m1][1] ⊕ R[m2][2] ⊕ … ⊕ R[mk][k]」で64ビットに符号化する。手の順序に対して非可換であり、乱数表は MaxMove × MaxDepth 個である。将棋では移動元と移動先を分けて表を小さくし、深さ97で剰余をとる実装とした（GPW2004版）。
- df-pn 固有の変更は、twin に (反)証明を保存したら base の pn, dn を1に初期化すること、および根の閾値を ∞ ではなく ∞ − 1 にすることである。後者は df-pn が展開前に閾値を置換表に保存するために根で GHI が起こるのを防ぐ。
- 定理1は、すべての証明済み・反証済み節点が置換表に残る限り draw-first と draw-last のどちらの誤りも起こらないと述べる。置き換えで消えた場合でも返す真偽値は正しいが、証明木が誤り得ることを GPW2004版の定理3.2と図2が示し、正しい証明木が必要なら再探索で再構築する手続きを与える。Kishimoto（2005）の学位論文はこの再構築を効率化した。
- 実験では、GHI を無視した df-pn が、囲碁では2題で勝敗値は正しいが証明木が誤り、チェッカーでは18題で反証を誤った（原文 p.648 の “Results in Go” は incorrect proof trees、“Results in Checkers” は incorrect disproofs と区別する）。対策版のオーバーヘッドは節点数で −1.5%（囲碁）とごく小さかった。詰将棋版では長井の方法で解けなかった不詰問題を解いた。

### 4.3 "Dynamic Decomposition Search"

- 書誌: A. Kishimoto, M. Müller, "Dynamic Decomposition Search: A Divide and Conquer Approach and its Application to the One-Eye Problem in Go," 2005 IEEE Symposium on Computational Intelligence and Games (CIG'05), pp. 164–170, 2005.
- URL: https://webdocs.cs.ualberta.ca/~mmueller/ps/kishi_mueller_cig05.pdf （確認済み）。
- 局面を探索中に動的に部分問題へ分割し、各部分問題の結果を合成して全体の値を求める分割統治である。手を1つの部分問題に限定することで pn, dn が小さくなる（サーベイ第7.2節）。
- 囲碁の一眼問題に特化しており、盤面を独立領域に分割できることが前提である。中将棋の詰め探索には直接の適用先がない。

### 4.4 Kishimoto, "Correct and Efficient Search Algorithms in the Presence of Repetitions"

- 書誌: A. Kishimoto, Correct and Efficient Search Algorithms in the Presence of Repetitions, Ph.D. thesis, Department of Computing Science, University of Alberta, Spring 2005.
- URL: https://ualberta.scholaris.ca/handle/123456789/82010 （PDF https://ualberta.scholaris.ca/bitstreams/18160a26-c44b-47b5-bbe6-96b75bed1dda/download 、確認済み、196ページ。旧URL era.library.ualberta.ca はここへリダイレクトする）。
- 第3章は AAAI 2004 の GHI 解法の詳細であり、第3.2.2節は証明・反証が置換表に収まらない場合の正当性を扱う。
- 第4章は df-pn(r) を定義する。df-pn は GHI 解法を入れても閉路で無限ループし得る（図4.2、付録Aに証明）。原因は、祖先の pn が子の pn に加算されて自分自身を含む過大評価になり、閾値を常に超えるためである。
- df-pn(r) の計算則は、normal な子（n_i.md > n.md）だけで和をとり、未解決の normal な子が1つもなければ old な子の最大をとる。「n.δ = min_{1≤i≤k} n_i.δ」は変えず、φ の和だけを修正する。節点に old な子しか残らなければ、その節点の md を未解決の old な子の md の最小に更新する。
- 第4.3節の実験では、囲碁で df-pn が162題中150題、df-pn(r) が157題を解き、チェッカーでは115題から166題へ増えた。
- 長井のソルバでこの問題が報告されていない理由として、将棋では発生頻度が低い可能性と、ドメイン知識が隠している可能性を挙げ、激指チームとの私信で将棋でも起こることを確認したと記す。

### 4.5 Kishimoto, Winands, Müller, Saito, "Game-Tree Search Using Proof Numbers: The First Twenty Years"

- 書誌: A. Kishimoto, M. H. M. Winands, M. Müller, J.-T. Saito, "Game-Tree Search Using Proof Numbers: The First Twenty Years," ICGA Journal, Vol. 35, No. 3, pp. 131–156, 2012.
- URL: https://webdocs.cs.ualberta.ca/~mmueller/ps/ICGA2012PNS.pdf （確認済み）。
- 本調査の骨格となるサーベイである。df-pn の閾値更新を「pt(c1) = min(pt(p), pn2 + 1), dt(c1) = dt(p) − dn(p) + dn(c1)」（OR節点）、「pt(c1) = pt(p) − pn(p) + pn(c1), dt(c1) = min(dt(p), dn2 + 1)」（AND節点）と与え、図5の negamax 擬似コードは「Cbest.φ = N.δ + φc − ΦSum(N); Cbest.δ = min(N.φ, δ2 + 1);」である。脚注8は、この擬似コードが Nagai の原版の「本質的な性能問題」を修正した版だと注記するが、修正点は明示していない。
- 無限大の扱いとして「∞ + x = ∞ かつ min(∞, x) = x」を定めつつ、閾値計算では「∞ − x < ∞ − y if 0 ≤ y < x」が必要であり、実装では大きな有限値で ∞ を表すと述べる。
- 第5.1節は、αβ で常用される二段置換表が詰将棋ソルバのコミュニティでは効果がないと見られていること、Pawlewicz の Hex ソルバが4連続エントリの多重プローブで部分木サイズ最小のエントリを置き換えること、SmallTreeGC が詰将棋ソルバと囲碁の死活ソルバで使われることを記す。
- 第6節は DAG と DCG の3問題（過大評価、GHI、無限ループ）を整理し、第6.3節は「有限 DAG 上では df-pn は完全だが、正しい GHI 解法を入れても有限 DCG 上では不完全」と述べ、df-pn(r) と TCA の2方法の完全性は未解決問題だと記す。長井のソルバの実用的対策は「新しい先端節点がしばらく展開されなければ閾値を上げる」ことである（Nagai 2010 私信）。
- 第7.4.2節の Kawano のシミュレーションは、証明済み節点 n の証明木から各 OR 節点の手を借用して類似節点 m の証明を試みる手法であり、合駒処理のために考案されたが囲碁でも df-pn と組み合わされている。
- 第8節は並列化を扱い、Kaneko（2010）の仮想証明数（pn に「その節点に入っているスレッド数」を加える）を紹介する。

## 5. Pawlewicz & Lew, "Improving Depth-First PN-Search: 1 + ε Trick"

- 書誌: J. Pawlewicz, Ł. Lew, "Improving Depth-First PN-Search: 1 + ε Trick," Computers and Games (CG 2006), Lecture Notes in Computer Science Vol. 4630, pp. 160–170, Springer, 2007.
- URL: 発表スライド https://webdocs.cs.ualberta.ca/~hayward/talks/hex.epstrick.pdf （確認済み、49ページ）。Springer の章ページ https://doi.org/10.1007/978-3-540-75538-8_14 は認証ページへリダイレクトされ未確認。著者サイト www.mimuw.edu.pl/~pan/papers/lm-pns.pdf は403であった。
- 弱点の説明: OR節点に有望な兄弟が2つあり、pn が 300 と 301 のように近いと、df-pn は一方を pn が 1 増えるたびに離れて他方へ移る「seesaw」を繰り返す。置換表に探索木が収まらないと、戻るたびに部分木を作り直すことになる。
- 修正後の閾値はスライドで「min(⌈p2(1 + ε)⌉, pt)」である。元の df-pn の「min(p2 + 1, pt)」の加算1を乗算 (1 + ε) に置き換えるだけで、dt 側の「dt − d + d1」は変えない。サーベイ第7.3節は同じ式を「pt(c) = min(pt(n), ⌈pn2 × (1 + ε)⌉)」と書く。Winands らの章の式7も同じである。
- スライドが挙げる利点は、再帰呼び出しと置換表の再充填が減り木の走査が減ることである。欠点は、先端節点の展開順が pn-search と一致しなくなることと、ε が大きいと本質的でない部分木に時間を費やすことである。
- 実験は Atari Go で、置換表サイズを 2^13 から 2^20 節点まで変えたときの解答時間を PDS、強化 PDS、df-pn、強化 df-pn（1 + ε）で比較し、強化 df-pn が最速であった。Lines of Action でも試験している（サーベイ第10節）。
- 本手法は ε の選択が実験的である。理論的な最証明節点の選択を放棄する代わりに、置換表の容量不足に対する頑健性を得る位置づけであり、節点予算つきの探索で置換表を小さく抑えたい用途と相性がよい。

## 6. PDS と PDS-PN

### 6.1 Nagai の PDS

- 書誌: A. Nagai, "A New AND/OR Tree Search Algorithm Using Proof Number and Disproof Number," Proceedings of Complex Games Lab Workshop, pp. 40–45, ETL, Tsukuba, 1998. A. Nagai, A New Depth-First-Search Algorithm for AND/OR Trees, M.Sc. thesis, University of Tokyo, 1999.
- URL: いずれも公開PDFは見つからず、書誌は ICGA 2012 サーベイと PDS-PN 論文の参考文献で確認した（書誌のみ）。内容は Winands らの PDS-PN 論文（付録 NegaPDS）と章に基づく。
- PDS は PN* に反証数を加えた多重反復深化である。根で (proof, disproof) = (1, 1) から始め、MID が解けずに戻るたびに「if (r.proof <= r.disproof) r.proof++; else r.disproof++;」と一方の閾値を1増やす。
- MID の終了条件は「proofSum(n) = 0 || disproofMin(n) = 0 || (n.proof <= disproofMin(n) && n.disproof <= proofSum(n))」であり、両方の閾値を超えたときに戻る。df-pn の「どちらか一方を超えたら戻る」とは異なる。
- 子の選択は「disproof_child = max(disproof_child, proof)」で補正した dn の最小をとり、同値なら pn の小さい子を選ぶ。選んだ子の閾値は条件に応じて proof か disproof を1増やす。
- 置換表の照合は「proof >= n.proof && disproof >= n.disproof」なら記録値を使って戻るという形で行い、節点の展開時に終端判定をする遅延評価である。
- Nagai & Imai（IEICE 2002）は PDS を「pn-search と漸近的にしか等価でない」と位置づける。Winands らは PDS が PN² の3倍遅い（遅延評価と再展開のため）と測定した。

### 6.2 Winands, Uiterwijk, van den Herik, "PDS-PN: A New Proof-Number Search Algorithm"

- 書誌: M. H. M. Winands, J. W. H. M. Uiterwijk, H. J. van den Herik, "PDS-PN: A New Proof-Number Search Algorithm: Application to Lines of Action," Computers and Games (CG 2002), Lecture Notes in Computer Science Vol. 2883, Springer, 2003（ページ範囲は未確認）. 拡張版は "An Effective Two-Level Proof-Number Search Algorithm," Theoretical Computer Science, Vol. 313, No. 3, pp. 511–525, 2004.
- URL: https://dke.maastrichtuniversity.nl/m.winands/documents/PDSPNCG2002.pdf （確認済み）。関連する解説章 H. J. van den Herik, M. H. M. Winands, "Proof-Number Search and its Variants," Oppositional Concepts in Computational Intelligence, pp. 91–118, 2008 は https://dke.maastrichtuniversity.nl/m.winands/documents/pnchapter.pdf （確認済み）。
- PDS-PN は第1段を PDS、第2段を PN search とする2段階探索である。置換表にない節点を展開するときに、PDS の再帰呼び出しの代わりに節点数上限つきの PN search を走らせ、終了後は根だけを置換表に保存する。
- 第2段の上限は PN² と同じ「y = min(x × f(x), N − x)」、f(x) = 1 / (1 + e^((a − x)/b)) で、x は置換表の非空エントリ数である。LOA では (a, b) = (450K, 300K) が最良であった。
- 488題の LOA 終盤で PN が356題、PN² が470題、PDS が473題、αβ が383題を解き、PDS-PN は467題を解いた。記憶できる節点数を半分ずつ減らす実験では、PDS-PN は約10,000節点を下回るまで解答数が安定したが、PN² は急落した。
- PDS は GHI 問題を無視しており、LOA の三回同形引き分けで経路依存の値が生じることを認めている。
- サーベイ第5.2節は、df-pn を第1段にした DFPN-PN が PDS-PN を置き換えるべきだが実例はないと述べる。

## 7. Ueda, Hashimoto, Hashimoto, Iida, "Weak Proof-Number Search"

- 書誌: T. Ueda, T. Hashimoto, J. Hashimoto, H. Iida, "Weak Proof-Number Search," Computers and Games (CG 2008), Lecture Notes in Computer Science Vol. 5131, pp. 157–168, Springer, 2008. DOI 10.1007/978-3-540-87608-3_15。
- URL: https://doi.org/10.1007/978-3-540-87608-3_15 （Springer は認証ページへリダイレクトされ未確認。dblp の CG 2008 目次 https://dblp.uni-trier.de/db/conf/cg/cg2008.html でページ範囲を確認した）。本文は読めていないため、以下はサーベイ第6.1節と Kishimoto（2010）の記述に基づく。
- WPNS は Okabe（2005）の経路分岐数の小改良であり、合流による過大評価を避けるために AND 節点の pn を和ではなく「最大の子 + 残りの子の個数」で定める。サーベイは「pn(n) = max_{1≤i≤k} pn(ci) + k − 1」（k は子の数）、Kishimoto（2010）は「pn(n) = max_{i=1,…,k} pn(ni) + b − 1」（b は未証明の子の数）と書き、両者は未証明の子だけを数えるかどうかで異なる。OR 節点の dn も同様で、OR 節点の pn と AND 節点の dn は通常どおりである。
- Kishimoto（2010）は WPNS が二重カウントを避ける代償として「dn(G) を dn(J) + 1 に少し過大評価し」、内部節点の pn をほぼ1と仮定するため全体として過小評価になると評する。発見的初期化 h(n) を使う実装では (b − 1) を (b − 1) × h(n) に置き換えると大きく改善した。
- Kishimoto（2010）の表1では、df-pn + TCA + WPNS は78題中1題だけ未解決で SNDA と同数であったが、最難問3題では SNDA の方が2.5倍から6.2倍速く、WPNS は3.1倍から8.8倍多く節点を展開した。WPNS の節点展開速度は SNDA より約37%高い。
- WPNS も SNDA も無限ループを常に防ぐわけではなく、TCA と併用しなければ未解決が7題に増えた。

## 8. GPS将棋グループと長井の実装論文、および「無限ループ」に関する補足

### 8.1 置換表のGCについて確認できたこと

- 「df-pn のガーベージコレクション」を主題とする金子知適の論文は本調査では見つからなかった。GC の一次典拠は Nagai の修士論文（1999）であり、長井・今井（2002）の第4.1節が SmallTreeGC と SmallTreeReplacement の運用（占有率80%で少なくとも30%を捨てる）を記す。KomoringHeights v0.4.0 の記事はこれを「Nagai, A. A new depth-first-search algorithm for AND/OR trees. ICGA Journal 22.1 (1999): 35–36」と引いており、修士論文の要旨が ICGA Journal に掲載されたものを指すと推測されるが、本調査では確認できなかった。
- やねうら王の記事（第10節）は、GC を実装すると「GCで探索した情報が掃除されてしまったがために解けなくなる問題が出てきます」と述べ、Kishimoto & Müller（2004）の定理1が置き換えなしを前提とすることと符合する。

### 8.2 金子知適, "Parallel Depth First Proof Number Search"

- 書誌: T. Kaneko, "Parallel Depth First Proof Number Search," Proceedings of the Twenty-Fourth AAAI Conference on Artificial Intelligence (AAAI-10), pp. 95–100, AAAI Press, 2010.
- URL: https://ojs.aaai.org/index.php/AAAI/article/view/7551 （PDF https://ojs.aaai.org/index.php/AAAI/article/view/7551/7412 、確認済み）。
- 図2に df-pn と df-pn+ の擬似コードがあり、子の値を「(pn(c), dn(c)) = Lookup(c) or (Hpn(c), Hdn(c))」で取り、「pn(c) += Costpn(n, c)」を加え、子閾値を「np = min(thpn, pn(n2) + 1) − Costpn(n, n1)」「nd = thdn − dn(n) + dn(n1)」とする。GC で捨てられた節点は未訪問節点と同じく (1, 1) に戻る。
- 共有置換表を使う複数エージェントの並列化で、仮想証明数「vpn(c) = pn(c) + Costpn(n, c) + T(n, c)」を導入する。T(n, c) は c の子孫を訪問中のエージェント数であり、T = 0 なら逐次版と同じ木を展開する。ロックのオーバーヘッドは約1%であった。
- 図1は、df-pn を PVS（principal variation search）の根と内部節点から呼び出す2形態を示す。これは本プロジェクトの「主探索から節点予算つきで呼ぶ」用途の先例である。
- 経路依存の情報はスレッドローカルに持つ必要があると注意し、GHI と過大評価の対策は Kishimoto（2005, 2010）と Nagai（2001）に委ねている。

### 8.3 金子知適・田中哲朗・山口和紀・川合慧の評価関数と固定深さ探索

- 書誌: 金子知適, 田中哲朗, 山口和紀, 川合慧, 「詰将棋における df-pn+ 探索のための，展開後の証明数と反証数を予測する評価関数」, ゲームプログラミングワークショップ2004論文集, pp. 14–21, 2004（https://ipsj.ixsq.nii.ac.jp/record/97563/files/IPSJ-GPWS2004003.pdf 、確認済み）。関連して 「効率的な詰将棋探索のための評価関数」情報処理学会研究報告 GI, 2004-GI-11, pp. 3–8, 2004（https://ipsj.ixsq.nii.ac.jp/records/58552 、PDFはスキャン画像）。
- 書誌: 金子知適, 田中哲朗, 山口和紀, 川合慧, 「Df-pn with Fixed-Depth Search at Frontier Nodes」, ゲームプログラミングワークショップ2005論文集, pp. 1–8, 2005。論文誌版は 「新規節点で固定深さの探索を行う df-pn の拡張」, 情報処理学会論文誌, Vol. 51, No. 11, pp. 2040–2047, 2010（書誌は KomoringHeights refs.md とサーベイで確認、本文未読）。
- 評価関数の論文は、df-pn で一定節点数を探索した後の pn, dn を教師にして、先端節点の初期値を最小二乗法で学習する方法を提案し、最良の評価関数で探索節点数が半分以下になったと報告する。
- 固定深さ探索の論文は、新規先端節点で1手または3手の固定深さ探索を行って短い証明を早く見つける方法であり、サーベイ第7.4.3節と cir.nii の要旨によれば実行時間約1/3、メモリ約1/5になった。
- 評価関数の特徴量は持ち駒を含む将棋固有のものであり、学習済みの値は中将棋へ移植できない。方法論（探索後の pn, dn を回帰で予測する）は移植できる。

### 8.4 長井歩「長手数の詰将棋を解くプログラムを改善するヒューリスティックス」

- 書誌: 長井歩, 「長手数の詰将棋を解くプログラムを改善するヒューリスティックス」, ゲームプログラミングワークショップ2010論文集, pp. 29–35, 2010.
- URL: https://ipsj.ixsq.nii.ac.jp/records/71310 （PDF https://ipsj.ixsq.nii.ac.jp/record/71310/files/IPSJ-GPWS2010005.pdf 、確認済み）。
- 6種のヒューリスティクス A〜F のうち、(1) 王手駒を取る手で成・不成の両方があれば成だけを数える、(2) 王手駒を同種の駒で取る手が複数あれば1手だけ数える、(3) 詰む手または不詰の手を記憶して類似局面で優先する、の3つで実行時間が約77%に抑えられた。
- ヒューリスティクス F は OR 節点の dn を「同一方向からの王手は1手しか数えない」（8方向 + 桂の2方向）で計算する案であり、効果は問題依存で採用されなかった。
- 親ポインタによる DAG 検出を切ると実行時間と節点数が平均で約1.7倍に悪化し、二重カウント対策の重要性を示した。
- 脊尾の工夫として「初めて探索する AND 節点で受けの手の数が証明数の閾値を超えていても、受けた後の1手詰だけは調べる」ことを紹介し、自身のプログラムにも導入している。

### 8.5 「Df-pn and the problem of infinite loops」という題の論文について

- この題名の論文は本調査では確認できなかった。無限ループを扱う一次文献は、Kishimoto & Müller, "About the Completeness of Depth-First Proof-Number Search," Computers and Games (CG 2008), LNCS 5131, pp. 146–156, 2008（https://doi.org/10.1007/978-3-540-87608-3_14 、Springer は未確認、著者ページ https://webdocs.cs.ualberta.ca/~mmueller/dfpn.html で書誌を確認）と、第9節の Kishimoto（2010）である。
- サーベイ第6.3節によれば CG 2008 論文は「有限 DAG では df-pn は完全、有限 DCG では GHI 解法を入れても不完全」を示し、Kishimoto（2010）の図1は同論文から採った無限ループの例である。
- Hashimoto の「simulation」については、サーベイが引く Hashimoto（2002）の学位論文は必至問題を PDS で解く研究であり、シミュレーションの原典は Kawano（1996）である。本調査では Hashimoto による simulation 論文を同定できなかった。

## 9. Kishimoto, "Dealing with Infinite Loops, Underestimation, and Overestimation of Depth-First Proof-Number Search"

- 書誌: A. Kishimoto, "Dealing with Infinite Loops, Underestimation, and Overestimation of Depth-First Proof-Number Search," Proceedings of the Twenty-Fourth AAAI Conference on Artificial Intelligence (AAAI-10), pp. 108–113, AAAI Press, 2010. DOI 10.1609/aaai.v24i1.7534。
- URL: https://ojs.aaai.org/index.php/AAAI/article/view/7534 （PDF https://ojs.aaai.org/index.php/AAAI/article/view/7534/7395 、確認済み）。
- 問題の整理: df-pn は DCG で無限ループする。df-pn(r) は old な子を無視して pn, dn を過小評価する（図2左で dn(D) = dn(E) となるが本来は dn(E) + dn(F)）。df-pn も df-pn(r) も DAG で過大評価する（図2右で dn(G) = 2 × dn(J)）。
- TCA は pn, dn を標準どおり計算し、節点 n に未証明の old な子があるときに閾値を「thpn = max(thpn, pn(n) + 1); thdn = max(thdn, dn(n) + 1);」へ引き上げる。引き上げは inc_flag で選択した子にも伝播し、c が normal な子しか持たなくても c の閾値を上げる。これは c で即座に終了条件が成立して n に戻る無限ループを防ぐためである。先端節点を展開するか新しい閉路を検出するまで引き上げを続け、新しい閉路は Kishimoto & Müller（2004）の方法で置換表に保存する。
- 擬似コード DFPNwithTCA は、「if (n is a leaf) inc_flag = false; if (n has an unproven old child) inc_flag = true;」で旗を決め、「first_time && inc_flag」のときだけ閾値を上げ、以後の反復では引き上げた閾値を再利用して再探索の要否を判定する。md の更新は Kishimoto & Müller（2003）と同じく、未証明の old な子しか持たない節点で行う。
- SNDA は Nagai の親ポインタ法を一般化する。各置換表エントリに親ポインタ1つと固定数 N の手を保持し、合流先 nd から親ポインタをたどって現在の経路上の節点 ns に合流したら、合流を引き起こす手 m1, m2 を ns のエントリに保存する。計算則は「dn(n) = max_{i=1,…,l} dn(ni) + Σ_{j=l+1}^{k} dn(nj)（OR節点）」「pn(n) = max_{i=1,…,l} pn(ni) + Σ_{j=l+1}^{k} pn(nj)（AND節点）」であり、n1, …, nl はエントリに保存された手で得られる子である。
- 実装では、エントリは最大10手を64ビット整数に収め（平均分岐数約5の詰将棋だから可能）、手は整列後の添字で保存して経路によらず同じ結果にする。保存した手と親ポインタは一度保存したら消さない方が成績がよかった。
- 実験は300手から1525手の難問78題、2 GB の置換表、1題50,000秒で、未解決数は df-pn(r) 14、df-pn(r) + SNDA 20、df-pn + TCA 8、+ NAGAI 4、+ WPNS 1、+ SNDA 1 であった。df-pn(r) に SNDA を足すと過小評価が強まって悪化する。
- 著者は TCA と df-pn(r) の有限 DCG 上の完全性を未解決問題として残しており、置換表の置き換えとの相互作用も扱っていない。

## 10. 実装者向けブログ記事

各記事の助言には、持ち駒（drops）に依存するかどうかを付記した。「持ち駒依存」の項目は中将棋には適用できない。

### 10.1 やねうら王「詰将棋アルゴリズムdf-pnのすべて」

- URL: https://yaneuraou.yaneu.com/2024/05/08/all-about-df-pn/ （2024-05-08公開、2025-09-11追記。確認済み）。
- 著者は、論文だけを読んでも「完璧にはほど遠い実装しかできず」、『コンピュータ将棋の進歩2』第1章（脊尾昌宏「共謀数を用いた詰将棋の解法」）、『進歩4』第5章（長井歩「df-pnアルゴリズムと詰将棋を解くプログラムへの応用」）、『進歩6』第6章（岸本章宏「難問詰将棋をコンピュータで解く」）、および絶版の『ゲーム計算メカニズム』（岸本章宏・柴原一友・鈴木豪, 2010）を揃える必要があると述べる。記事中で引用された助言として「進歩本、ゲーム計算アルゴリズム、金子さんの論文。この3つを揃えてやっと完璧になる」を引く（金子の論文は第8.2節の並列 df-pn を指す）。持ち駒非依存。
- tanuki と dlshogi の詰将棋ルーチンには解けない問題があり「簡単に言えばバグっている」とし、KomoringHeights を「かなり完璧なコード」として参照実装に推す。持ち駒非依存。
- GC を実装すると「GCで探索した情報が掃除されてしまったがために解けなくなる問題が出てきます」と述べ、実装が複雑化すると警告する。持ち駒非依存であり、中将棋でも GC と正当性の相互作用（第4.2節の定理1の前提）は同じ形で生じる。
- 脊尾の1998年の章は証明数だけを明示し反証数の実装を示していないと指摘する。
- 2025-09-11の追記で Kishimoto & Müller（2004）の GHI 論文に言及し、重複節点の数え方を理解する鍵だと述べる。
- 無駄合の判定はどの実装でも難しいと述べる。無駄合は合駒（持ち駒を打つ受け）に関する詰将棋固有の規約であり、持ち駒依存である。

### 10.2 komorinfo（コウモリのちょーおんぱ）の連載

いずれも確認済みである。

- 「詰将棋に対するdf-pnアルゴリズムの解説」 https://komorinfo.com/blog/df-pn-basics/ （2021-06-30）。pn, dn の定義と OR/AND 節点の計算則を示し、実装上の三大問題として無限ループ（GHI）、分岐の合流による二重カウント、置換表の扱いを挙げる。持ち駒非依存。
- 「詰将棋探索における証明駒／反証駒の活用方法」 https://komorinfo.com/blog/proof-piece-and-disproof-piece/ （2021-07-04）。証明駒は詰みを示すのに攻め方が必要とする持ち駒集合、反証駒は不詰を示すのに玉方が必要とする集合であり、OR 節点では手 m による持ち駒の増減を補正した Before_{n,m}(p_{m(n)})、AND 節点では近接王手なら子の和集合、遠隔王手なら合駒の可能性から攻め方の独占駒種 A_n を加える。千日手を含む経路依存の局面は対象外と明記している。持ち駒依存であり、中将棋には適用できない。
- 「オープンソースの詰将棋エンジン『KomoringHeights』を作った」 https://komorinfo.com/blog/komoring-heights/ （2021-07-08）。df-pn を基礎に n手詰めルーチンと証明駒・反証駒で節点を削減した約1,500行の実装である。参照した実装として やねうら王の tanuki-mate-search、DeepLearningShogi の dfpn.cpp を挙げる。
- 「詰将棋ソルバーにおけるGHI問題対策」 https://komorinfo.com/blog/and-or-tree-ghi-problem/ （2021-10-18）。詰将棋では連続王手の千日手が攻め方の負けであるため、不詰の判定が経路に依存する。経路を区別して置換表を使わない方法（やねうら王の詰めルーチン）と、Kishimoto & Müller（2005）の base/twin 方式を比較し、KomoringHeights は千日手だけを扱う経路ハッシュ表を持ち、不詰を kNonRepetitionDisproven（経路非依存）と kRepetitionDisproven（経路依存）に分け、後者では反証駒を別経路で使わない。初版の実装は不完全で2021年11月のコミット 4e268e5b で修正したと追記している。持ち駒非依存（反証駒の扱いを除く）。
- 「高速な詰将棋ソルバー『KomoringHeights』v0.4.0を公開した」 https://komorinfo.com/blog/komoring-heights-v040/ （2022-01-11）。df-pn+（GPS将棋の評価関数を参考に初期値を調整）、二重カウントの簡易対策、TCA、PV 探索の改良（最善手と詰み手数を置換表に保存）、SmallTreeGC（置換表の85%で部分木最小の30%を削除、必要メモリが1024 MBから128 MBへ）を導入した。TCA は探索節点を増やすため適用範囲を限定し、補助的な詰み探索と併用した。GC と TCA は持ち駒非依存、df-pn+ の評価関数は持ち駒依存。
- 「詰将棋探索における簡易的な二重カウント対策」 https://komorinfo.com/blog/proof-number-double-count/ （2022-02-14）。合流しやすい手種（合駒、角・飛の持ち駒による王手、不成の王手）を発見的に l 個選び、「dn(n) = max(dn(n1), …, dn(nl)) + Σ dn(n_{l+1}, …, n_k)（OR節点）」「pn(n) = max(pn(n1), …, pn(nl)) + Σ pn(n_{l+1}, …, n_k)（AND節点）」とする。SNDA の計算式を、合流検出なしに手種の発見則で近似した方法である。将棋図巧第1番（69手）で探索局面数が 183,957,288 から 21,800,413 へ 88.1% 減った。手種の選び方は持ち駒依存だが、「合流しやすい手種に max を使う」という骨格は持ち駒非依存である。
- 「難解作品が解ける詰め将棋エンジン KomoringHeights v0.5.0 を公開した」 https://komorinfo.com/blog/komoring-heights-v050/ （2022-05-18）。SNDA 相当の合流検出を実装し、置換表エントリを32バイトから56バイトへ拡張して親ポインタ（12バイト）と合流フラグ（8バイト）を加えた。合駒の展開を歩→桂→香→銀→角→飛→金の順に遅延させた（持ち駒依存）。詰み後の余詰探索（PostSearch）を追加した。GC が PostSearch に必要なエントリを消す不具合を既知の問題として挙げる。
- 「安定性が向上した詰将棋エンジン『KomoringHeights v1.0.0』を公開した」 https://komorinfo.com/blog/komoring-heights-v1/ （2022-12-30）。本探索を「M手以下で詰むか」を判定する形に拡張し、「M手以下で詰む かつ M−2手以下で詰まない」を調べて正確な詰み手数を得る。v0.5.0 の PostSearch は千日手回避の流れで無限ループに陥ることと GC が必要なエントリを消すことがあったため廃止した。無駄合検知は将棋の規則と整合しないため削除した。df-pn は裸玉（王手が多く兄弟節点が多い局面）に弱いと述べる。持ち駒非依存（無駄合を除く）。
- KomoringHeights の参考文献 refs.md（確認済み）は、GHI 対策、無限ループ対策（TCA）、二重カウント対策（長井・今井2002と Kishimoto 2010）、優越関係（脊尾1999）、固定深さ探索（金子ら2010）、deep df-pn（Song, Iida, van den Herik 2016）を列挙する。

### 10.3 eukaryote「df-pn亜種でオセロの終盤完全読みを試みた」

- URL: https://eukaryote.hateblo.jp/entry/2020/05/03/133918 （2020-05-03、確認済み）。実装は https://github.com/eukaryo/algorithm-study/blob/master/DFPN_single_file.cpp 。
- Othello 終盤の多値の結果を、「解の値に対する二分探索」で二値問題の列に変換し、各二値問題を証明数・反証数を閾値とする再帰的反復深化で解く。
- 二重カウントの対策として WPNS、seesaw の対策として 1 + ε trick を参照し、置換表が満杯になったときの GC と終盤の null window search は未実装と明記する。
- Othello には繰り返し規則がないため、局面と状態（履歴）を区別する必要がない点を注記しており、中将棋とは閉路の扱いが異なる。
- 著者は、この分野の高度な手法が「人工的に長い問題例に過適合している可能性」を指摘する。アルゴリズムの詳細は『ゲーム計算メカニズム』とサーベイに委ねている。
- 中将棋との関係では、持ち駒非依存であり、多値（R1 の引き分けなど）を二値探索へ還元する発想は参考になる。

## 11. 詰み手数、二重カウント、TCA についての補足

- 最短詰み手数を df-pn の枠内で選ぶ手法を主題とする学術論文は、本調査では見つからなかった。長井・今井（2002）の付録は φ, δ だけを置換表に保存し、手数を扱わない。
- 確認できた具体的方式は KomoringHeights だけである。v0.4.0 は最善手と詰み手数を置換表に保存して解手順の構築に使い、v1.0.0 は「M手以下で詰む」を判定する探索を M について繰り返し、「M手以下で詰む かつ M−2手以下で詰まない」で正確な手数を確定する（第10.2節）。長井・今井（2002）は393手の「呪われた夜」に265手以上の早詰があることを発見したと報告しており、df-pn が返す証明木が最短とは限らないことを示す。
- 二重カウント（DAG の合流）への対策は3系統ある。長井の親ポインタ法（第2.4節）は合流元で和を max に置き換えるが過小評価を招き、SNDA（第9節）は合流に関与する子だけを max にまとめて残りは和にし、WPNS（第7節）は検出せずに「最大 + 残りの子の個数」で一律に抑える。いずれも TCA のような無限ループ対策と併用する必要がある（Kishimoto 2010）。
- TCA の要点は、df-pn(r) が old な子を無視して pn, dn を小さくすることで終了条件を破るのに対し、閾値を「max(thpn, pn(n) + 1)」へ上げて終了条件を破る点にある。閾値を上げると探索空間が広がる懸念に対し、著者は「ほとんどの節点は normal な子しか持たない」という観察で答え、実験で df-pn(r) より良いことを示した。
- サーベイ第6.3節が記す長井の実用策（新しい先端節点が一定期間展開されなければ閾値を上げる）は、TCA を発見則で代替したものといえる。

## 中将棋への適用上の注意

本節は、RULES.md（中将棋統合競技規則案 第15版）の条文と対照して、上記の技法のうち何が適用できないか、何を再定義すべきか、何が閉路対策として必要かを整理する。

### 持ち駒（drops）を前提とするため適用できない技法

- 証明駒と反証駒（脊尾1999、長井・今井2002第4.4節、komorinfo 2021-07-04）は、詰みに必要な攻め方の持ち駒集合を置換表に記録して優越局面へ結果を転用する。中将棋は第4条第4項および第5項により取った駒を再使用せず持ち駒がないため、この概念自体が成立しない。
- 優越関係（長井・今井2002第4.2節）は「盤上が同一で持ち駒が真に包含する」場合に限って定義されるため、持ち駒のない中将棋では同一局面以外に優越関係は生じない。置換表の照合は厳密な同一局面に限られる。
- 合駒に関する技法はすべて適用できない。具体的には、Kawano（1996）の simulation の原動機となった無駄合、komorinfo v0.5.0 の合駒の遅延展開、やねうら王記事の無駄合判定、長井（2010）のヒューリスティクス（3）の一部、脊尾の「正の相関の強い子を1つだけ数える」手法のうち合駒を対象とする部分である。ただし盤上の駒を動かして王手を遮る「移動合」は中将棋にも存在するため、遮断手を相関の強い子として1つに数える発想は、合駒以外の形で再利用できる。
- komorinfo（2022-02-14）の簡易二重カウント対策が選ぶ手種（合駒、持ち駒の角・飛による王手、不成の王手）は将棋固有である。「合流しやすい手種に max を使う」という骨格だけが残り、中将棋では獅子の2段階移動や走り駒の経路違いによる合流など、別の手種を実測で選び直す必要がある。
- 金子ら（2004）の評価関数の特徴量は持ち駒を含み、学習済みパラメータは移植できない。Kaneko（2010）が Cost として与えた「捨て駒へのペナルティ」も、持ち駒がなく捨て駒の意味が異なる中将棋では再設計が要る。

### 単一の王を前提とする技法と、王駒が2枚になる場合の再定義

- 長井・今井（2002）の「証明数は玉の逃げ方の総数、反証数は攻め方の王手の総数」という直観、長井（2010）のヒューリスティクス F（同一方向の王手を1手に数える）、および OR 節点の手生成を王手に限る設計は、相手の王駒が1枚で「王手」が一意に定まることを前提にする。
- 中将棋では第20条により醉象が成った太子も王駒であり、王将と太子の両方を持つ側は一方を取られただけでは敗北しない。RULES.md 第8条第1項の王手はどの王駒への捕獲脅威も含むので、規則上の王手と、ソルバが OR 節点の候補にする「最後の王駒への捕獲脅威」は区別して定義する必要がある。王駒が2枚ある間は、片方への脅威は最後の王駒への脅威ではなく、詰め探索の OR 節点で許す手の集合を「相手の最後の王駒を次に取れる手」または「2枚同時に取れる手」に限るか、あるいは全合法手を許すかを設計で決める必要がある。
- 終局条件は第21条第1項から第3項（最後の王駒の捕獲または詰み）であり、第21条第5項は獅子が1手で王将と太子の両方を取れば勝ちと定める。獅子の2段階移動は第7条第8項により第1段階の捕獲を反映した局面で第2段階を判定するため、終端判定の手生成で獅子の居喰いと2枚取りを扱う必要がある。
- 第21条第3項bおよび第4項により、王手を受けている側が相手の最後の王駒を先に取れば、その側の勝ちである。第8条により攻め方の王手が自分の最後の王駒を相手の利きに残すことも合法なので、すべての AND 節点で「玉方が攻め方の最後の王駒を取れるなら反証」という終端判定が必要になる。将棋では自殺手が不合法なので詰将棋ソルバにこの判定はなく、単一の王を前提とする設計の差がここに現れる。
- 第8条第3項から第5項により、自分の王駒を相手の利きへ置く着手や王手を放置する着手も合法である。AND 節点の子は「王手を解消する手」ではなく全合法手になり、玉方の分岐数が将棋より大きくなる。ただし、玉方の最後の王駒を次に取られる手は即座の負けなので健全に枝刈りでき、2枚ある王駒の一方を捨てる手は本物の受けなので残す必要がある。Kishimoto（2010）が SNDA のエントリに最大10手を収めたのは「平均分岐数約5」の詰将棋だから可能であり、中将棋では手の保存形式を見直す必要がある。
- E1（第32条）を採用する規則セット（lishogi、HaChu）では詰みでは終局せず最後の王駒の実捕獲まで続くため、詰め探索の終端条件を規則セットごとに切り替える必要がある。第23条の「合法手がない側の負け」も終端条件に含める。

### 閉路（繰り返し）を扱う技法と、RULES.md の反復規則との対応

- Kishimoto & Müller（2004）の2つのシナリオに中将棋を当てはめると、規則によって扱いが変わる。詰将棋は first-player-loss（繰り返しは攻め方の負け、誤りは偽の反証だけ）であり、長井の ∞ − 1 方式や Breuker の BTA でも足りる。これに対し R2 と R3（第31条）は既出局面を再現する着手そのもの（R2 は2回目以降、R3 は4回目の出現だけ）を第26条第11項の不合法な着手とするため、Kishimoto & Müller の current-player-loss（囲碁のSSK規則と同型）に当たる。この場合は偽の証明と偽の反証の両方が生じ、著者らは Nagai の方法と BTA が機能しないと論じ、base/twin と経路符号による2004年の方式だけが正当性を示されている。R3 では2回目と3回目の再現が合法なので、経路の状態は「再現したか」ではなく出現回数を持たなければならない。また R2 は開始局面を含む既出局面を対象とするため、主探索の途中から呼ばれるソルバは根より前の対局履歴を根の状態に引き継ぐ必要がある。
- R1（第31条）は4回目の出現で原則引き分け、ただし一方の着手がすべて攻撃的着手なら攻撃側の負けである。OR 節点の手を王手に限る詰め探索では根以後の攻め方の着手がすべて攻撃的になるため、繰り返しは攻め方の勝ちにはならず、first-player-loss に近い形で扱える。ただし R1 の判定区間は最初の同一局面の出現から始まり根より前にも及ぶので、区間全体で攻め方の着手がすべて攻撃的だったとは限らず、裁定には4回目の出現と「直前の不可逆手から可逆手が12手以上」という前提条件もある。したがって「繰り返しは攻め方の非勝利」とするのは規則上の断定ではなく保守的な近似であり、正確な裁定とは一致しない。詰み判定を保守的に（詰みの見逃しを許容し、偽の詰みを許容しない）設計するなら、この近似は安全側である。
- 第24条第1項cにより、先獅子による直後の捕獲禁止の有無と対象は同一局面の条件に含まれる。置換表のキーがこの状態を含まないと、GHI 対策とは別に、合法手集合の異なる局面を同一視して誤った証明または反証を転用する。R2 と R3 の同一局面判定は第24条第1項aからcまでで行うため、経路ハッシュとは別に、局面ハッシュ自体に先獅子状態を含める必要がある。同項dにより、P2 を採用する規則セットでは成りの待機状態も同じ扱いになる。
- 無限ループ対策（df-pn(r) または TCA）は閾値の問題であり、繰り返し規則の種類に依存しない。中将棋では獅子のじっと（第12条第9項）や王駒の往復で短い閉路が将棋より容易に生じるため、Kishimoto（2005）が囲碁のパスで観察した「短いループで pn が自己参照する」状況が頻発し得る。TCA と SNDA または WPNS の併用（Kishimoto 2010）を既定とし、df-pn(r) 単独は避けるのが文献上の推奨である。
- GHI 対策の定理1（Kishimoto & Müller 2004）は証明済み・反証済みエントリが置換表から消えないことを前提にする。SmallTreeGC（長井・今井2002）を使うなら、Kishimoto（2005）の証明木再構築か、KomoringHeights v1.0.0 のように再探索で手順を確定する手当てが必要である。節点予算つきで主探索から呼ぶ場合は、予算到達で打ち切った部分木の pn, dn が置換表に残るため、Kaneko（2010）が注意する「経路依存情報の分離」と「中間結果で確定結果を上書きしない」規律を守る必要がある。

### 持ち駒や王の枚数に依存せず適用できる技法

- df-pn の閾値更新（Nagai & Imai 2002）、1 + ε trick（Pawlewicz & Lew 2007）、TCA と SNDA（Kishimoto 2010）、WPNS（Ueda ら2008）、base/twin と経路符号による GHI 対策（Kishimoto & Müller 2004）、SmallTreeGC と SmallTreeReplacement（長井・今井2002）、分岐数による初期化（Allis 1994）、固定深さの先端探索（金子ら2005）、Kawano の simulation の一般形（証明木からの手の借用）、Kaneko（2010）の仮想証明数による並列化は、いずれもゲーム規則に依存しない。
- Nagai（2010）の実用策「新しい先端節点が一定期間展開されなければ閾値を上げる」と、長井（2010）のヒューリスティクス（1）（2）（成・不成の重複と同種駒による取りの重複を1手に数える）は、中将棋の成り規則（第17条から第19条）と駒種に合わせて再定義すれば適用できる。

## 付録: 本調査で取得した一次資料

- 本文を読んだPDF: Allis 学位論文、Nagai & Imai IEICE 2002、長井・今井 IPSJ 2002、Kishimoto & Müller AAAI 2004 と Information Sciences 2005、岸本 GPW 2004、Kishimoto 学位論文 2005、Kishimoto & Müller CIG 2005、Kishimoto & Müller AAAI 2005、Kishimoto AAAI 2010、Kaneko AAAI 2010、金子ら GPW 2004、長井 GPW 2010、Winands ら PDS-PN 2003 と章 2008、Kishimoto ら ICGA 2012、Pawlewicz & Lew スライド、Kawano 1996。
- 書誌のみ確認: Allis ら AI 1994（DOI）、Seo ら AI 2001（DOI、書誌ページ）、Nagai 学位論文 2002、Nagai 1998、Nagai 修士論文 1999、Nagai & Imai GPW 1999、Ueda ら CG 2008、Kishimoto & Müller ACG 2003 と CG 2008、Seo 1999、金子ら GPW 2005 と IPSJ 2010。
- 読んだブログ: やねうら王 2024-05-08、komorinfo の2021年から2022年の記事8本と KomoringHeights refs.md、eukaryote 2020-05-03。
