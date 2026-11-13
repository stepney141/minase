# 自己対局の生成時間に占める合法手の全件生成の割合

## 目的

ニューラルネットワーク（以下、ネット）の推論を使う実際の自己対局の生成で、`Game::legal_moves`による合法手の全件生成が生成スレッドの時間のどれだけを占めるかをプロファイルで測る。
自己対局の探索はMCTS（モンテカルロ木探索）であり、探索木に新しい節点（葉）を作るたびに、その局面の合法手をすべて生成する。
[やねうら王の合法手生成の構造をMCTSの葉の全件生成へ応用する調査](../../proposals/archived/yaneuraou-movegen-for-leaf-legal-moves.md)は、この全件生成を速くする3つの候補を挙げており、本測定の結果で、その候補を設計書として起案するかを判断する。
この提案は、ネットの推論を行わない一様評価器で駆動したMCTSの命令数の内訳を根拠にしており、推論を含む生成の時間に対する割合は示していなかった。
この測定は、どの設計書も計画していない。

## コマンドライン

測定日は2026年11月13日である。
エンジンは、フレームポインタと行番号のデバッグ情報を付けて、リポジトリの外の一時ディレクトリ`$S`へビルドした。
`RUSTFLAGS`を設定すると`.cargo/config.toml`の`rustflags`が使われなくなるので、そこで指定している2つのフラグ（`-C target-cpu=native`と`-C llvm-args=-inline-threshold=1000`）を先頭に書いている。

```console
RUSTFLAGS="-C target-cpu=native -C llvm-args=-inline-threshold=1000 -C force-frame-pointers=yes" \
  CFLAGS="-fno-omit-frame-pointer" CARGO_PROFILE_RELEASE_DEBUG=line-tables-only \
  nice -n 19 cargo build --release --offline --locked -p minase --bin minase --target-dir $S/target
```

TensorRTのエンジンを作る時間をプロファイルに含めないために、記録の前に、次のコマンドの`--games`と`--chunk-games`を16に替えた生成を`perf`なしで1回行い、エンジンをキャッシュに作った。
続けて、次の生成の全体を`perf`で記録した。
引数は、対局数とGPUの枚数を除いて、[借りた機械での生成の速さの測定](../alphazero-inference-transfer/alphazero-inference-transfer-generation-speed.md)のGPU 1枚の機械と同じである。

```console
Z=tools/train/zero
SP=$Z/.venv/lib/python3.13/site-packages
export ORT_DYLIB_PATH=$Z/.runtime/onnxruntime-linux-x64-gpu_cuda13-1.31.1/lib/libonnxruntime.so
export LD_LIBRARY_PATH=$SP/tensorrt_libs:$SP/nvidia/cu13/lib:$SP/nvidia/cudnn/lib
export XDG_CACHE_HOME=$S/cache
nice -n 19 perf record -F 499 -e cpu-clock:u --call-graph fp --user-callchains -o $S/perf.data -- \
  $S/target/release/minase data zero-selfplay --output-dir $S/run --nn-model $S/rl2-init.onnx \
  --nn-backend tensorrt --generation-commit de8ab9ab3c48b3794493b4fa370d21e8dc155d0a --seed 9100000000 \
  --start-position shuffled-setup --games 2048 --target-positions 1000000000 --chunk-games 256 \
  --simulations 32 --playthrough-percent 100 --threads 4 --devices 1 --games-per-thread 128
perf script -i $S/perf.data -F comm,tid,period,ip,sym,dso > $S/perf.script
python3 -I analyze.py $S/perf.script > profile.json
```

`--user-callchains`は、呼び出しスタックの記録をユーザー空間に限る指定である。
作業機の`kernel.perf_event_paranoid`は2であり、この指定がないと、`perf`がカーネル側のスタックも要求してイベントを開けない。
`cpu-clock`は時間で標本を取るイベントなので、標本の`period`（ナノ秒）の和がそのままCPU時間になる。
[analyze.py](zero-selfplay-legal-moves-share/analyze.py)は、標本をスレッドごとに分け、スタックのどこかに指定の関数を含む標本の時間（包含時間）と、`Game::legal_moves`を呼んだ関数ごとの時間を集計する。
呼び出し元は、インライン展開されたフレームを飛ばして、実体のある最初のフレームとした。

## エンジン

測定したエンジンは、masterの`de8ab9ab`（2026年11月13日）である。
ネットは、[2回目の強化学習の実行](../../plans/alphazero-rl-2.md)がサイクル1の生成に使う、ランダムに初期化した20ブロック256チャネルのネット（NASの`cycles-rl2/cycle-0000/model.onnx`、SHA-256は`7a9f3ec7…`）であり、推論はTensorRTのFP16である。
規則は、生成器が固定で使う`JCSA`（`L0,P0,R1,E0`）である。
フレームポインタを付けたビルドは、masterの通常のビルドと機械語が異なる。
この差が生成の速さに与える影響は測っていない。

## 環境

作業機（Intel Core Ultra 7 265KF、物理20コア、論理20コア、GPUはNVIDIA GeForce RTX 3060 Ti 8 GB）で測った。
カーネルは`6.18.57-2-lts`、`perf`は7.2.11、ONNX Runtimeは1.31.1である。
生成のスレッドは4、スレッドごとの同時対局数は128である。
同じ機械でlishogiのBotが対局しており、ロードアベレージは測定の開始時に3.95、終了時に7.77であった。
生成スレッドが性能コアと効率コアのどちらで動いたかは記録していない。

