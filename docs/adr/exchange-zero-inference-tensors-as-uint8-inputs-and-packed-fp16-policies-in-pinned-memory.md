# AlphaZero型探索のネットの推論は、入力をuint8で送り、方策を取り得る16,108ラベルのFP16で受け取り、出力を呼び出しごとにピン留めメモリへ結び付ける

## ステータス

承認済み。決定日は2026年11月11日。

## コンテキスト

ニューラルネットワーク（以下、ネット）の推論は、1局面あたり、float32の入力（159面×12×12、91,584バイト）をホストからGPUへ送り、float32の方策の出力（35,856要素、143,424バイト）と価値の出力（1要素）をGPUからホストへ戻していた。
どちらも、ONNX Runtimeの`session.run`が扱うページング可能なメモリ（OSが退避できる普通のメモリ）を通っていた。
GPUを時間単位で貸すサービスvast.aiで借りた、RTX 4090を4枚積んだ機械では、自己対局の生成の速さが借りた機械によって約2.7倍違い、遅い機械では、ページング可能なメモリを通るGPU→ホストの転送が4枚の合計で毎秒約7 GBで頭打ちになっていた（[複数GPU化のvast.aiの測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-vast.md)）。
入力の159面はすべて0か1であり、方策の35,856ラベルのうち盤面によらず合法手になり得るのは16,108個である（[マスクのADR](mask-impossible-policy-labels-and-diagnose-the-zero-trainer-on-final-weights.md)）。
dlshogi（ふかうら王）、Lc0、およびKataGoのCUDAの実装は、入出力をピン留めメモリ（OSが退避しないメモリ）に置き、Lc0は方策を固定の表で取り得る着手だけに詰め、FP16で返すネットも持つ。
minaseは`unsafe_code = "forbid"`なので、独自のCUDAカーネルと、自分で確保したメモリのポインタから作るテンソルは使えない。どちらも`unsafe`を要するからである。
決定の経緯は[推論の転送を減らす設計書](../plans/archived/alphazero-inference-transfer.md)にある。

## 決定

推論の入出力を、次の4点で改める。

1. TensorRTとCUDAの実行プロバイダでは、ONNX Runtimeの`IoBinding`（推論の前に入力と出力のメモリの置き場所を指定する機能）で、方策と価値の出力をCUDAのピン留めメモリへ結び付けて受け取る（pinned host buffer）。`IoBinding`の束縛は推論器が1つ持ち続け、出力の結び付けだけを推論の呼び出しごとに消して結び付け直す。ピン留めメモリを指定するときの機器番号には、その推論器が使うGPUの機器番号を渡す。入力は今までどおりホストの通常のメモリから送り、CPUの実行プロバイダは`session.run`のままとする。
2. 入力はuint8の`[バッチ, 159, 12, 12]`で送り、ONNXのグラフの先頭の`Cast`でfloat32へ変換する（uint8 input）。局面を面へ符号化するRustの関数は、推論器と、Pythonの学習器が呼ぶ拡張モジュールが共有している。この関数はuint8の配列へ書き、拡張モジュールはその配列をfloat32へ変換して学習器へ返す。
3. 方策の出力は、グラフの最後の`Cast`でFP16へ変換して返す（FP16 output）。価値はfloat32のまま返す。
4. 方策の出力は、定数マスク`policy_mask`が真である16,108個の添字を昇順に並べた表で`Gather`して詰める（policy map）。表はONNXのメタデータ`policy_labels`に書く。推論器はセッションを作るときに表を読んで、添字が昇順であること、35,856未満であること、および要素の数が出力の幅と等しいことを確かめ、元のラベルから詰めた後の添字を引く逆引きの表を作る。推論のときに合法手のラベルが表になければ、推論をエラーにする。

