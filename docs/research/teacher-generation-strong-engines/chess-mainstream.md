# チェスの主流エンジンにおける教師データの生成

本書は[強豪エンジンの教師生成と自己参照の偏りへの対策](../teacher-generation-strong-engines.md)の付録である。

ここでは、チェスの主流エンジンであるStockfishとLeela Chess Zero（以下lc0）が、評価関数の教師データをどう生成してきたかを一次資料で調べた結果である。
特に、自分の評価関数で作った教師から次の評価関数を学ぶ反復の中で、評価の誤りや偏りが強化されるのをどの仕組みで防いでいるかに重点を置いた。
参照日は2026年10月25日であり、調べた版は末尾の表にまとめた。
探索値と対局結果の混合比λとlc0の学習アルゴリズムの概要は、minaseのリポジトリの`docs/research/chess-evaluation-targets.md`と`docs/research/alphazero-prior-art.md`で扱われているので、本書では繰り返さずに参照する。
NNUE（efficiently updatable neural network）は着手ごとに第1層を差分更新できる小さなニューラルネットであり、WDLは勝ち・引き分け・負けの3分類を指す。

## 結論

Stockfishは、現在は自分で教師を生成して次のネットを学ぶ反復を行っていない。
2026年10月時点の採用ネットの学習手順は、2021年から2022年にStockfish自身が生成したデータと、2020年から2024年2月までのlc0の自己対局データを中心とする固定のデータ集を使い、その評価値をlc0の大型のtransformerネットBT4の価値出力で付け直したものを教師にする（[nettestのthreats.yaml 1〜44行](https://github.com/vondele/nettest/blob/ab168ab44a9957e96732a0e39d11852d592b0746/threats.yaml#L1-L44)）。
データ集にはこのほか、名前から終盤データベースの局面と推測される小さなファイルが1つ入っている。
付け直し前のデータを外して付け直し後のデータだけで学ぶ構成は、2026年7月2日の採用ネットで対局試験を通過した（[d91c7f6e](https://github.com/official-stockfish/Stockfish/commit/d91c7f6eac385faa68d12d46eaf91ea6909a2f52)、[nettest PR #403「relabel-only ablation」](https://github.com/vondele/nettest/pull/403)）。
Stockfish 19の公開文は、学習局面が「hundreds of billions」であり、すべてが強いlc0のネットで付け直されたと述べる（[edb0d9db](https://github.com/official-stockfish/Stockfish/commit/edb0d9db6731067ec50ce619ff372b463bc4dd5d)）。
minaseと同じく自己生成の反復でネットを更新していたのは2020年8月から2021年前半までであり、本書ではこの時期をminaseに最も近い先例として扱う。

Stockfishが教師の誤りに対して取った主な対策は、次の5つである。
第1は自分より強い外部の教師の導入であり、2021年6月にlc0の自己対局データを、2026年6月にBT4による付け直しを採用した。
第2は、対局結果を教師に混ぜる代わりに、評価値と対局結果が矛盾する局面を確率的に捨てる選別（以下WDL選別）である。開発者は2021年11月にこれを導入し、2022年2月の採用コミットで、WDL選別の下では結果を混ぜないλ=1.0のほうが良いと報告した（[cb9c2594](https://github.com/official-stockfish/Stockfish/commit/cb9c2594fcedc881ae8f8bfbfdf130cf89840e4c)）。
第3は、静的評価と深さ9の探索値の差が内部単位で100を超える局面を集めて開始局面集にし、そこから新しい教師を生成する方法である（[d53071ef](https://github.com/official-stockfish/Stockfish/commit/d53071eff4e75bc77dc86f65c52358d8014cb71c)）。
第4は、最善手が駒取りの局面、王手の局面、良い手が1つしかない局面の除外と、重複局面のうち後の世代の評価を残す重複除去である。
第5は、予測が教師より楽観的な場合に損失を重くする非対称な損失である。
いずれの採用も、データや学習率などの変更と同時に対局試験を通過した記録であり、単独の効果は分離されていない。

lc0は、探索で改善した方策と価値をネットへ戻す自己対局の反復を続けており、偏りへの対策は探索の外側に置いた仕組みの組合せである。
自己対局では温度と雑音で手を散らし、対局の20%では投了を無効にして最後まで指し、誤投了の有無を判定できる値を記録する（[学習サーバーの設定](https://training.lczero.org/training_runs)）。
学習の前には、終盤データベースで確定した勝敗で対局結果の教師を上書きし、温度で選ばれた悪手の前の局面の結果を探索値で補正する。
学習時には1局から1/32だけを抜き出し、探索前と探索後の評価の差が大きい局面を残すように抽出を偏らせる機能も持つ。
開発者は、この反復を探索で強化した自分自身を教師とする蒸留として説明し、2026年の論文では、教師に使う自己対局データの強さは、それを生んだネットの構造よりはるかに重要だと述べる（[Monroe et al. 2026 §5.1](https://arxiv.org/abs/2605.19091)）。

minaseへの含意は推論として最後の節にまとめた。
要点は、両プロジェクトで最も強く効いた補正は、現在の評価関数と独立な信号から来ていることである。
中将棋で使える独立な信号は、対局結果、詰み探索による証明、人間の棋譜の局面に限られる。
WDL選別と食い違い局面の採取は既存の教師と結果だけで試せるが、自己対局の双方が同じ癖を持つ場合、その癖は対局結果を変えないので、結果に基づく選別では検出できない可能性がある（推論）。

## 調査の範囲と方法

Stockfishについては、手元の複製で既定のネットを差し替えた119件のコミットのメッセージを全文読み、学習データと学習手順の記述を抜き出した。
そのうえで、教師生成器（`tools`ブランチの`generate_training_data`）、公式学習器nnue-pytorchの読み込み処理と選別処理、同学習器のwiki、採用ネットの学習手順を記録するnettestのレシピと履歴、およびデータ集のHugging Face上の構成を確認した。
lc0については、自己対局を実装するlc0本体、学習器lczero-training、配布クライアントとサーバー、公開されている学習サーバーの設定、公式ブログとwiki、および開発者を含む著者による2026年の論文を確認した。
コミットの勝敗数は手元の複製のメッセージから写した。
本書の数値はすべて資料の値をそのまま写し、換算した値には換算前の値を併記した。

## Stockfish

### 教師データの出どころの変遷

Stockfishの教師データは、自己生成、自己生成とlc0のデータの混合、lc0のデータの付け直しという順に移った。
次の表は、採用コミットのメッセージから出どころが読み取れる節目を並べたものである。

| 採用時期 | 教師の出どころ | 根拠 |
|---|---|---|
| 2020年8月 | 「millions of positions at moderate search depth」の評価で学んだネット。学習の道具はnodchipのリポジトリにあり、最初のネットはgekkehenkerとsergiovieriが提供した。 | [84f3e867](https://github.com/official-stockfish/Stockfish/commit/84f3e867903f62480c33243dd0ecbffd342796fc) |
| 2020年8月10日 | 主変化（PV）末端の静的評価ではなく探索値を教師にした最初のネット。 | [bcdf41da](https://github.com/official-stockfish/Stockfish/commit/bcdf41dadc8a5f8a23116236a0f449a08b46dc6b) |
| 2021年5月18日 | 当時の採用ネットで生成し評価した深さ8、9、10の640億局面（193 GB）。 | [e8d64af1](https://github.com/official-stockfish/Stockfish/commit/e8d64af1230fdac65bb0da246df3e7abe82e0838) |
| 2021年6月3日 | 前項に、静的評価と深さ9の探索値が大きく異なる局面から生成した20億局面を追加。 | [d53071ef](https://github.com/official-stockfish/Stockfish/commit/d53071eff4e75bc77dc86f65c52358d8014cb71c) |
| 2021年6月13日 | lc0のT60とT74の自己対局データだけで学んだ最初のネット。 | [f8c779db](https://github.com/official-stockfish/Stockfish/commit/f8c779dbe538315aa6f65556d0acf11640558504) |
| 2021年6月18日 | Stockfishの深さ9の自己対局160億局面とlc0の63億局面の混合（72%と28%）。 | [14b673d9](https://github.com/official-stockfish/Stockfish/commit/14b673d90f7f486becda4622eb1e8f4b63bb0785) |
| 2021年7月24日 | 採用ネットで約160億局面を生成し、採用ネットから継続して学ぶ強化学習の反復。 | [b939c805](https://github.com/official-stockfish/Stockfish/commit/b939c805139e4b37f04fbf177f580c35ebe9f130) |
| 2021年11月 | WDL選別の導入と、lc0のデータを終盤データベースで付け直したデータの追加。 | [9ee58dc7](https://github.com/official-stockfish/Stockfish/commit/9ee58dc7a75d7cdc09935f8782e927bb19700afe)、[e4a0c6c7](https://github.com/official-stockfish/Stockfish/commit/e4a0c6c75950bf27b6dc32490a1102499643126b) |
| 2022年2月 | 自己生成の`nodes5000pv2_UHO`で乱数から学び、lc0中心の混合データで再学習する2段階。 | [cb9c2594](https://github.com/official-stockfish/Stockfish/commit/cb9c2594fcedc881ae8f8bfbfdf130cf89840e4c) |
| 2022年7月 | 二重フィッシャーランダムチェス（DFRC）の開始局面から生成したStockfishのデータを追加。 | [85f8ee61](https://github.com/official-stockfish/Stockfish/commit/85f8ee6199f8578fbc082fc0f37e1985813e637a) |
| 2022年12月〜2023年9月 | lc0のT77〜T80のデータを7駒の終盤データベースで付け直し、Stockfishの浅い探索で選別し、重複を除いて追加。 | [c6208861](https://github.com/official-stockfish/Stockfish/commit/c620886181b5c0fb4a0c067b4173143a81819199)、[3d2381d7](https://github.com/official-stockfish/Stockfish/commit/3d2381d76d7bf9686ef0e0671f60c3b885a7058a)、[c3ce2204](https://github.com/official-stockfish/Stockfish/commit/c3ce2204083400267592dc088b8ad9e88aed56b1) |
| 2026年6月25日〜29日 | BT4の価値出力で付け直した20億局面、35億局面、740億局面を順に微調整段へ追加。 | [9fcd47a7](https://github.com/official-stockfish/Stockfish/commit/9fcd47a717cfdf8c44127710fdb3325218a12259)、[9d2ba2be](https://github.com/official-stockfish/Stockfish/commit/9d2ba2bed507057425559aaefc5e1a14441e8d6d)、[4488343c](https://github.com/official-stockfish/Stockfish/commit/4488343cf67b39b68c8dc41c91f26796b98df673) |
| 2026年7月2日以降 | 付け直し後のデータだけで学ぶ構成。現行のレシピもこの構成である。 | [d91c7f6e](https://github.com/official-stockfish/Stockfish/commit/d91c7f6eac385faa68d12d46eaf91ea6909a2f52) |

現行のレシピのデータ集に入っているStockfish自身のデータは、`nodes5000pv2_UHO`、`wrongIsRight_nodes5000pv2`、`multinet_pv-2_diff-100_nodes-5000`、`dfrc_n5000`の4つであり、いずれもBT4で付け直した版である（[threats.yaml 7〜10行](https://github.com/vondele/nettest/blob/ab168ab44a9957e96732a0e39d11852d592b0746/threats.yaml#L7-L10)）。
これらの名前が採用コミットに初めて現れたのは2021年11月から2023年6月であり、その後にStockfishが新しい自己対局データを生成して採用ネットに使った記録は、今回読んだコミットには見当たらなかった。
現行のレシピに入るlc0のデータで最も新しいものは2024年2月の`test80`であり、2026年7月以前のレシピには2024年6月までの`test80`も入っていた（[threats.yaml 42〜43行](https://github.com/vondele/nettest/blob/ab168ab44a9957e96732a0e39d11852d592b0746/threats.yaml#L42-L43)、[nettest 1009f26の差分](https://github.com/vondele/nettest/commit/1009f26c4c56a2ec482e12e2a20a5958c2bc544c)）。
したがって2024年後半以降のStockfishのネットの改善は、新しい教師の生成ではなく、構造、学習器、学習時間、および既存データの付け直しによるものと考えられる（推論）。

### 自己生成期の生成器

Stockfishの教師生成器は`tools`ブランチの`generate_training_data`であり、自己対局を指しながら各局面の探索値を記録する。
生成器の各局面では、指定の深さまたはノード数で探索し、その探索値と最善手を局面とともに書き出す（[training_data_generator.cpp 322〜375行](https://github.com/official-stockfish/Stockfish/blob/9a4c7cf4e311f8d9526b79295b80c4d0464c07cf/src/tools/training_data_generator.cpp#L322-L375)）。
対局結果は対局の終了後に、書き出したすべての局面へ手番側から見た値として付ける（同672〜697行）。
局面の重複を防ぐため、局面のハッシュ値の表に記録済みの局面は書き出さない（同392〜412行）。
序盤は既定で16手目より前を書き出さず、ソースのコメントは「Initial positions would be too common」と理由を述べる（同362〜365行）。

対局途中のランダム化は2種類ある。
既定では、1手目から24手目のうち無作為に選んだ5手で合法手を一様に選ぶ（同54〜60行、`random_move_min_ply`、`random_move_max_ply`、`random_move_count`）。
`random_multi_pv`を指定すると、一様な選択の代わりに複数の最善候補を探索し、最善手から`random_multi_pv_diff`以内の候補から1つを選ぶ（同639〜660行）。
実際の生成では`random_multi_pv_diff 100`がよく使われており、ランダムな手を、最善手との探索値の差がエンジン内部の単位で100以内の候補に限っていた（推論の根拠は次の表のコマンド）。

勝敗の裁定は次のとおりである。
探索値の絶対値が`eval_limit`以上になった場合、対局の80%では4手連続で超えたときか`VALUE_KNOWN_WIN`に達したときに、残り20%では`VALUE_KNOWN_WIN`に達したときだけに、探索値の符号で勝敗を決める（同297〜349行）。
この80%は`prng.rand(10) > 1`による選択であり、`rand(10)`は0から9を返す（[misc.h 455行](https://github.com/official-stockfish/Stockfish/blob/9a4c7cf4e311f8d9526b79295b80c4d0464c07cf/src/misc.h#L455)）。
文書は`eval_limit`を超えた局面を書き出さないと説明するが、コードは連続回数が4に満たない間はその局面を書き出す（[generate_training_data.md](https://github.com/official-stockfish/Stockfish/blob/9a4c7cf4e311f8d9526b79295b80c4d0464c07cf/docs/generate_training_data.md)）。
引き分けは、50手規則、3回反復、400手到達、詰め手のない駒不足、および80手目以降に評価値0が8手連続した場合に裁定する（同417〜520行）。
lc0と異なり、投了しなかった20%の対局から誤投了の率を集計する仕組みは生成器にない。

採用コミットとwikiに残る実際の生成コマンドは次のとおりである。

| データ | 生成者と時期 | 主な設定 | 根拠 |
|---|---|---|---|
| 640億局面 | 2021年5月以前 | 深さ8、9、10。当時の採用ネットで生成し評価。 | [e8d64af1](https://github.com/official-stockfish/Stockfish/commit/e8d64af1230fdac65bb0da246df3e7abe82e0838) |
| 食い違い局面集からの20億局面 | vondele、2021年6月 | `depth 9 random_multi_pv 2 random_multi_pv_diff 100 random_move_max_ply 8 random_move_count 3 eval_limit 32000 book wrongNNUE.epd` | [d53071ef](https://github.com/official-stockfish/Stockfish/commit/d53071eff4e75bc77dc86f65c52358d8014cb71c) |
| 強化学習の反復 | Sergio Vieri、2021年7月 | `min_depth 9 max_depth 11 random_move_count 10 random_move_max_ply 80 random_multi_pv 12 random_multi_pv_diff 100 random_multi_pv_depth 8 write_min_ply 10 eval_limit 1500 book noob_3moves.epd`。最新の採用ネットで生成を繰り返す。 | [b939c805](https://github.com/official-stockfish/Stockfish/commit/b939c805139e4b37f04fbf177f580c35ebe9f130)、[gen.shとexec.sh](https://github.com/sergiovieri/Stockfish/tree/6b0838b1404e91aafff17c4f2ac0a8ced999a307/rl) |
| 160億局面 | vondele、2021年9月 | `depth 9 random_multi_pv 4 random_multi_pv_diff 100 book noob_3moves.epd`。wikiは「depth 9 still seems to be the sweet spot」と記す。 | [wikiのTraining datasets](https://github.com/official-stockfish/nnue-pytorch/wiki/Training-datasets) |
| `nodes5000pv2_UHO` | 2021年11月以前 | 深さの代わりに1手5,000ノードで生成した。wikiはUHOの開始局面集での強さが最も良いと記す。 | 同上 |

Sergio Vieriの反復は、生成スクリプトが常に`latest.nnue`を読み込み、`exec.sh`が生成と検証用データの生成を無限に繰り返す構成である（[exec.sh](https://github.com/sergiovieri/Stockfish/blob/6b0838b1404e91aafff17c4f2ac0a8ced999a307/rl/exec.sh)）。
終盤データベースを探索に使う版の生成スクリプトもある（[gentb.sh](https://github.com/sergiovieri/Stockfish/blob/6b0838b1404e91aafff17c4f2ac0a8ced999a307/rl/gentb.sh)）。
この反復で作ったネットは、採用ネットから継続して`lambda=0.2`で学んだと記録されている（[b939c805](https://github.com/official-stockfish/Stockfish/commit/b939c805139e4b37f04fbf177f580c35ebe9f130)）。
当時の学習器の損失は`lambda * loss_eval + (1 - lambda) * loss_result`なので、これは評価値20%、対局結果80%の重みである（[nnue-pytorch 3c6d94e、model.py 274〜276行](https://github.com/official-stockfish/nnue-pytorch/blob/3c6d94ef277ed853a07da3045de966302a6340a6/model.py#L274-L276)）。
採用されたネットはさらに、そのネットから別の混合データで`lambda=1.0`の再学習を加えたものであり、自己生成の反復だけの効果は分離されていない。

UHOはStefan Pohlの開始局面集の名称であり、通称はUnbalanced Human Openingsとされるが、本調査では作者の説明を確認できなかった。
ファイル名は`UHO_XXL_+0.90_+1.19`のように評価値の範囲を含み、偏りのある開始局面を集めたものと読める（推論）。
UHOは生成用の開始局面集として`nodes5000pv2_UHO`に使われたほか、対局試験の開始局面集として使われている。
2026年のWDLモデルの更新はUHO_Lichess_4852の局面集を使った対局から当てはめており、nettestの対局試験もこの局面集を取得する（[3f6f417b](https://github.com/official-stockfish/Stockfish/commit/3f6f417b87c0e80ee30914b6b539b4ab7d3b2a5b)、[nettest/test.py 324〜339行](https://github.com/vondele/nettest/blob/ab168ab44a9957e96732a0e39d11852d592b0746/nettest/test.py#L324-L339)）。
UHOの局面の選定基準は、作者のページを取得できなかったため確認していない。

### 誤りを狙った局面の採取

自己生成期の対策として最も明確なのは、評価関数が誤っていそうな局面を探して、そこから教師を生成する方法である。
vondeleは2021年6月1日に、探索中に次の条件をすべて満たす局面を書き出す変更を作った（[vondele/Stockfish 3ce43ab0](https://github.com/vondele/Stockfish/commit/3ce43ab0c4ce09c1fc5bca5ca27a248e67fddd24)）。
条件は、深さ9のPVノードで探索値が窓の内側にあること、静的評価と探索値の差が100を超えること、どちらか一方の絶対値が100未満であること、王手でないこと、および最善手が駒取りや成りでないことである。
こうして集めた局面を開始局面集`wrongNNUE.epd`とし、前節の表のコマンドで20億局面を生成して既存の640億局面に加えたネットが採用された（[d53071ef](https://github.com/official-stockfish/Stockfish/commit/d53071eff4e75bc77dc86f65c52358d8014cb71c)）。
その後も同じ系統の`Wrong_NNUE_2`、`wrongIsRight_nodes5000pv2`、`WrongIsRight-Reloaded`がデータ集に入っている。
採用コミットのメッセージは、`Wrong_NNUE_2`を他のデータとほぼ同じ重みにするために分量をそろえて交互に並べたと記す（[adfb23c0](https://github.com/official-stockfish/Stockfish/commit/adfb23c029e54c7522aadca1adf3e0b15fdcebcd)）。
`wrongIsRight_nodes5000pv2`の生成コマンドは確認できず、名前から同じ考え方を1手5,000ノードで行ったものと推測するにとどまる（推論）。

### 学習時の局面選別

公式学習器nnue-pytorchは、教師データを読み込む段階で局面を選別する。
以下の行番号は、現行のレシピが固定する学習器のコミット140fd799による（[threats.yaml 101〜103行](https://github.com/vondele/nettest/blob/ab168ab44a9957e96732a0e39d11852d592b0746/threats.yaml#L101-L103)）。

駒取りと王手の除外は、旧名`--smart-fen-skipping`、現行名`filtered`の機能であり、記録された手が駒取りである局面と、手番側が王手を受けている局面を捨てる（[training_data_loader.cpp 1044行](https://github.com/official-stockfish/nnue-pytorch/blob/140fd799ef4c63576833c15a7fe52ab88ba411b6/data_loader/cpp/training_data_loader.cpp#L1044)）。
この機能は2020年11月に、nodchipの学習器にあった同名の機能に倣って追加された（[nnue-pytorch 7d91d6c](https://github.com/official-stockfish/nnue-pytorch/commit/7d91d6c6489975f0fcf3dbf969dcf24fa51e6f72)）。
現行の既定値は有効であり、レシピも無効にしていない（[config.py 7行](https://github.com/official-stockfish/nnue-pytorch/blob/140fd799ef4c63576833c15a7fe52ab88ba411b6/data_loader/config.py#L7)）。
2024年3月の採用ネットでは、静的駒交換評価SEEが負になる駒取りは捨てずに学習に残した（[bd579ab5](https://github.com/official-stockfish/Stockfish/commit/bd579ab5d1a931a09a62f2ed33b5149ada7bc65f)）。

無作為の間引き`--random-fen-skipping N`は、各局面を確率N/(N+1)で捨てる（[training_data_loader.cpp 910〜911行](https://github.com/official-stockfish/nnue-pytorch/blob/140fd799ef4c63576833c15a7fe52ab88ba411b6/data_loader/cpp/training_data_loader.cpp#L910-L911)）。
採用ネットで使われた値は3、10、12、14、500であり、残る割合に直すとそれぞれ1/4、1/11、1/13、1/15、1/501である。
現行のレシピは3を使う（[threats.yaml 68行](https://github.com/vondele/nettest/blob/ab168ab44a9957e96732a0e39d11852d592b0746/threats.yaml#L68)）。
Stockfish側の資料に間引きの目的の説明は見当たらなかったが、lc0の学習器は同種の間引きを、同じ対局の連続した局面が強く相関するのを避けるためと説明する（[lczero-training chunkparser.py 34〜35行](https://github.com/LeelaChessZero/lczero-training/blob/ee626a97e517df76e85084339bbe6d0895eaba09/tf/chunkparser.py#L34-L35)）。

序盤の除外`--early-fen-skipping N`は、N手目以下の局面を捨てる（同1037行）。
2022年12月に学習データ全体で28手目以下を捨て始め、現行のレシピは18手目以下を捨てたうえで、32手目未満の局面を手数に応じた確率で捨てる（[be9bc420](https://github.com/official-stockfish/Stockfish/commit/be9bc420afa11314c886c0aede4c4ae3d76f8a50)、[threats.yaml 69〜83行](https://github.com/vondele/nettest/blob/ab168ab44a9957e96732a0e39d11852d592b0746/threats.yaml#L69-L83)）。
さらに、盤上の駒数の分布が目標の分布に近づくように局面を捨てる（同1084行付近、[導入コミットd9c577b](https://github.com/official-stockfish/nnue-pytorch/commit/d9c577b5973c33a535cb57627853ad70cc5bc836)）。
2023年12月の採用コミットは、データの駒数分布が16枚未満の局面に大きく偏っていることを問題として挙げている（[f12035c8](https://github.com/official-stockfish/Stockfish/commit/f12035c88c58a5fd568d26cde9868f73a8d7b839)）。

WDL選別は、教師の評価値から予測される勝ち、引き分け、負けの確率のうち、実際の対局結果に当たる確率をpとし、その局面を確率1−pで捨てる（[training_data_loader.cpp 1047〜1051行](https://github.com/official-stockfish/nnue-pytorch/blob/140fd799ef4c63576833c15a7fe52ab88ba411b6/data_loader/cpp/training_data_loader.cpp#L1047-L1051)）。
pを計算する勝率モデルは、fishtestの長時間対局の統計に当てはめた3次多項式とロジスティック関数であり、学習用にその幅の係数を1.5倍している（[training_data_entry.h 68〜103行](https://github.com/official-stockfish/nnue-pytorch/blob/140fd799ef4c63576833c15a7fe52ab88ba411b6/data_loader/cpp/lib/training_data_entry.h#L68-L103)）。
導入したvondeleのコミットは、目的を「skip (or more correctly weight) data points that are possibly incorrectly evaluated, i.e. retain data that are more likely to be correct」と書く（[nnue-pytorch b7dd8d3、2021年11月20日](https://github.com/official-stockfish/nnue-pytorch/commit/b7dd8d3e53a9ea36e8241f19ec0abf1d28af4391)）。
最初の採用ネットのメッセージも、誤って評価された局面から学ぶ可能性を下げるはずだと説明する（[9ee58dc7](https://github.com/official-stockfish/Stockfish/commit/9ee58dc7a75d7cdc09935f8782e927bb19700afe)）。
重要なのは、この時期にはこの選別がλによる混合の代わりに使われたことである。
2022年2月の採用コミットは、「we now use lambda=1.0 instead of lambda=0.8 (previous nets), because tests show that WDL-skipping introduced by vondele performs better with lambda=1.0」と記す（[cb9c2594](https://github.com/official-stockfish/Stockfish/commit/cb9c2594fcedc881ae8f8bfbfdf130cf89840e4c)）。
対局結果を教師値へ混ぜるのではなく、結果と矛盾する教師を捨てるために使うという使い分けであり、既存の調査が扱ったλの値とは別の結果の使い方である。
その後の採用ネットは再び結果を混ぜる設定を使っており、2022年5月以降はλを学習中に1.0から0.75や0.7へ下げる設定とWDL選別を併用した（[c079acc2](https://github.com/official-stockfish/Stockfish/commit/c079acc26f93acc2eda08c7218c60559854f52f0)）。
現行のレシピもWDL選別を無効にしておらず、λの扱いはminaseの`docs/research/chess-evaluation-targets.md`のとおりである。

WDL選別は、外部のデータが持つ偏りへの対策にも使われた。
2023年12月の採用コミットは、選別を強めた理由として、Stockfishが終盤の評価の誤りで指し損なうという報告が繰り返し寄せられたことを挙げ、その原因がlc0の終盤の弱さが教師データに反映されたことにある可能性を述べる（[f12035c8](https://github.com/official-stockfish/Stockfish/commit/f12035c88c58a5fd568d26cde9868f73a8d7b839)）。
原文は「More WDL skipping is meant to increase lambda accuracy and train on fewer misevaluated positions where position scores are unlikely to correlate with game outcomes」である。
このとき使った学習器の分岐では、捨てる確率を1−pから1−p²へ強めていた（[linrock/nnue-pytorchのd3c8b3f](https://github.com/linrock/nnue-pytorch/commit/d3c8b3fb8f9e85c0ac518c10e1651a1a1e2422c1)）。
公式の学習器は現在も1−pを使う。

このほか、容量を減らした形式の学習データで、評価値の代わりに0を仮置きした局面を推定して捨てる処理がある（同1018〜1030行）。

### 外部データの前処理

lc0のデータをStockfishの形式へ変換する際には、lc0の付け直しツール（rescorer）で終盤データベースによる対局結果の付け直しを行う。
nnue-pytorchのwikiは、borgが最初に使ったスクリプトとして、`rescorer rescore --syzygy-paths=../syzygy --nnue-plain-file=...`の呼び出しを載せる（[Training datasets](https://github.com/official-stockfish/nnue-pytorch/wiki/Training-datasets)）。
変換時の評価値は、lc0の探索後の最善手のQを`660.6·q/(1−0.9751875·q^10)`で換算したものであり、Stockfish自身の探索値ではない（[lc0 rescorer.cc 444〜455行、1113行](https://github.com/LeelaChessZero/lc0/blob/58e780090b8ffb765743b73a794f994c3a8c9476/src/trainingdata/rescorer.cc#L444-L455)、既定値は同1317行）。

2022年12月から2023年4月にかけて、linrockはlc0のデータと既存の混合データに3段の前処理を加えた。
第1は、Stockfishの深さ6で上位2手を求める探索（MultiPV 2）で、最善手または次善手が駒取りの局面を除く選別であり、最初はStockfishのDFRCデータを含む既存の混合データ`Leela-dfrc_n5000`に適用した（[c6208861](https://github.com/official-stockfish/Stockfish/commit/c620886181b5c0fb4a0c067b4173143a81819199)）。
第2は、同じ探索で良い手が1つしかない局面を除く選別であり、最善手が大きな優勢に至るのに次善手が互角以下になる場合と、最善手が互角なのに次善手が負けになる場合を、良い手が1つの局面と定義した（[3d2381d7](https://github.com/official-stockfish/Stockfish/commit/3d2381d76d7bf9686ef0e0671f60c3b885a7058a)）。
第3は重複除去であり、lc0の学習が進んだ後の時期のファイルから順に読み、最初に現れた局面だけを残した（[c3ce2204](https://github.com/official-stockfish/Stockfish/commit/c3ce2204083400267592dc088b8ad9e88aed56b1)）。
その理由は「This ordering was done with the assumption that position scores seen later in time are generally more accurate than scores seen earlier in the test run」と書かれている。
これらの前処理と終盤データベースの付け直しを加えた採用ネットは、それぞれデータの追加や学習の延長と同時に対局試験を通過している。

### 教師値

教師の評価値は、自己生成期にはStockfish自身の探索値、混合期にはStockfishの探索値とlc0の探索後のQ、2026年7月以降はBT4の価値出力である。
BT4による付け直しは、局面ごとにBT4の評価を1回だけ求め、探索は行わない。
付け直しツールは、各局面の評価値を価値出力のQから前節と同じ式で換算した値に置き換え、手、手数、対局結果はそのまま残す（[xu-shawn/lc0 3f83fab、main.cc 26〜33行、120〜165行](https://github.com/xu-shawn/lc0/blob/3f83fab871d641e8aaed8a55481f3b7487977606/src/main.cc#L26-L33)）。
したがってWDL選別とλによる混合が参照する対局結果は、元のlc0またはStockfishの自己対局の結果であり、lc0のデータでは終盤データベースで付け直した結果である。
最初の付け直しの採用コミットは、20億局面は全学習データのごく一部であり、付け直しを増やせばさらに伸びると予想している（[9fcd47a7](https://github.com/official-stockfish/Stockfish/commit/9fcd47a717cfdf8c44127710fdb3325218a12259)）。
現行のデータ集には`tb5dtm.binpack`も入っており、名前からは5駒の終盤データベースの局面に詰みまでの手数に基づく評価を付けたものと推測されるが、生成方法は確認できなかった（[threats.yaml 44行](https://github.com/vondele/nettest/blob/ab168ab44a9957e96732a0e39d11852d592b0746/threats.yaml#L44)）。
このファイルは2026年7月2日にレシピへ入り、翌日に外され、7月26日に再び入った。

損失関数には、予測が教師より高い場合に損失を割り増す非対称性がある（[model/nnue.py 75〜76行](https://github.com/official-stockfish/nnue-pytorch/blob/140fd799ef4c63576833c15a7fe52ab88ba411b6/model/nnue.py#L75-L76)）。
割増しは2024年1月の10%、2月の15%、5月の20%と増え、現行のレシピは`qp-asymmetry: 0.22866886710086187`を使う（[6deb8872](https://github.com/official-stockfish/Stockfish/commit/6deb88728fb141e853243c2873ad0cda4dd19320)、[8e75548f](https://github.com/official-stockfish/Stockfish/commit/8e75548f2a10969c1c9211056999efbcebe63f9a)、[1b7dea3f](https://github.com/official-stockfish/Stockfish/commit/1b7dea3f851cd5c5411ba6f07a2f935bfb7da8a9)、[threats.yaml 85行](https://github.com/vondele/nettest/blob/ab168ab44a9957e96732a0e39d11852d592b0746/threats.yaml#L85)）。
採用コミットは、Tomasz Sobczykの実験記録にあるvondeleの着想の変形だと説明するが、楽観側を重く罰する理由は書いていない。

### データ量

自己生成期の量は、2021年5月の640億局面（193 GB）、2021年6月の追加20億局面、2021年7月のSergio Vieriの約160億局面、2021年9月のvondeleの160億局面である。
混合期の例として、2021年6月のネットはStockfishの160億局面とlc0の63億局面を混ぜた。
Stockfish 18の公開文は、学習が1,000億を超えるlc0の評価データを使うと述べる（[cb3d4ee9](https://github.com/official-stockfish/Stockfish/commit/cb3d4ee9b47d0c5aae855b12379378ea1439675c)）。
BT4による付け直しは、20億局面、35億局面（うち選別後に残るのは29億局面）、740億局面の順に追加され、合計は換算で約795億局面である。
nettestのREADMEは、現行の学習に約700 GBの記憶領域と、高性能なGPU 1台で4〜5日を要すると記す（[README](https://github.com/vondele/nettest/blob/ab168ab44a9957e96732a0e39d11852d592b0746/README.md)）。
レシピのエポックの大きさ`epoch-size`は、2026年10月7日に134,217,728局面から1,073,741,824局面へ変わったので、過去の採用記録のエポック数と現行のエポック数は直接比べられない（[nettest PR #497](https://github.com/vondele/nettest/pull/497)）。

### 世代の回し方

ネットを前のネットから継続して学ぶか、乱数から学ぶかは、時期によって変わった。
2021年には、前の採用ネットを`--resume-from-model`で初期値にして再学習する採用が多い（[adfb23c0](https://github.com/official-stockfish/Stockfish/commit/adfb23c029e54c7522aadca1adf3e0b15fdcebcd)ほか）。
2022年2月の構造変更では、乱数から自己生成データで400エポック学び、その最良のネットからlc0中心のデータで800エポック再学習する2段階を採った（[cb9c2594](https://github.com/official-stockfish/Stockfish/commit/cb9c2594fcedc881ae8f8bfbfdf130cf89840e4c)）。
このコミットは第1段の目的を「to establish a strong network base for the second session」と述べる。
2023年の構造変更では、乱数からの学習と再学習を4〜6段つなぎ、各段で強いエポックを選んで次の段の初期値にした（[c1fff716](https://github.com/official-stockfish/Stockfish/commit/c1fff71650e2f8bf5a2d63bdc043161cdfe8e460)、[70ba9de8](https://github.com/official-stockfish/Stockfish/commit/70ba9de85cddc5460b1ec53e0a99bee271e26ece)）。
現行のレシピは、乱数から付け直し後の全データで560エポック学ぶ段と、そのチェックポイントから600エポックまで続ける段の2段である（[threats.yaml 164〜187行](https://github.com/vondele/nettest/blob/ab168ab44a9957e96732a0e39d11852d592b0746/threats.yaml#L164-L187)）。
古いデータは捨てずに蓄積され、毎回の学習は同じ固定のデータ集を使う。
nnue-pytorchのwikiは2021年12月6日の記述として、まずStockfishの生成データで学び、それからlc0のデータで再学習するのが通常は最良であり、lc0のデータだけでは局面の網羅性か概念の難しさのために良い結果にならないと書いた（[Training datasets 103行](https://github.com/official-stockfish/nnue-pytorch/wiki/Training-datasets)）。
現行のレシピは全データを1つの集合にまとめているので、この順序の助言は現在の手順には反映されていない。

### 開発者の説明

Stockfishの開発者がlc0のデータを使う理由として書いた一次資料は、次の4つである。
第1に、nnue-pytorchのwikiは、良いデータ集の条件は「We don't really know」としたうえで、より「positional」な評価を持つデータが良いのかもしれず、評価が良ければ常に良い結果になるわけではなく、品質と学習しやすさの間に兼ね合いがあり、データ集は経験的に選ぶと書く（[Training datasets 5行](https://github.com/official-stockfish/nnue-pytorch/wiki/Training-datasets)）。
同じページは、lc0のデータから変換したデータ集はStockfishで生成したものより品質が高く、一般により良いネットになると書く（同39行）。
これらの記述は2021年12月6日にTomasz Sobczykが書き、2026年10月の改訂後も残っている。
第2に、Stockfish 14の公開文は、lc0の評価した数十億局面とStockfishの評価した数十億局面を組み合わせて自由に使えたことが進歩に不可欠だったと述べる（[773dff02](https://github.com/official-stockfish/Stockfish/commit/773dff020968f7a6f590cfd53e8fd89f12e15e36)）。
第3に、前節で述べたとおり、2023年12月の採用コミットは外部の教師の弱点も教師データを通じて伝わると認めている。
第4に、2026年6月の付け直しの採用コミットは、付け直したデータを増やすほど伸びると予想した。
Stockfish自身の生成データが自己参照の偏りを持つという趣旨の記述は、今回読んだ資料には見当たらなかった。

### 対局による測定記録

次の表は、教師データに関わる変更を含む採用の記録である。
いずれも同時に別の変更を含むので、表の結果を教師の変更だけの効果とは読めない。
STCは10秒＋0.1秒、LTCは60秒＋0.6秒の対局であり、LLRは逐次確率比検定の対数尤度比である。
勝敗数はコミットのメッセージから写した。

| 変更 | 条件 | 勝ち | 負け | 引き分け | 判定 | 同時の変更 |
|---|---|---:|---:|---:|---|---|
| 食い違い局面からの20億局面の追加（[d53071ef](https://github.com/official-stockfish/Stockfish/commit/d53071eff4e75bc77dc86f65c52358d8014cb71c)） | STC | 6,254 | 6,028 | 52,310 | 通過（LLR 2.94） | 学習のやり直し |
| 同上 | LTC | 1,998 | 1,826 | 41,736 | 通過（LLR 2.96） | 同上 |
| lc0のデータだけによる最初のネット（[f8c779db](https://github.com/official-stockfish/Stockfish/commit/f8c779dbe538315aa6f65556d0acf11640558504)） | STC 1万局 | 1,241 | 1,137 | 7,622 | Elo +3.61±3.3 | 間引き率 |
| 同上 | LTC | 685 | 545 | 8,418 | 通過（LLR 2.93） | 同上 |
| WDL選別の導入（[9ee58dc7](https://github.com/official-stockfish/Stockfish/commit/9ee58dc7a75d7cdc09935f8782e927bb19700afe)） | STC 4万局 | 10,447 | 10,498 | 19,055 | Elo −0.44±1.8 | データの追加 |
| 同上 | LTC | 7,663 | 7,396 | 14,317 | 通過（LLR 2.94） | 同上 |
| WDL選別の強化（[f12035c8](https://github.com/official-stockfish/Stockfish/commit/f12035c88c58a5fd568d26cde9868f73a8d7b839)） | LTC | 66,378 | 65,574 | 133,374 | 通過（LLR 2.94） | 最適化手法の変更 |
| BT4による付け直しの微調整（[9fcd47a7](https://github.com/official-stockfish/Stockfish/commit/9fcd47a717cfdf8c44127710fdb3325218a12259)） | STC | 18,891 | 18,784 | 34,549 | 途中（LLR −0.01） | 微調整段の追加そのもの |
| 同上 | LTC | 6,566 | 6,288 | 12,256 | 通過（LLR 2.94） | 同上 |
| 付け直し後のデータだけで学習（[d91c7f6e](https://github.com/official-stockfish/Stockfish/commit/d91c7f6eac385faa68d12d46eaf91ea6909a2f52)） | STC | 7,185 | 6,883 | 12,908 | 通過（LLR 2.93） | 評価値の換算の桁あふれの修正 |
| 同上 | LTC | 26,460 | 26,009 | 48,925 | 通過（LLR 2.95） | 同上 |

WDL選別の導入は短時間では中立で、長時間で有意に勝った。
BT4による付け直しも、短時間の試験は途中で中立に近く、長時間とさらに長い時間の試験で通過した。
教師の質の改善が短時間より長時間の対局で現れるという傾向が2件で一致するが、2件だけなので一般化はできない（推論）。

### 初代のネット

Stockfishの最初の教師は、従来の手作り評価関数を使った生成器で作られた。
nodchipが2020年5月30日に公開した実行ファイル群には「training data generator with the original Stockfish evaluation function」が含まれる（[nodchip/Stockfishのリリース](https://github.com/nodchip/Stockfish/releases/tag/stockfish-nnue-2020-05-30)）。
翌日の手順の転載は、最初の反復では元の評価関数で深さ8の教師を1,000万局面作り、2回目以降はNNUEの実行ファイルで生成し、生成と学習を繰り返せると説明する（[talkchessへのRaphexonの投稿、2020年5月31日](https://talkchess.com/viewtopic.php?p=845295)）。
これは転載であり、nodchip本人の文書は確認できなかった。
2020年8月の最初の採用ネットを作ったgekkehenkerとsergiovieriが、どのデータを使ったかは確認できなかった。
2021年5月の構造変更では、駒の損得を表す線形の経路（PSQT）の初期値を従来の駒価値にした（[e8d64af1](https://github.com/official-stockfish/Stockfish/commit/e8d64af1230fdac65bb0da246df3e7abe82e0838)）。
2020年9月以降には、乱数から学び直したネットの採用もある（[9cc482c7](https://github.com/official-stockfish/Stockfish/commit/9cc482c7889cbc5f6d92e1b69ccd28d422a44a32)）。

## Leela Chess Zero

### 自己対局の設定

lc0の自己対局の設定は、学習サーバーが配布クライアントへ渡す引数で決まり、その値は学習サーバーの公開ページに載っている（[training_runs](https://training.lczero.org/training_runs)）。
2026年10月25日に参照した時点では3つの学習系列が載っており、主系列のT80（test80）は「Main run started April 4th 2022 (Partially pre-trained)」と説明され、現在は停止している。
test91とtest90は、BT4を強化学習で学ぶための試験系列と説明されており、稼働中なのはtest91だけである。
T80の学習用の主な引数は次のとおりである。

| 項目 | T80の値 | 意味 |
|---|---|---|
| 探索量 | `--visits=10000`、`--minimum-kldgain-per-node=0.000050` | 1手の上限は10,000訪問であり、方策の変化が小さくなると早く打ち切る。 |
| 温度 | `--temperature=0.9`、`--tempdecay-delay-moves=20`、`--tempdecay-moves=60`、`--temp-cutoff-move=40`、`--temp-endgame=0.60`、`--temp-visit-offset=-0.8` | 訪問回数を温度で調整した確率で手を選ぶ。初期温度0.9を20手後から60手かけて下げ、40手目以降は0.6を使う。 |
| 雑音 | `--noise-epsilon=0.1`、`--noise-alpha=0.12` | 根の事前確率にディリクレ雑音を10%混ぜる。 |
| 投了 | `--resign-percentage=2.0`、`--resign-wdlstyle=true`、`--resign-playthrough=20` | 勝ち、負け、引き分けのいずれかの確率が98%を超えたら裁定し、20%の対局では投了を無効にする。 |
| 開始局面 | `--openings-pgn=books/run2_book.pgn.gz`、`--openings-mode=shuffled` | 開始局面集を無作為な順で使う。 |

各引数の意味はlc0のソースの説明による（[search/classic/params.cc 226〜290行](https://github.com/LeelaChessZero/lc0/blob/58e780090b8ffb765743b73a794f994c3a8c9476/src/search/classic/params.cc#L226-L290)）。
温度の組合せの解釈は引数の説明から組み立てたものであり、各手の温度の正確な値は確認していない（推論）。
自己対局の既定では温度1.0、雑音の混合率0.25が設定されるが、学習系列ではサーバーの引数が上書きする（[selfplay/tournament.cc 154〜170行](https://github.com/LeelaChessZero/lc0/blob/58e780090b8ffb765743b73a794f994c3a8c9476/src/selfplay/tournament.cc#L154-L170)）。
1手の平均の訪問数は、上限とKullback–Leibler情報量（KLD）による打ち切りの組合せで決まり、実際の平均は確認していない。
lc0のwikiは、KLDによる打ち切りを「This allows selfplay to spend more time when the Policy prediction was bad, and less time on obvious moves」と説明する（[Technical explanation](https://lczero.org/dev/wiki/technical-explanation-of-leela-chess-zero/)）。
選んだ手の訪問数が少なすぎる場合に選び直す`--minimum-allowed-visits`はT80の引数になく、既定値0では無効である（[selfplay/game.cc 51〜55行、75行](https://github.com/LeelaChessZero/lc0/blob/58e780090b8ffb765743b73a794f994c3a8c9476/src/selfplay/game.cc#L51-L75)）。

探索のばらつきに関する開発者の比較として、2019年の公式ブログは、終盤で温度を0にしたtest53が、温度を残したtest52より「clearly weaker」だったと報告する（[What's going on with training, 2019年6月16日](https://lczero.org/blog/2019/06/whats-going-on-with-training/)）。

### 投了と誤投了の確認

投了は手の探索値で判定し、WDL形式では勝ち、負け、引き分けのいずれかの確率が`1 − 投了率`を超えたときにその結果で対局を終える（[selfplay/game.cc 267〜296行](https://github.com/LeelaChessZero/lc0/blob/58e780090b8ffb765743b73a794f994c3a8c9476/src/selfplay/game.cc#L267-L296)）。
投了による裁定は学習データに印として記録される（[trainingdata.cc 102〜103行](https://github.com/LeelaChessZero/lc0/blob/58e780090b8ffb765743b73a794f994c3a8c9476/src/trainingdata/trainingdata.cc#L102-L103)）。
`--resign-playthrough`で選ばれた対局では投了を無効にし、終局後に、最終的な勝者または引き分けの側が対局中に示した最悪の評価を記録する（[selfplay/tournament.cc 401〜423行](https://github.com/LeelaChessZero/lc0/blob/58e780090b8ffb765743b73a794f994c3a8c9476/src/selfplay/tournament.cc#L401-L423)、[game.cc 329〜344行](https://github.com/LeelaChessZero/lc0/blob/58e780090b8ffb765743b73a794f994c3a8c9476/src/selfplay/game.cc#L329-L344)）。
この値は、その対局で投了を許していたら誤った裁定になった閾値の境界を表す。
配布クライアントはこの値を`fp_threshold`としてサーバーへ送り、サーバーは対局ごとに`ResignFPThreshold`として保存する（[lczero-client lc0_main.go 149〜166行](https://github.com/LeelaChessZero/lczero-client/blob/37e557771c7148b9e4aa24df5fa223575f1397bd/lc0_main.go#L149-L166)、[lczero-server main.go 639〜701行](https://github.com/LeelaChessZero/lczero-server/blob/0d16642c28587f9dea0126606548f4703502b15f/main.go#L639-L701)）。
記録は確認できたが、投了閾値の調整にこの値をどう使ったかは公開されたサーバーのコードでは確認できなかった。

### 価値の目標と付け直し

lc0の方策の目標は探索の訪問分布、価値の目標は対局結果と探索後の評価を`q_ratio`で混ぜたものであり、その式と既定値はminaseの`docs/research/chess-evaluation-targets.md`にある。
`q_ratio`を導入したtest52は、test51とおおむね同じ強さだったと報告されている。

対局結果の目標は、学習の前にrescorerで2種類の付け直しを受ける。
第1は終盤データベースによる付け直しである。
rescorerは対局の手順を先頭からたどり、駒取りまたはポーンの移動の直後で、キャスリング権がなく、駒数がデータベースの範囲に入った局面に達すると、その局面の勝敗をデータベースで調べる（[rescorer.cc 560〜608行](https://github.com/LeelaChessZero/lc0/blob/58e780090b8ffb765743b73a794f994c3a8c9476/src/trainingdata/rescorer.cc#L560-L608)）。
そのうえで、その局面と、前回付け直した局面より後のすべての局面の結果を、証明された結果に手番を交互に反転させて置き換える。
したがって、データベースの範囲に入った後の終盤の指し損ないで、それより前の局面の対局結果の教師が歪むことはない。
2019年の公式ブログは、test40のネットを7駒の終盤データベースで付け直したデータで追加学習したと記す（[What's going on with training](https://lczero.org/blog/2019/06/whats-going-on-with-training/)）。
第2は悪手による結果の補正（deblunder）であり、温度で選ばれた手の評価が最善手より閾値以上悪い場合に、それより前の局面の結果を探索値から推定し直す。
その実装は既存の調査にあり、コードの既定では無効、JAXによる新しい学習器の設定例では閾値0.10、幅0.06である（[rescorer.cc 70〜80行、1313〜1315行](https://github.com/LeelaChessZero/lc0/blob/58e780090b8ffb765743b73a794f994c3a8c9476/src/trainingdata/rescorer.cc#L70-L80)、[example.textproto 40〜55行](https://github.com/LeelaChessZero/lczero-training/blob/ee626a97e517df76e85084339bbe6d0895eaba09/docs/example.textproto#L40-L55)）。
lc0のwikiの履歴は、T60でnet 68200からdeblunderを使い始めたと記す（[Project history](https://lczero.org/dev/wiki/project-history/)）。
T80で使った閾値は確認していない。

### 学習の窓と抽出

TensorFlow版の学習器は、新しい順に`num_chunks`個の対局ファイルを窓として読み、各対局から局面を1/32の確率で抜き出す（[tf/train.py 29行、113行](https://github.com/LeelaChessZero/lczero-training/blob/ee626a97e517df76e85084339bbe6d0895eaba09/tf/train.py#L29)）。
同じ抽出の段階に、探索前のネットの評価`orig_q`と探索後の評価`best_q`の差、および探索前後の方策のKLDから重みを作り、ネットの予測が当たっている局面を確率的に捨てる「diff focus」がある（[chunkparser.py 37〜44行、444〜459行](https://github.com/LeelaChessZero/lczero-training/blob/ee626a97e517df76e85084339bbe6d0895eaba09/tf/chunkparser.py#L37-L44)）。
docstringは、この選別が「how accurate the no-search eval ('orig_q') is compared to eval after search ('best_q')」に従うと説明する。
既定値では無効である。
wikiの履歴はT60のnet 67739以降とT76の開始時から「value focus」の設定を使ったと記しており、これはdiff focusに相当すると推測されるが、対応も具体的な値も確認していない（[Project history](https://lczero.org/dev/wiki/project-history/)）。

2018年の初期の系列では、窓と抽出の設定の誤りが評価の劣化を招いた記録がある。
lc0のwikiは、1日に約2,160万局面を生成して約3億720万局面を学習に使い、各局面を平均14.2回使っていたことを挙げ、過学習が強さの変動の最大の要因であり、価値の損失の重みを0.25に下げると強さが上がり始めたと記す（[Large Elo fluctuations starting from id253](https://lczero.org/dev/wiki/large-elo-fluctuations-starting-from-id253/)）。
同じページは、学習用と検証用のデータを分け、新しい対局が10万〜50万局たまってから次のネットを学ぶ方針を記す。
lc0の参加者Dubslowによる分析も、価値の出力が「overfit since close to the beginning of test 10」だったとし、1局面あたりの抽出数を1.016から0.762へ下げ、さらに約0.5へ下げることを提案した（[Dubslow's analysis of Test 10 problems](https://lczero.org/dev/wiki/dubslows-analysis-of-test-10-problems/)）。

### 世代の回し方と初代のネット

lc0の自己対局の反復は、1つのネットを連続して更新し、更新したネットを順に配布して自己対局に使う。
各系列の開始方法は異なり、T78は外部から提供された初期ネットから、T80はT79のデータで約200万ステップ学んだネットから始めた（[Project history](https://lczero.org/dev/wiki/project-history/)）。
2026年の論文は、lc0を「iteratively teaches a randomly initialized neural network」と説明しており、乱数からの開始が本来の形である（[Monroe et al. 2026 §5.1](https://arxiv.org/abs/2605.19091)）。

lc0の大型ネットの一部は、自己対局の反復ではなく、固定した自己対局データからの教師あり学習で作られた。
同じ論文は、2024年4月の強化学習の系列のうち強さが頭打ちになった後の対局だけを固定して使ったと記す。その対局は、1億パラメータのtransformerが1手600ノードで指したものである。
この方式で1億9,100万パラメータのLeela-CFを学び、それはデータを生成したネットを上回った（同A.2）。
同じ方式は、Stockfishに勝った大会のネットの学習にも使われたと記されている。
価値の目標は対局結果のWDLである（同§3.3）。
BT4がこの論文のLeela-CFと同じネットかどうかは確認していない。

### 開発者が説明する自己参照の補正

lc0の開発者は、自己対局の反復を、探索で強化した自分を教師とする蒸留として説明する。
2026年の論文は「In effect, a model continually generates strong search-enabled oracles whose play is then distilled back into that model」と書く（§5.1）。
同じ節は、畳み込みネットと別のtransformerがそれぞれ生成した自己対局データで学んだモデルがほぼ同じ強さになったという初期の実験を挙げ、「the strength of the oracle is far more important than the model underlying its search process」と結論する。
2018年の公式ブログは、探索後の評価を学ぶことを知識蒸留ととらえ、対局結果の併用が探索の限界や誤りの自己強化を抑える可能性を述べており、その内容は既存の調査にある。

自己対局が同じネット同士で指すことの弊害も、開発者は認識している。
2019年の公式ブログは今後の案として、異なる世代のネット同士に学習対局を指させれば「rock-paper-scissors sequences」を防げると書いたが、これは「unlikely, especially from the very beginning」とされた案であり、実施の記録は見つからなかった（[What's going on with training](https://lczero.org/blog/2019/06/whats-going-on-with-training/)）。
2023年1月の公式ブログは、評価を誤った対局をより深い探索で指し直して評価を直す「action replay」が、少なくとも一部の死角を取り除いたと報告する（[2022 progress](https://lczero.org/blog/2023/01/2022-progress/)）。
2019年3月のtest40では、駒取りを伴う昇格の手に方策をほぼ100%置き、通常の昇格を考えない死角が生じ、その原因はバッチ正規化の扱いにあったと報告された（[Test40 update](https://lczero.org/blog/2019/03/test40-update/)）。
この例では、死角はデータではなく学習の実装から生じており、偏りの原因をデータに限定できないことを示す。

## 比較表

表は、項目を行に、対象を列にとった。
Stockfishは、2020年8月から2021年までの自己生成期と、2021年後半から現在までの外部データ期に分けた。

| 項目 | Stockfish 自己生成期 | Stockfish 外部データ期と現行 | lc0 T80 |
|---|---|---|---|
| 1. 生成者 | 自分の採用ネット。最初の反復は手作り評価関数。 | 2021〜2022年の自己生成データとlc0の自己対局データ。現行はすべてBT4で付け直し。 | 自分の最新のネット。 |
| 2. 1手の探索量 | 深さ8〜11、または1手5,000ノード。 | lc0のデータは探索後のQ。付け直しは探索なしのBT4の価値出力。 | 上限10,000訪問とKLDによる打ち切り。平均は未確認。 |
| 3. 開始局面の多様化 | `noob_3moves`、UHO、DFRC、食い違い局面集。 | 既存データの開始局面を継承。対局試験はUHO_Lichess_4852。 | `run2_book`を無作為な順で使用。 |
| 4. 対局途中のランダム化 | 80手以内の最大3〜10手を、最善手から100以内の候補から選択。 | 新規生成なし。 | 温度0.9から減衰し40手目以降0.6。雑音10%。 |
| 5. 裁定 | `eval_limit`の4連続超過で80%の対局を裁定。評価値0の8連続で引き分け。 | 新規生成なし。 | WDLの98%超で裁定。20%は投了なしで最悪評価を記録。 |
| 6. 局面の選別 | 序盤16手未満と重複の除外。 | 駒取りと王手、序盤、駒数分布、WDL選別、無作為な間引き。lc0のデータは駒取りと良い手1つの局面の除外、重複除去。 | 1/32の抽出。diff focusは値未確認。 |
| 7. 教師値 | 自分の探索値。結果の混合は時期により0.2〜1.0。 | lc0のQ、現行はBT4の価値出力。結果はWDL選別とλ混合に使用。 | 訪問分布と、結果と探索後評価の混合。終盤データベースとdeblunderで結果を補正。 |
| 8. データ量 | 640億局面、20億局面、約160億局面など。 | 1,000億超から数千億局面。付け直しは20億、35億、740億局面を追加。 | 未確認。 |
| 9. 世代の回し方 | 前の採用ネットから継続して再学習。 | 固定のデータ集で、2022〜2023年は多段、現行は乱数からの2段。古いデータは保持。 | 1つのネットを連続更新。新しい対局の窓で学習。 |
| 10. 偏りへの対策と記録 | 食い違い局面からの再生成。採用試験を通過。 | 外部の強い教師、WDL選別、結果混合からWDL選別への置換、非対称な損失、重複除去。各採用は複合変更の試験。 | 温度と雑音、誤投了の記録、終盤データベース、deblunder、diff focus、過学習の抑制。 |
| 11. 初代のネット | 手作り評価関数による深さ8の教師（転載資料）。 | 自己生成データで乱数から学ぶ第1段。現行は乱数から。 | T79のデータで約200万ステップ学んだネット。 |

## minaseへの含意（推論）

以下はすべて推論であり、中将棋とminaseで効果を確かめた結果ではない。

第1に、Stockfishで偏りの補正に最も効いたのは自分より強い外部の教師であり、中将棋にはこれに当たるものがない。
したがってStockfishの現行の手順は、そのままではminaseの手本にならない。
minaseに近いのは2020年8月から2021年前半の自己生成期であり、その時期の手段は、食い違い局面の採取、ランダムな手の範囲の制限、結果を重く混ぜた反復、および深さ9を中心とする浅い探索の大量生成である。

第2に、対局結果をλで混ぜる代わりに、結果と矛盾する教師を捨てるWDL選別は、minaseの既存の教師と結果だけで試せる。
Stockfishの開発者はWDL選別の下では結果を混ぜないほうが良いと報告しており、minaseで観測した「λ=0.75でも偏りが生じた」こととは別の結果の使い方である。
ただし、麒麟を右端へ運ぶ癖のように自己対局の双方が同じ選好を持つ場合、その選好は勝敗を左右しにくく、結果との矛盾として現れない可能性がある。
WDL選別が拾うのは、評価値が勝敗の予測を誤る局面であり、勝敗に関係しない配置の選好ではない。

第3に、食い違い局面の採取は、minaseでは「静的評価と深い探索の値が大きく食い違う局面」を集めて開始局面集にする形で応用できる。
ただし、minaseの過去の診断では1,000万ノードまで付け直しても王駒の露出の誤りが直らなかったので、探索の末端で同じ評価関数を使う限り、深い探索が偏りを打ち消すとは限らない。
偏りの検出には、評価関数と独立な基準として、lishogiの人間の対局での配置の頻度や、旧世代の評価関数との差を使うほうが確実である。

第4に、lc0が終盤データベースで結果の目標を上書きする仕組みは、中将棋では詰み探索による証明で置き換えられる可能性がある。
minaseのdf-pnの計画が実装されれば、詰みの証明が得られた局面より前の結果を証明された結果で上書きし、終盤の指し損ないによる対局結果の教師の歪みを除ける。

第5に、自己生成の多様化について、Stockfishの生成器は序盤の最大3〜10手を最善手から100以内の候補から選び、lc0は40手目以降も温度0.6を残す。
lc0は終盤の温度を0にした系列が明らかに弱かったと報告している。
minaseは対局途中のランダムな手の仕組みを持ちながら使っておらず、開始局面のランダム化だけで多様化している。
特定の駒の配置の癖が対局を通じて固定される場合、途中のランダム化と開始局面集の変更は、その癖を持たない局面への到達を増やす候補になる。

第6に、Stockfishの生成器の20%の対局は投了しないが、lc0と異なり誤投了の率を記録しない。
minaseが投了や評価値による打ち切りで対局を短くする場合、一部の対局を最後まで指し、打ち切りが誤りだった割合を記録すれば、打ち切りの閾値が偏りを生んでいないかを測れる。

第7に、lc0の論文は、自己対局データを生んだネットの構造よりも、探索で強化した教師の強さのほうが重要だと結論している。
これは生徒側のネットの構造についての結論ではないので、minaseで評価関数をPSTからNNUEへ作り替えても癖が残った観察と直接には対応しない。
ただし、教師の出どころと強さが学習結果を左右するという点では同じ方向を指しており、評価関数の形式より教師を生む探索とデータの出どころを比べる理由の1つにはなる。

## 未確認の事項

- `multinet_pv-2_diff-100_nodes-5000`、`data_pv-2_diff-100_nodes-5000`、`tb5dtm.binpack`の生成方法と生成時期。
- `nodes5000pv2_UHO`と`wrongIsRight_nodes5000pv2`の生成コマンド。本書の説明はファイル名とwikiの記述からの推測を含む。
- Hugging Faceの`vondele/rescored`にある、lc0のデータをStockfishの3,000〜80,000ノードの探索で付け直したファイル群の目的と結果。現行のレシピには入っていない。
- UHOの正式名称と局面の選定基準。作者のページを取得できなかった。
- Stockfishの最初の採用ネット（2020年8月）に使われた教師データの深さと量、およびnodchip本人による手順の原文。
- 現行のレシピの微調整段が追加で何エポック学ぶか。本書は600エポックまで続けると書いたが、学習器の再開時のエポック数の扱いから40エポックの追加と推測している。
- lc0のT80の1手あたりの平均訪問数、データ量、窓の大きさ、`q_ratio`、deblunderの閾値、diff focusの値、およびwikiの「value focus」とdiff focusの対応。
- lc0のサーバーが`ResignFPThreshold`を投了閾値の調整に使っているかどうか。
- BT4が2026年の論文のLeela-CFと同じネットかどうか、およびBT4の学習データ。
- 2026年5月の採用コミットが「does not use a lambda schedule for the most part」と書いた設定と、現行の余弦曲線によるλの変化との関係。

## 確認した版

| 対象 | コミットと日付 | 用途 |
|---|---|---|
| Stockfish（手元の複製） | `5c4cef0bfe61fb3249178b2bcf21f5673557efe7`、2026年10月13日 | 既定のネットを変えた119件のコミットのメッセージ、公開文。 |
| Stockfishの`tools`ブランチ | `9a4c7cf4e311f8d9526b79295b80c4d0464c07cf`、2023年4月25日 | 教師生成器と文書。 |
| nnue-pytorch | `ed84cc1fd2255faa0f33dcfa7839c9b6b4bb49b8`、2026年10月17日 | 履歴の調査。行番号はレシピが固定する`140fd799ef4c63576833c15a7fe52ab88ba411b6`（2026年10月14日）による。 |
| nnue-pytorchのwiki | `0e18032a596878725bd4d4772c24283bb605a4e2`、2026年10月10日 | データ集の説明。該当の記述は2021年12月6日の`46a9ae3`で追加。 |
| nettest | `ab168ab44a9957e96732a0e39d11852d592b0746`、2026年10月24日 | 現行のレシピと履歴。 |
| sergiovieri/Stockfish `tools_mod` | `6b0838b1404e91aafff17c4f2ac0a8ced999a307`、2021年7月24日 | 強化学習の反復のスクリプト。 |
| vondele/Stockfish | `3ce43ab0c4ce09c1fc5bca5ca27a248e67fddd24`、2021年6月1日 | 食い違い局面の採取。 |
| xu-shawn/lc0 | `3f83fab871d641e8aaed8a55481f3b7487977606`、2026年6月25日 | BT4による付け直しツール。 |
| official-stockfish/books | `a804a97e709f40aa75a00f9d19e84915b7a3a45c`、2026年10月16日 | 開始局面集の一覧。 |
| lc0 | `58e780090b8ffb765743b73a794f994c3a8c9476`、2026年10月20日 | 自己対局、投了、rescorer。 |
| lczero-training | `ee626a97e517df76e85084339bbe6d0895eaba09`、2026年10月24日 | 抽出、diff focus、設定例。 |
| lczero-client | `37e557771c7148b9e4aa24df5fa223575f1397bd`、2026年5月4日 | 誤投了の記録の送信。 |
| lczero-server | `0d16642c28587f9dea0126606548f4703502b15f`、2021年6月23日 | 誤投了の記録の保存。 |
| Webページ | 2026年10月25日に参照 | 学習サーバーの設定、lc0の公式ブログとwiki、Hugging Faceのデータ集、arXiv:2605.19091。 |
