# 対局の実行器によるαβ探索のネットの記録と照合

## 目的

同じコミットのαβ探索を候補と基準にして同じネットを`EvalFile`で渡したとき、対局の実行器`minase match run`が両側の`network_sha256`を`manifest.json`に記録すること、および途中で止めた測定の基準のネットを別のネットに差し替えて再開すると照合の不一致で止まることを確かめる。

## コマンドライン

記録の確認には、決まったペア数（1ペアは同じ開始局面で先後を入れ替えた2局）を指す実行の型`elo --pairs 2`を使った。
`<NAS>`は`/mnt/synology_nas/minase/nnue/nets/987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd.bin`、`<基準>`は、後から書き換えられるように`<NAS>`を同じバイト列のまま一時ファイルへ複写したものである。

```console
target/release/minase match run --run-dir data/matches/nnue-external-network-file-manifest --seed 20261117100 \
  --candidate commit:e91c4f5e --baseline commit:e91c4f5e \
  --candidate-option EvalFile=<NAS> --baseline-option EvalFile=<基準> \
  --rules JCSA --each depth=2 --concurrency 2 elo --pairs 2
```

完了した測定の再開はエンジンを起動せずに集計を出すだけなので、照合の確認には別の実行ディレクトリで20ペアの測定を始め、1ペア目が確定した時点でSIGTERMで止めた。
続けて`<基準>`を固定ネット`crates/minase/nets/nnue-epoch0.bin`（SHA-256 `cc61415498636b74…`）で上書きし、再開した。

```console
target/release/minase match run --run-dir data/matches/nnue-external-network-file-manifest-resume-2 --seed 20261117300 \
  --candidate commit:e91c4f5e --baseline commit:e91c4f5e \
  --candidate-option EvalFile=<NAS> --baseline-option EvalFile=<基準> \
  --rules JCSA --each depth=2 --concurrency 1 elo --pairs 20
target/release/minase match run --resume data/matches/nnue-external-network-file-manifest-resume-2
```

これより前に、実行ディレクトリ`nnue-external-network-file-manifest-resume`（シード20261117200）で同じ手順を試したが、バックグラウンドで起動したプロセスにSIGINTが届かず20ペアを完走したので、照合の確認には使っていない。

## エンジン

候補と基準はどちらもコミット`e91c4f5e`（[外部ファイル化の設計書](../../plans/archived/nnue-external-network-file.md)のフェーズ2の後）であり、実行ファイルのSHA-256は`a4e31a21…`である。
規則は`JCSA`、思考制限は両側とも深さ2である。

## 環境

測定機はIntel Core Ultra 7 265KF（20コア、20スレッド）、メモリ30 GiBである。
同時対局数は記録の確認で2、照合の確認で1であり、`USI_Hash`はエンジンの既定の256 MiBである。

## 結果

記録の確認の2ペアは、ペンタノミアル度数[0, 0, 2, 0, 0]、破棄ペア0、異常0件で終わった。
`manifest.json`には、候補と基準の両方に`network_sha256: 987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd`が記録され、オプションの欄には渡した`EvalFile`のパスが残った。

照合の確認では、1ペア目の確定後に止めた測定を、基準のネットを差し替えて再開すると、エンジンを事前に起動した時点で次の行を出して終了コード1で終わった。

```text
failed to restore experiment: baseline engine: network_sha256 mismatch: recorded 987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd, received cc61415498636b740a8fe2281537be5f6e437440b19d674bedccf4258d9b714f
```

再開の前後で`pairs/`の記録は1件のままだった。

## 結論

αβ探索のエンジンが`isready`で報告するネットのSHA-256は、PV-MCTSと同じ仕組みで記録され、再開時の事前の起動で照合される。