`Cast`と`Gather`は、学習器がONNXを書き出すときだけネットにかぶせる薄いモジュールに置き、学習の順伝播と損失は変えない。
書き出したネットの推論の出力は、学習器がPyTorchで計算した参照の出力と、288局面の合法手のlogitと価値で照合する。
ONNX RuntimeのCPUでの照合は、参照のlogitをFP16へ丸めてから比べ、絶対誤差1×10⁻⁴にFP16の1目盛りを足した範囲を許す。
TensorRTとCUDAの照合は、今までどおり合法手の確率1×10⁻²、価値5×10⁻²とする。
4点の採否は、対局での逐次確率比検定（SPRT）ではなく、自己対局の生成の速さと照合の許容誤差で決めた。4点はネットも探索の規則も変えず、推論の値を照合の許容誤差の中で変えるだけだからである。

## 検討した代案

出力の結び付けを1回だけ行って呼び出しの間で保つ方式は採らなかった。
ONNX Runtime 1.31.1では、TensorRTでもCUDAでも、2回目の`run_binding`が`gpu_data_transfer.cc`の「転送の元と先が同じ領域である」という検査で失敗した（[作業機での確認](../measurements/alphazero-inference-transfer/alphazero-inference-transfer-workstation.md)）。

入力もピン留めメモリに置く方式は採らなかった。
ONNX Runtime 1.31.1では、入力をピン留めメモリに置くと、TensorRTが入力をGPUのメモリへ転送せずにホストのメモリを直接読む動作になり、Lc0は、直接読む方式が遅いと記している。

合法手のラベルの並びを局面ごとにGPUへ送り、GPUの上で合法手のlogitだけを取り出して戻す方式は採らなかった。
先行実装に例がなく、合法手の数の上限も分かっていないからである（2026年11月11日の開発者の決定）。

dlshogiとLc0のように入力をビットで詰めてGPUで展開する方式、CUDAのストリームの指定、および自分で確保したメモリのポインタから作るテンソルは採らなかった。
どれも`unsafe`を要する。

方策を詰める添字の表をRustの側にも定数として持つ方式は採らなかった。
表をネットのファイルに持たせれば、RustにPythonと同じマスクの規則を書き写さずに済み、書き出しと推論で表が食い違わない。

## 結果

決定の1から4を番号の順に積み重ねて1つずつ測ると、RTX 4090を4枚積んだ遅い機械の自己対局の生成の速さは、毎秒426局面から、594、1,005、1,228、1,306局面へと上がり、合わせて約3.07倍になった。
同じ構成の速い機械でも、毎秒1,171局面から1,424局面へ、約1.22倍になった。
ColabのG4と同じGPU（RTX PRO 6000 Blackwell Server Edition）を1枚積んだ機械では、4点を入れると毎秒581局面から712局面へ、約1.23倍になった（[生成の速さの測定](../measurements/alphazero-inference-transfer/alphazero-inference-transfer-generation-speed.md)）。
方策をFP16へ丸めたことによる合法手の確率の変化は、学習したネットで最大9.3×10⁻⁴であった。

ONNXの入力と出力の型と形が変わったので、これより前に書き出したネットは新しい推論器で読めない。
保存する形式の版は1つに保つので（[保存形式のADR](keep-one-version-per-saved-format-and-allow-run-private-scratch-files.md)）、必要なネットは学習器の`export-onnx`でチェックポイントから書き出し直す。
書き出し直すとONNXのSHA-256が変わり、TensorRTのエンジンのキャッシュも作り直される。
[推論の方式のADR](run-zero-network-inference-through-ort-load-dynamic.md)のうち、TensorRTとCUDAでも推論を`session.run`の経路で行う点と、`IoBinding`を採らない点は、本書が置き換える。
[入力のADR](encode-zero-network-inputs-as-159-planes-with-pseudo-control-from-the-side-to-move.md)と[方策のADR](encode-the-zero-policy-as-249-planes-per-from-square.md)が定める面とラベルの意味は変わらず、変わるのは受け渡しの型と、方策の出力の幅だけである。
入力の符号化または方策のラベルを変えるときは、定数マスクと、そこから作る表も合わせて見直す必要がある。
