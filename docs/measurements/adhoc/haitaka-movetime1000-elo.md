# haitaka-movetime1000-elo

## 目的

minaseと、変則将棋エンジンhaitaka-variantsの中将棋を、1手1秒の固定時間で20ペア対局させ、両者をつなぐアダプタが異常なく動くことと、対等な思考時間での勝敗を確かめる。
haitaka-variantsは将棋ったーの中将棋（ルール48）を実装するエンジンであり、手作りの評価関数と反復深化のαβ探索で指す。
採否の測定ではなく、外部エンジンとの最初の対局である。

## コマンドライン

対局ハーネスは、master `b10ad9b6`の作業ツリーでビルドしたバイナリ（v3.0.0-alpha.1、SHA-256 `bd089ac21d3939051b205d580ef9baa56d5930a9d452d5fcdcee15e75e3e4a02`）から起動した。

```console
target/release/minase match run \
  --run-dir data/matches/haitaka-movetime1000-elo --seed 20261200 \
  --candidate commit:97e2da0d \
  --baseline "python3 scripts/haitaka/haitaka_usi.py /home/stepney141/board-games/minase/data/external-engines/haitaka-variants-6222835/haitaka_cli" \
  --candidate-option EvalFile=/mnt/synology_nas/minase/nnue/nets/37054c9e49c9218bb18efdd1814fe1b298de5cbfc9c14352f6ad6eaecbb70dea.bin \
  --rules L1,P0,P3,R1,E1,E2 --each movetime=1000,timemargin=200 \
  --concurrency 8 elo --pairs 20
```

固定時間（`movetime=`）の測定は`minase match report`の集計対象外なので、標準出力の最終サマリと`pairs/`の保存記録を記録の値とする。

## エンジン

候補はminaseのコミット`97e2da0d7215a0e1f5fe3f0456a20b593594fd4f`（バイナリのSHA-256 `623abc10e2ded0ca7bdd34a37a2f999b82b54f2ee4db88825b2d2bcad6974621`）であり、`Threads`は1である。
ネットは[NNUEの評価関数の手引き](../../guides/evaluation-nnue.md)が標準とするP型のネットで、`network_sha256`は`37054c9e49c9218bb18efdd1814fe1b298de5cbfc9c14352f6ad6eaecbb70dea`である。

