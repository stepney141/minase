# NNUE学習器の速度の調査：tataraが速い理由と現行学習器の律速

本書は、将棋のNNUE学習器tatara（SH11235/tatara、コミット`db08ed2`、MITライセンス）がなぜ高速に学習できるのかをソースコードと設計記録から解明し、minaseの現行のPython学習器（`tools/train`の`train-nnue`）がなぜ遅いのかをプロファイラで測った結果をまとめる。
調査は2026年10月12日に、Claudeのサブエージェント2本とcodexがそれぞれ独立に行い、結論は一致した。
相違は1点だけで、codexが「特徴分解を`simple`構成に適用しないとする設計記録（`docs/decisions/2026-06-12-ft-factorizer.md`）は現状と異なり、現行コードでは`simple`でも既定で有効である（`bins/nnue_train/src/training.rs`2271行）」と指摘した。
本書の結論は、[Rust製NNUE学習器の設計書](../plans/archived/nnue-trainer-rust.md)の根拠として使う。

## 結論

tataraの速さは、Rustという言語の選択ではなく、GPUカーネルとデータ経路の設計に由来する。
既定の経路では、カーネルはnvccでコンパイルしたCUDA C++であり、Rustはホスト側（データ読み込み、特徴抽出、転送、カーネル起動の順序づけ）だけを担う。
比較対象のbullet-shogiも「ホストはRust、カーネルはCUDA C++」の同じ構成であり、tataraが示す+37%はRust同士の比較なので、差を生んでいるのは設計である。
tataraにはCPUだけで学習する経路がなく、GPU機能を外すと学習コマンドを拒否する（`bins/nnue_train/src/main.rs`91〜95行、114〜117行）。

minaseの現行学習器が遅い理由も、PythonやPyTorchという言語や枠組みそのものではなく、第1層の疎な入力を(局面数, 145, 幅)の密な配列へ展開してから足す実装と、駒除去差分の補助損失がその展開を2回繰り返す実装にある。
1更新の壁時間の86〜89%はGPUカーネルであり、CPU側の前処理を無限に速くしても壁時間は1割程度しか縮まない。

## tataraの構成

tataraは7つのcrateと9つのバイナリからなる（`Cargo.toml`1〜19行）。
`nnue-train`がデータローダ、学習率スケジュール、学習ループのホスト側を持ち、`cuda-native-runtime`がCUDA Driver APIへのFFIと`kernels/native_kernels.cu`（2,932行）を持つ。
`gpu-kernels`はGPUカーネルのCPU参照実装だけを置き、GPUとCPUの等価性テスト（`bins/nnue_train/src/tests/gpu_cpu_equivalence_tests.rs`、6,613行）の照合に使う。
当初はRustをPTXへコンパイルするcuda-oxideを採り、その理由は「Rust一言語で完結」と「bullet-shogiの上流追従からの解放」であって性能ではなかった（`docs/decisions/2026-05-09-cuda-oxide-adoption.md`20〜29行）。
その後、Windows対応と導入障壁の低減のためにnvccによるCUDA C++の経路を並行して作り、2026年8月8日にそれを既定にした（`docs/decisions/2026-07-18-native-cuda-backend.md`、`2026-07-30-native-backend-default.md`）。
ツールチェーンはnightly（`nightly-2026-04-03`）に固定され、理由はcuda-oxideのコード生成ABIとf16の言語機能である（`rust-toolchain.toml`）。

## 速さの要因

要因を、計算の経路、データの経路、言語とツールチェーンの3つに分けて示す。
数値はtataraのコミットメッセージと設計記録からの引用であり、いずれもRTX 3080 Tiでの実測である。

### 計算の経路（最大の寄与）

第1層（feature transformer）の前進は、1スレッドが「1局面×連続する4出力」を受け持ち、発火した最大40行の重みをレジスタ内で足し合わせる（`native_kernels.cu`264〜304行）。
保存するのは(局面数, 幅)の結果だけであり、(局面数, 発火数, 幅)の配列を作らない。

第1層の後退は、各発火から勾配の行へ直接アトミック加算する方式ではなく、逆索引による4段の構成である（`bins/nnue_train/src/trainer_layerstack.rs`4193〜4214行）。
特徴ごとの出現回数を数え、前置和を取り、局面を特徴ごとに並べ替えてから、特徴ごとに担当スレッドが勾配を集めて1回で書く（`native_kernels.cu`1818〜1994行）。
アトミック演算は計数と配置の段階だけに残る。
この変更で後退は58 msから30 msになり（コミット`c2ba7f5`）、`simple`構成では全体で+28%だった（コミット`a0816bc`）。

