# HaChuの深さの選定

## 目的

[強さを測る固定の相手の設計書](../../plans/mcts-fixed-opponents.md)のフェーズ2として、1回目の強化学習のサイクル32のネットのMCTSを、外部エンジンHaChuの深さ1、2、3、および4と30ペアずつ対局させ、HaChuを固定の相手にする深さを選ぶ。
選定の規則は、設計書の「深さの選定」の節にある。得点率が5割に最も近い深さを選び、その得点率が2割から8割の範囲になければ採らない。
加えて、設計書の「HaChuとの対局の条件」の節は、拒否着手または不正着手のあった対局を含むペアと、手数の上限で破棄したペアの合計が、試行したペアの5%を超えた条件の値を選定に使わないと定めている。
深さ4は設計書の範囲の外であり、深さ1から3のどれも2つの条件を満たさなかったので、利用者に諮る前の材料として足した。

## コマンドライン

測定日は2026年11月13日から14日である。
[固定の相手との対局のスクリプト](mcts-fixed-opponents-scripts/)の`run_match.py`を、作業機のリポジトリの根で、次のように4回実行した。
`<d>`は1、2、3、4であり、この順に続けて実行した。4つの深さは同じシードで対局したので、開始局面の列は同じである。

```console
source tools/train/zero/env.sh   # bashで実行する
export XDG_CACHE_HOME=$HOME/.cache
W=data/worktrees/mcts-fixed-opponents
python3 -I $W/docs/measurements/mcts-fixed-opponents/mcts-fixed-opponents-scripts/run_match.py hachu \
  --harness $W/target/release/minase --mcts-commit c1c84466476eb36cf43abc529792469862b8e856 \
  --net data/mcts-fixed-opponents/nets/cycle-0032.onnx --hachu ../hachu-debian/hachu --depth <d> \
  --name mcts-fixed-opponents-hachu-depth-d<d> --seed 20261113200000 --pairs 30 --concurrency 4
```

スクリプトが起動した対局ハーネス`minase match run`のコマンドは次のとおりである（`<net>`はネットのファイルの絶対パス）。

```console
data/worktrees/mcts-fixed-opponents/target/release/minase match run \
  --run-dir data/matches/mcts-fixed-opponents-hachu-depth-d<d> --seed 20261113200000 \
  --candidate commit:c1c84466476eb36cf43abc529792469862b8e856 --candidate-args '--search mcts' \
  --candidate-option NnModel=<net> --candidate-option NnBatchSize=16 --candidate-limit nodes=800 \
  --baseline cecp:../hachu-debian/hachu --baseline-limit depth=<d> --baseline-hash 256 \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --max-ply 1900 --concurrency 4 elo --pairs 30
```

1ペアは、ハーネスが初期局面から8〜12手を乱択で指した同じ開始局面で、先後を入れ替えた2局である。
本番の前に、同じスクリプトでHaChuの深さ1と2ペアを対局させ（実行ディレクトリ`mcts-fixed-opponents-script-smoke-hachu`、シード20261113500000）、最後まで異常なく動くことを確かめた。
拒否着手と不正着手は、同じディレクトリの`check_hachu_forfeits.py`で分類した（結果は[forfeits-d2.json](mcts-fixed-opponents-hachu-depth/forfeits-d2.json)、[forfeits-d3.json](mcts-fixed-opponents-hachu-depth/forfeits-d3.json)、[forfeits-d4.json](mcts-fixed-opponents-hachu-depth/forfeits-d4.json)）。

```console
python3 -I $W/docs/measurements/mcts-fixed-opponents/mcts-fixed-opponents-scripts/check_hachu_forfeits.py \
  data/matches/mcts-fixed-opponents-hachu-depth-d<d> --hachu ../hachu-debian/hachu --minase $W/target/release/minase
```

### 接続の検証

対局の前に、[外部エンジンは接続前に棋譜照合と着手転送で検証する](../../lessons/verify-external-engine-before-cecp-match.md)に従って、次の2つを確かめた。

- `scripts/hachu_selfplay.py`でHaChuの自己対局を3局（380手、1,675手、234手の計2,289手）生成し、`scripts/hachu_replay.py`でコミットc1c84466のminaseに読ませて再生した。審判層は、規則`L1,L3,P0,P5,P6,R2,E1,E2`で全手を合法と判定した。
- 2つのHaChuのプロセスに深さ2で交互に指させ、着手をCECPのコマンド`usermove`で相手のプロセスへ80手転送した。どちらのプロセスも着手を拒否せず、最後に盤面を印字させると両者は一致した。同じ80手を審判層で再生すると、全手が合法であった。