基準は[haitaka-variants](https://github.com/na2hiro/haitaka-variants)のコミット`6222835915ea2a3886696e96c14034ca4294a7e2`（2026年11月19日）である。
rustc 1.100.0で`cargo build --release -p haitaka_cli --features chushogi`を実行してビルドした`haitaka_cli`（SHA-256 `41e35a552f626dc798dcd32ce6e0abb3cd49b804abb236cb1b8b596a567bc8f3`）を、`data/external-engines/haitaka-variants-6222835/`へ複写して使った。
haitakaの中将棋はUSIを実装せず、独自のJSONで局面と着手を受け渡す`haitaka_cli json`だけを入口に持つので、この入口とUSIの間を中継するアダプタ`scripts/haitaka/haitaka_usi.py`（Python 3.14.10、本記録と同じコミットで追加した版）を介して対局した。
アダプタは`go movetime`の値を、着手の適用にかかった時間を引いたうえで、haitakaの探索の打ち切り時間として渡す。
haitakaの中将棋は手作りの評価関数だけを使い、アダプタはネットのチェックサムを報告しないので、基準の`network_sha256`は`null`である。

規則セットは審判層とminase側の双方に`L1,P0,P3,R1,E1,E2`を与えた。各コードの内容は[chu-shogi.md](../../rules/chu-shogi.md)の第10章にある。
haitakaは規則を切り替えられず、この組が最も近い。
この組とhaitakaの規則の間に残る差は、アダプタのdocコメントにある。

接続の前に、`scripts/haitaka/haitaka_rule_diff.py`でminase（上記のハーネスと同じバイナリ）とhaitakaの合法手と終局の判定を突き合わせた。
乱数の対局30局（1局400手まで）を2通りの条件で行い、それぞれ12,030局面を比べた。
条件は、シード1で駒を取る手だけから次の着手を選ぶ確率（`--capture-rate`）を0.3としたものと、シード2でこの確率を0.7としたものである。
食い違いは7局面と2局面で、すべて同じ種類だった。
獅子が第1段階で歩兵または仲人を取り、第2段階で移動元に隣接する足のある獅子（味方の駒に守られた獅子、chu-shogi.md第13条）を取る着手を、minaseだけが合法とした。
haitakaだけが合法とする着手と、終局の判定の食い違いはなかった。
乱数の対局に現れなかった先獅子の差は、局面`5k3r2/12/12/12/6n5/6+H2N2/12/12/12/12/12/5K6 b - 1`を双方に与えて確かめた。
先手の角鷹が居喰い`6f6e6f`で獅子を取った直後に、後手の飛車が先手の獅子を取る`3a3f`を、haitakaは合法とし、minaseは不合法とした。
続けて、1手0.5秒の4ペア（`data/matches/haitaka-smoke`、シード20261122）で起動を確かめ、異常0件、ペンタノミアル度数[0, 0, 0, 0, 4]だった。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ33,218,777,088バイト）、OSはLinuxである。
候補の`Threads`は1であり、haitakaの探索は1スレッドで、アダプタも`Threads`を1と宣言する。
候補の`USI_Hash`は256 MBである。
haitakaは置換表の容量の設定を持たず、アダプタは`USI_Hash`を宣言しない。
haitakaの置換表は探索ごとに作る65,536項目までの表であり、アダプタは手ごとに別のプロセスを起動して探索するので、手をまたいで残らない。
同時対局数は明示の8、手数上限は4,096手、応答タイムアウトは120秒である。
思考制限は両者とも1手1,000 ms、時間切れ判定の許容時間は200 msである。
2026年11月22日10時35分から10時47分に実施した。
同じ機械ではlishogiのBotが対局中で、開始時点ではエンジン2プロセスが各約4コアを使っていた。
測定中に、このBotとハーネス以外の対局、生成、学習は走らせていない。

## 結果

| 量 | 値 |
|---|---:|
| 有効ペア | 20（破棄0） |
| ペンタノミアル度数 | [0, 0, 0, 0, 20] |
| 勝敗（minaseから見て） | 40勝0敗0分 |
| Elo | 全勝のため点推定は無限大 |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 687.9秒（`summary.json`の累計は687.8秒） |

40局の終局理由は、王駒の捕獲が35局、反復の裁定が5局である。
反復の5局（ペア3の両局、ペア4の第2局、ペア6の第2局、ペア12の第2局）は、同一局面の4回目の出現までのhaitakaの着手がすべて攻撃的着手だったので、規則R1によりhaitakaの負けと裁定された対局である。
5局とも、最後の10手は双方が2升を往復する着手の繰り返しであり、minaseは反復に入ると、反復による勝ちを表す評価値29,742を出した。
haitakaの規則は、連続して王手をかけた場合に限って反復を負けとし、それ以外を引き分けとする。
5局の最終局面を`haitaka_cli json inspect`に渡すと、haitakaはどれも引き分けと判定した。
したがって、この5局はminaseがR1の裁定を読んで選んだ勝ちであり、haitakaの規則の下での勝ちではない。
最短の対局はペア4の第2局の34手で、反復の間のhaitakaの評価値は−360前後だった。

1局の手数（開始局面を作る8〜12手を含む）は中央値118.5手、最小34手、最大245手である。
1手の思考時間は、minaseが中央値1,007.6 ms、最大1,018.2 ms、haitakaが中央値1,001.1 ms、最大1,005.7 msであり、時間切れの判定までの余裕はどの手でも180 ms以上あった。
探索が完了した深さの中央値は、minaseが22、haitakaが3である。
ハーネスが記録した1局のCPU時間の中央値は、minaseが55.2秒、haitakaの側が0.08秒である。
haitakaの側の値はアダプタだけのもので、手ごとに起動する`haitaka_cli`の探索の時間を含まない。
アダプタが理由を標準エラーへ書いて終了した対局はなく、接続の前の突き合わせで見つかった獅子の着手の差は、この40局には現れなかった。

## 結論

1手1秒の対等な思考時間で、minase（コミット`97e2da0d`）はhaitaka-variantsの中将棋（コミット`6222835`）に40戦全勝し、アダプタに起因する異常は0件だった。
ただし、40勝のうち5勝は、haitakaの規則なら引き分けになる反復を規則R1が負けと裁定したものであり、王駒の捕獲による勝ちは35局である。