## 結果

生成は630.5秒で2,048局、296,403手（スレッドあたり毎秒117.5手）を指し、訓練局面は73,594局面であった。
生成器は手番ごとに、確率0.25で32回の完全探索を、それ以外は8回の速い探索を行い、完全探索の手番だけを訓練局面として記録する。
完全探索は73,594手、速い探索は222,809手であり、手数上限で破棄した対局はない。
`perf`の標本は1,255,882個、全スレッドのCPU時間は2,516.8秒であった。

生成の4スレッドのCPU時間は合計2,492.1秒であり、経過時間の4倍の98.8%にあたる。
ただし、その96.5%はGPUの推論の完了を待つ時間であり、`libcuda`の中が85.4%、`clock_gettime`が11.0%を占めた。
次の表は、4スレッドの包含時間の合計である。
下の3行（`generate_piece_moves`、`push_with_promotion`、および`special_move_is_legal`）は、呼び出し元を問わない合計であり、着手の合法性と終局を裁定する審判層（`Game::play`）からの呼び出しを含む。
1手あたりの値は、合計を296,403手で割ったものである。

| 処理 | 合計 | 生成スレッドのCPU時間に占める割合 | 1手あたり |
|---|---:|---:|---:|
| 生成スレッドの全体 | 2,492.1秒 | 100% | |
| 自己対局のコード（`zero_selfplay`を含むスタック） | 68.42秒 | 2.75% | 230.8 µs |
| MCTSの駆動部と探索木（`search::mcts`を含むスタック） | 28.78秒 | 1.15% | 97.1 µs |
| `Game::legal_moves` | 10.56秒 | 0.42% | 35.6 µs |
| 全手生成で駒1枚の着手を作る処理（`generate_piece_moves::<false, false>`） | 10.41秒 | 0.42% | 35.1 µs |
| 着手ごとの判定と出力（`push_with_promotion`） | 4.25秒 | 0.17% | 14.3 µs |
| 獅子の捕獲制限の検査（`special_move_is_legal`） | 1.19秒 | 0.05% | 4.0 µs |

`Game::legal_moves`の10.56秒を呼び出し元で分けると、次のとおりである。

| 呼び出し元 | 合計 | 用途 |
|---|---:|---|
| `Driver::select_leaf` | 8.59秒 | MCTSが新しい節点を作るときの全件生成 |
| `mate_in_three::winning_attack` | 1.46秒 | 自己対局の3手詰めの判定 |
| `SearchSnapshot::from_game` | 0.36秒 | 探索の根の準備 |
| `records::sorted_legal_moves` | 0.14秒 | 訓練局面の記録 |

MCTSの葉の全件生成は8.59秒で、生成スレッドのCPU時間の0.34%、MCTSの駆動部と探索木の29.8%にあたる。
このうち駆動部と探索木に対する29.8%は、提案が一様評価器の命令数から求めた37.9%より小さい。

集計の全体は[profile.json](zero-selfplay-legal-moves-share/profile.json)に、生成器の出力は[run.log](zero-selfplay-legal-moves-share/run.log)と[summary.json](zero-selfplay-legal-moves-share/summary.json)にある。
`perf.data`は約225 MBあるので、リポジトリには置いていない。

### 借りた機械への換算

作業機のGPUは借りた機械のGPUより遅いので、生成スレッドが推論を待つ割合は作業機のほうが大きい。
そこで、`Game::legal_moves`の1手あたりのCPU時間35.6 µsが機械によらないと仮定し、[借りた機械での生成の速さの測定](../alphazero-inference-transfer/alphazero-inference-transfer-generation-speed.md)の、転送の削減をすべて入れた変種Dのスレッドあたりの毎秒の手数に掛けた。
次の表の借りた機械の2行は、この仮定に基づく見積もりであり、実測ではない。
借りた機械のCPUは作業機と異なり、その速さの差は測っていない。

| 機械 | スレッドあたりの毎秒の手数 | `Game::legal_moves`が生成スレッドの時間に占める割合 | その時間を半分にした場合の生成の速さの変化 |
|---|---:|---:|---:|
| 作業機（実測） | 117.5 | 0.42% | 0.2% |
| GPU 1枚の機械（612,681手、214.1秒、4スレッド） | 715.4 | 2.5% | 1.3% |
| RTX 4090を4枚積んだ速い機械（605,751手、105.6秒、16スレッド） | 358.4 | 1.3% | 0.6% |

同じ測定での生成の速さの幅（最大と最小の差を中央値で割った値）は、GPU 1枚の機械の変種Dが0.2%、速い機械の変種Dが3.6%であった。

## 結論

実際の自己対局の生成では、`Game::legal_moves`は作業機の生成スレッドの時間の0.42%であり、借りた機械に換算した見積もりでも1.3〜2.5%にとどまる。
提案の3つの候補がこの費用を半分にしても、生成の速さの変化は作業機で0.2%、借りた機械で0.6〜1.3%であり、生成はGPUの推論の待ちに律速されている。
この結果を受けて、利用者は2026年11月13日に、提案の3つの候補を不採用と決めた。
推論が今より大幅に速くなるなど、探索のCPUの仕事が生成の時間に占める割合が上がったときは、同じ手順で測り直す。
