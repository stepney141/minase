# ビルドのLLVMインライン閾値を1000へ上げる

## ステータス

承認済み。決定日は2026年11月5日。

## コンテキスト

[教師生成の高速化](../plans/archived/teacher-generation-speedup.md)の調査で、手生成と利き計算の小さな関数（`piece_control_without_special`、`sliding_control`など）がインライン展開されずに関数呼び出しのまま残り、`Bitboard`を値で受け渡す費用が命令数の増加として現れていた。
workspaceのreleaseプロファイルはLTOと`codegen-units = 1`を使っており、残る調整項目はLLVMのインライン閾値である。
PGOは開発者の決定により使わない。

## 決定

`.cargo/config.toml`の`rustflags`に`-C llvm-args=-inline-threshold=1000`を加え、全ビルドへ適用する。

## 検討した代案

閾値ごとの測定は[記録](../measurements/teacher-generation-speedup/teacher-generation-speedup-engine.md)にある。
閾値400はサイクルの減少が小さく、閾値2500は閾値1000と比べてサイクルが減らず、実行ファイルが大きくなるだけだった。
関数ごとに`#[inline(always)]`を付ける案は、対象の選択を保守し続ける必要があり、探索を変更するたびに効果を確かめ直すことになる。
`panic = "abort"`ではサイクルが減らなかった。

## 結果

探索結果は変わらず、ノードあたりの命令数が12.5%、サイクル数が10.3%減った。
フラグは`.cargo/config.toml`に置くので、生成機を含むすべてのビルドに同じ設定が入り、`dev`プロファイルと学習器にも適用される。
`minase`と`minase-core`のテストの全ビルドは1分19秒から1分43秒へ、workspace全体のreleaseビルドは53秒から1分16秒へ延びた。