最適化器は、重み減衰、1次と2次のモーメントの更新、更新量、範囲制限、勾配のゼロ化を1つのカーネルにまとめる（`native_kernels.cu`1479〜1515行）。
Ranger、RAdam、AdamWの3種がこのカーネルを共用し、違いはホストが渡すスカラーだけである（`crates/nnue-train/src/optimizer.rs`12〜27行）。
更新は密であり、第1層の全要素を毎ステップ走査する。
疎なAdamや遅延更新は使っていない。

特徴分解（学習時だけ使う玉区分に依存しない仮想行）は、疎な索引へ仮想特徴を追加するnnue-pytorchの方式では約37%遅くなった（1.40M→0.882M局面/秒、`docs/decisions/2026-06-12-ft-factorizer.md`173〜177行）。
tataraは代わりに、毎ステップ`C = W_real + W_virt`を作る畳み込み（fold）と、仮想行の勾配を対応する実行の勾配の和で埋める縮約（reduce）の密なカーネル2本で配線し（`native_kernels.cu`306〜508行）、+52.8%を得た（コミット`1f6d6e5`）。
追加費用は1ステップあたり約1〜2 msである（同設計記録66〜67行）。

既定の精度は純粋なf32でビット単位に再現可能であり、opt-inで第1層の重みと出力、最適化器の状態を半精度にし、密層の行列積にTF32を使える（`bins/nnue_train/src/cli.rs`666〜712行）。
第1層の前進はDRAM帯域の約90%に達しており、半精度化で+19.7%と+20.4%を得ている（コミット`f0592b5`、`aa70772`）。
これらは追加の効果であり、READMEの基準値（f32）の説明にはならない。

### データの経路

教師データは40バイト固定長のPSVを1 MiBの`BufReader`で逐次読みし、mmapは使わない（`crates/nnue-train/src/dataloader.rs`628行）。
既定ではシャッフルせず、ディスク上で事前にシャッフル済みという前提を置く（`docs/decisions/2026-08-09-windowed-teacher-shuffle.md`26〜30行）。
既定16本のstdスレッドがバッチ単位で生データを受け取り、ロックの外で1局面につき1回だけ復号し、その盤面を特徴抽出と局面分類の両方に使う（同1441〜1498行）。
バッチの表現は固定幅の`i32[局面数, 最大発火数]`を両視点ぶん持ち、局面ごとの実発火数を添えてカーネルはその数までしか読まない（同380〜409行）。
バッチ用の領域はプールで使い回し、1バッチあたり約21 MBの確保を繰り返さない（同1214〜1218行、1384〜1407行）。

GPUへの転送は、2面のページ固定（pinned）ホストメモリへ写してから専用の転送ストリームで非同期に行い、デバイス側の入力も2面で切り替えて、転送が直前ステップの計算と並走する（`bins/nnue_train/src/trainer_common.rs`1019〜1209行、`trainer_layerstack.rs`2511〜2539行）。
損失は1ステップ遅れでページ固定メモリへ非同期に読み出し、ステップごとの同期を置かない（`trainer_common.rs`888〜994行）。
借用による不要なコピーの削減でステップ時間が120 msから100 msに（コミット`c2ba7f5`）、転送の重ね合わせで+3.2〜6.7%（`3f9e206`）、非同期の損失回収で+1.16%（`bf42b02`）だった。
データ経路が律速になるかは構成に依り、`layerstack`構成ではワーカー数を8から32に変えても速度は横ばいだった。

### 言語とツールチェーン

既定の経路でRustが担うのはホスト側だけであり、「Rustであること」に由来する速度の寄与を数値化する根拠はない。
Rustがマイナスに働いた記録はある。
cuda-oxideのカーネルでは境界検査が1段階の約24%を占め、生ポインタへ置き換えて+6.1%だった（コミット`c2ba7f5`）。
CPU側のSIMD化は端から端で−2.2%と誤差の範囲であり、GPUが飽和しているため効いていない（コミット`338ff91`）。
Rustに由来し得る利点は、借用によるコピー削減、GILのないstdスレッド、`#[target_feature]`による実行時の振り分けだが、いずれも設計次第でC++でも得られる。

## 実測値の読み方