## エンジン

候補（`--candidate`）は、[固定したαβ探索との基準の記録](mcts-fixed-opponents-alphabeta-baseline.md)と同じく、masterのコミットc1c84466のMCTSに、サイクル32のネット（ONNXのSHA-256 `cdc64e02fbd3ff54a935775c1bfe7610e33b02e498d41270a55bc705792d76a6`）を与えたものである。ハーネスがビルドしたバイナリのSHA-256は`60e6611ef44f6b23669be6da2164b21dce4c0ef383b218813086fd7ee06e7c36`である。
思考の制限は1手800回のシミュレーションであり、方策の温度は既定の1.4である。

基準（`--baseline`）は、HaChu（Debianパッケージ収録のオリジナル版、コミットdf26f4a、`../hachu-debian`で`gcc -O2`によりビルド、バイナリのSHA-256は`10db1299f95626fe58875b1c24a1cee6acce0eee5fb42ed8b17ce1373a9dcbdf`）である。
規則オプションは既定の設定（“Okazaki rule” 無効、“Promote on entry” 有効、“Allow repeats” 無効）であり、思考の制限は深さ（CECPの`sd <d>`）、置換表は`memory 256`である。

規則は、HaChuの既定の設定に対応する`L1,L3,P0,P5,P6,R2,E1,E2`（[競技規則](../../rules/chu-shogi.md)第33条第7項）を、審判層とMCTSの双方に与えた。
HaChuは棋譜の配列の上限が2,000手で、それを超えると異常終了するので、手数の上限`--max-ply`を1,900手にした。
対局のハーネスは、[固定したαβ探索との基準の記録](mcts-fixed-opponents-alphabeta-baseline.md)と同じバイナリである。

## 環境

作業機（Intel Core Ultra 7 265KF、物理20コア、論理20コア、NVIDIA GeForce RTX 3060 Ti 8 GB、ドライバ615.78.08）で、ONNX Runtime 1.31.1とTensorRT 10.14.1を使った。
候補の`Threads`は1、推論はTensorRT（FP16）、`NnBatchSize`は16である。HaChuは探索のスレッド数を報告しない。
候補の`USI_Hash`は既定の256 MB（MCTSは使わない）、HaChuの`memory`は256 MB、同時対局数は4、応答タイムアウトは120秒である。
同じ機械では、lishogiのBotが通信対局を指していた。対局はシミュレーション数と深さの制限なので、この負荷は対局の結果に影響しない。

## 結果

全部の対局で、エンジンの異常終了、応答タイムアウト、時間切れ（`time_forfeits`）、および拒否着手（`rejected_moves`）は0件であった。
ペンタノミアル度数は、先後を入れ替えた2局のMCTSの得点が0、0.5、1、1.5、および2であるペアの数であり、得点率とEloは有効ペアの値である。
影響ペアは、HaChuの不正着手のあった対局を含むペアと、手数の上限で破棄したペアの和集合である。
集計は`summarize.py`の出力（[summary.json](mcts-fixed-opponents-hachu-depth/summary.json)）である。

| 深さ | ペンタノミアル度数 | 得点率 | Eloの点推定 | 95%信頼区間 | 破棄ペア | 不正着手のペア | 影響ペアの割合 | 経過時間（秒） |
|---|---|---|---|---|---|---|---|---|
| 1 | [0, 0, 1, 0, 28] | 98.3% | +702 | [+510, +∞] | 1 | 0 | 3.3% | 303 |
| 2 | [0, 0, 14, 0, 13] | 74.1% | +182 | [+105, +282] | 3 | 2 | 16.7% | 557 |
| 3 | [14, 0, 13, 0, 1] | 26.8% | −175 | [−284, −90] | 2 | 2 | 13.3% | 562 |
| 4 | [22, 0, 6, 0, 0] | 10.7% | −368 | [−597, −260] | 2 | 3 | 16.7% | 647 |

設計書が併記を求めるEloは次のとおりである。
不正着手のペアを除いたElo、破棄ペアをMCTSの2敗とした場合のElo、および2勝とした場合のEloを並べる。

| 深さ | 不正着手のペアを除く | 破棄ペアを2敗とする | 破棄ペアを2勝とする |
|---|---|---|---|
| 1 | +702 [+510, +∞] | +512 [+345, +∞] | +708 [+516, +∞] |
| 2 | +164 [+87, +260] | +120 [+35, +223] | +207 [+129, +310] |
| 3 | −209 [−323, −126] | −191 [−302, −107] | −134 [−242, −46] |
| 4 | −478 [−∞, −340] | −382 [−613, −274] | −280 [−478, −170] |