READMEの表（`README.ja.md`36〜40行）は、バッチ65,536で、RTX 3080 Tiの`simple`構成（HalfKP、256x2-32-32）が7.30M局面/秒、`layerstack`構成（HalfKA-hm-merged、幅1536）が0.99M局面/秒である。
この表はコミット`7e8adf1`（2026年7月3日）で書かれ、現在既定のnvcc経路が既定になる前のcuda-oxide経路の実測である。
nvcc経路での同等の数値はリポジトリ内にない。
minaseの現行学習器の27,021局面/秒（P型、幅128、バッチ16,384、RTX 3060 Ti）と並べると約270倍の差になるが、GPU、バッチ、1視点の発火数（tataraのHalfKPは最大38、minaseは最大145で平均約48）、損失（tataraは勝率モデル損失、minaseは二値交差エントロピーに駒除去差分の補助損失を加えたもの）が異なるので、この倍率をそのまま期待値にはできない。

## 先行実装との比較

bullet（手元の複製は`~/board-games/BulletOu-minishogi`に同梱されたフォーク、`shogi-support`ブランチ、HEAD `cc202f3`）は、カーネルをRustの`format!`で組み立てたCUDA C++の文字列として実行時にNVRTCでコンパイルする（`crates/gpu/src/pointwise/write.rs`212〜284行、`crates/gpu/src/runtime/cuda.rs`209〜236行）。
第1層の後退は`atomicAdd`で、勾配が0なら書き込みを省き、勾配バッファは常駐させてカーネル前にゼロ埋めする。
最適化器の更新は1カーネルに融合され、バイアス補正はない（`crates/trainer/src/optimiser/adam.rs`33〜139行）。
GPUなしでは学習できず、feature未指定では型検査用の空実装が選ばれる（`crates/bulletou_lib/src/nn.rs`8〜15行）。

variant-nnue-pytorch（`~/board-games/variant-nnue-pytorch`、HEAD `749db94`）は、第1層をCuPyの`RawKernel`で書いたCUDAカーネルで計算し、`autograd.Function`でPyTorchにつなぐ（`feature_transformer.py`）。
前進は1ブロック＝1局面で、後退は`atomicAdd`であり、呼び出しのたびに重みと同じ大きさの`torch.zeros`を新しく確保する（同266〜270行、353行、464行）。
特徴分解は疎な索引の列に仮想特徴を追加する方式である（`training_data_loader.cpp`111〜152行）。
文書は「カスタムカーネルで2〜4倍速くなった」と記す（`docs/nnue.md`2085行）。

したがって、疎なgather、逆索引またはアトミックによる後退、融合した最適化器、NVRTCによる実行時コンパイルは、この分野の学習器に共通する設計であり、tataraはそのうち後退の逆索引化と特徴分解のfold/reduceで一歩進んでいる。

## minaseの現行学習器の律速

現行学習器の1更新（幅128、バッチ16,384、駒除去差分の分割2,048）を、`torch.profiler`と手動のタイマーで測った（2026年10月12日、RTX 3060 Ti、CPUは`nice -n 19 taskset -c 18,19`の2コアに制限、教師データの生成が12コアで並走）。
スクリプトと出力は本環境の作業領域にあり、リポジトリには置いていない。

| 区分（1更新あたり、ms） | P型 | HalfKP-hm-b8 |
|---|---:|---:|
| 壁時間 | 611 | 788 |
| memmapからの収集 | 5.3 | 5.0 |
| numpyによる特徴番号の計算 | 47.0 | 87.4 |
| CPUからGPUへの転送（CPU側の壁時間） | 5.7 | 5.4 |
| GPUカーネルの合計 | 543 | 681 |
| 　うち第1層の前進（gather） | 19 | 34 |
| 　うち第1層の後退（scatter） | 93 | 168 |
| 　うち駒除去差分の補助損失（第1層を除く） | 320 | 342 |
| 　うちその他（要素ごとの演算、スライスの後退のゼロ埋めとコピー、損失、更新、切り詰め） | 111 | 137 |
| 最大GPUメモリ（MiB） | 1,269 | 1,589 |

壁時間からGPUカーネルの合計を引いた差（68 msと107 ms）は、CPU側の3区分の和（58 msと98 ms）にほぼ一致する。
すなわち壁時間は「GPUカーネル時間＋CPUのデータ準備」の直列の和であり、各分割のページ可能メモリからの転送と`.item()`系の同期（P型で1更新116回）が、前の分割のGPU作業を排水してから次へ進むため、CPUの準備とGPUの実行が重ならない。

GPU時間の内訳では、本来の計算である第1層のgather（8 ms）とscatter（47 ms）は小さく、(2,048, 145, 128)の配列を何度も読み書きする帯域律速の要素ごとの演算が支配的である。
駒除去差分の補助損失の経路が最大で、補助損失の係数を0にすると更新はP型で2.7倍、HalfKP-hm-b8で2.3倍速くなる（611→227 ms、788→342 ms）。
この経路は`hidden[:, None, :] - rows[:, :144]`以降の切り詰め、二乗、乗算、和とその後退に加え、`removal_deltas`が第1層を再び呼ぶため、第1層の前進と後退が2倍になる。
次に大きいのはスライスの後退であり、`rows[:, :144]`や`board[:, :, :6]`のようなスライスの後退が(2,048, 145, 128)の単精度配列（152 MB）をゼロ埋めして勾配をコピーするため、補助損失を外しても基本経路のGPU時間の約半分を占める。
HalfKP-hm-b8がP型より177 ms遅い内訳は、numpyの特徴番号の計算が40 ms増、第1層の後退が75 ms増（特徴の行と仮想行の2つの表）、補助損失が67 ms増である。

## 設計への含意

以上から、Rust製の学習器が速さを得るために取り入れる設計は次のとおりである。

- 第1層の前進は、発火した行をレジスタ内で足し合わせ、(局面数, 発火数, 幅)の配列を作らない。
- 第1層の後退は、バッチ内の発火を特徴番号で並べ替えた逆索引を使い、特徴ごとに固定順で足して1回で書く。アトミック演算を使わないので、CPU参照実装との等価性を厳密に近い許容誤差で検査でき、学習の再現性も得られる。
- 駒除去差分の補助損失は、除去ごとの(幅)の中間配列を実体化せず、除去1件あたりのスカラーの勾配だけを保持して、第1層の後退で行ごとに再計算する。現行の最大の費用がここにあり、tataraにもbulletにも相当物がないので、minaseが自前で設計する唯一の部分である。
- 最適化器の更新、切り詰め、未観測行の固定、特徴分解の畳み込みを1つのカーネルにまとめ、仮想行の縮約を密な1パスで行う。
- データ経路は、ワーカースレッドで復号と特徴抽出と逆索引の構築を行い、ページ固定メモリの複数面と転送専用ストリームで転送を計算と重ね、損失は1ステップ遅れで回収する。
- tataraの「事前シャッフル済みの逐次読み」は使えない。MNSDは対局順に並び、検証分割は対局番号のハッシュで決まるので、エポックごとの順列による乱択アクセスが要る。

取り入れない設計は、LayerStack（隠れ層と出力バケット。minaseの出力部は隠れ層のない直接内積）、勝率モデル損失（既定値がチェス向け）、半精度化とTF32（f32の設計を測ってから別に判断する）、nightlyのツールチェーンとcuda-oxide（stable RustとNVRTCで足りる）である。

## 参考資料

- tatara: `README.ja.md`、`docs/decisions/2026-05-09-cuda-oxide-adoption.md`、`2026-05-09-fused-kernel-strategy.md`、`2026-06-12-ft-factorizer.md`、`2026-07-18-native-cuda-backend.md`、`2026-07-30-native-backend-default.md`、`2026-08-09-windowed-teacher-shuffle.md`、`crates/nnue-train/src/dataloader.rs`、`crates/cuda-native-runtime/kernels/native_kernels.cu`、`bins/nnue_train/src/trainer_common.rs`、`bins/nnue_train/src/trainer_layerstack.rs`、`bins/nnue_train/src/trainer_simple.rs`
- bullet（BulletOu同梱のフォーク、HEAD `cc202f3`）: `crates/gpu/src/pointwise/write.rs`、`crates/gpu/src/runtime/cuda.rs`、`crates/trainer/src/optimiser/adam.rs`、`crates/trainer/src/run.rs`、`crates/bulletou_lib/src/value/loader.rs`
- variant-nnue-pytorch（HEAD `749db94`、作業ツリーに未コミットの変更あり）: `feature_transformer.py`、`training_data_loader.cpp`、`docs/nnue.md`
- minase: `tools/train/src/minase_train/nnue/train.py`、`nnue/model.py`、`pst/removal.py`、[学習の記録](../measurements/nnue-replacement-training.md)