終局の内訳は次のとおりである。MCTSの勝ちはすべてHaChuの投了であり、HaChuの勝ちはすべて王駒の捕獲である。
MCTSは、ハーネスが投了を無効にしているので投了しない。

| 深さ | HaChuの投了 | HaChuによる王駒の捕獲 | HaChuの不正着手 | 手数の上限 | 1局の手数（最小、中央値、最大） |
|---|---|---|---|---|---|
| 1 | 58 | 1 | 0 | 1 | 111、239、1,900 |
| 2 | 40 | 15 | 2 | 3 | 120、401、1,900 |
| 3 | 14 | 42 | 2 | 2 | 169、378、1,900 |
| 4 | 3 | 52 | 3 | 2 | 130、310、1,900 |

### HaChuの不正着手

HaChuの不正着手は7件あり、7件とも、HaChuが反復の判定に直近の200局面しか使わないために、200手より前の局面を再現する着手を指したものであった（`check_hachu_forfeits.py`の分類`history_window`）。
分類のスクリプトは、記録の着手列に沿ってHaChuを同じ深さで動かし直し、HaChuの手番ごとの着手が記録と一致することを確かめたうえで、不正着手の手を得る。その手をminaseのCECPのエンジンに送ると、7件とも`Illegal move (repetition)`が返った。

| 深さ | ペア | 何手目か | HaChuの着手（USI） | 同じ配置と手番の局面との差 |
|---|---|---|---|---|
| 2 | 14 | 889 | `6g6f` | 248手 |
| 2 | 26 | 1,078 | `3d3l` | 208手 |
| 3 | 1 | 1,231 | `5e7e` | 212手 |
| 3 | 21 | 1,464 | `6f7g` | 226手 |
| 4 | 19 | 878 | `5k11e` | 284手 |
| 4 | 20 | 1,851 | `9f11f` | 208手 |
| 4 | 29 | 1,550 | `7f5f` | 334手 |

### 長い対局の内容

破棄と不正着手は、どちらも対局が長くなったときに起きた。
HaChuの既定の規則は、既出の局面を再現する着手を禁じ（R2）、駒枯れの裁定も持たない（E2）ので、王駒を取るか投了するまで対局が終わらない。
850手以上の局は、深さ1で60局のうち1局、深さ2で13局、深さ3で10局、深さ4で8局であった。

深さ1から4の、手数の上限に達した8局（破棄ペアごとに1局）と不正着手の7局について、記録の着手列を盤面に再生し、終わった時点の駒の数と、MCTSが最後に報告した評価値を調べた。
MCTSの評価値は、MCTSの手番から見た値をセンチポーン相当に換算した値である。
勝ちきれない側は、局によって異なった。
MCTSが優勢のまま勝ちきれなかった局は6局である。たとえば、深さ2のペア16の第2局は、HaChuの王将と成り駒1枚に対してMCTSが10枚の駒を持ち、評価値+4,310のまま1,900手に達した。深さ2のペア14の第2局は、王将だけのHaChuに対してMCTSが王将と成り駒1枚を持ち、評価値+1,291で889手目にHaChuの不正着手で終わった。
HaChuが優勢のまま勝ちきれなかった局は8局である。たとえば、深さ2のペア11の第2局は、HaChuが獅子と成り駒を含む15枚の駒を持ってMCTSの8枚を上回り、評価値−2,563のまま1,900手に達した。深さ4のペア19の第1局は、HaChuが18枚、MCTSが8枚で、評価値−5,048のまま877手目にHaChuの不正着手で終わった。
評価値が互角に近い局は、深さ3のペア24の第2局（評価値+109）の1局である。
長い局は深さ2から4でほぼ同じ割合で起き、影響ペアの割合は深さを変えても5%以下にならなかった。

## 結論

得点率が5割に最も近いのは深さ3（26.8%）であり、2割から8割の範囲に入るのは深さ2と3である。しかし、どちらも影響ペアの割合が5%を超え（16.7%と13.3%）、影響ペアが5%以下の深さ1は得点率が98.3%で範囲の外であった。
設計書の2つの条件を同時に満たす深さはなかった。

利用者は、2026年11月14日に、固定したαβ探索を主な指標とし、HaChuを深さ3の副次の指標とすることに決めた。
HaChuとの対局は、影響ペアの割合が5%を超えても値を捨てず、不正着手のペアを除いたElo、破棄ペアを2敗と2勝としたElo、および影響ペアの割合を毎回併記する。
