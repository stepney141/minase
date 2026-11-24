# カーネルの遅さを直したら、同じ書き方をしている別の構造のカーネルも測る

## 症状

v3.0.0-alpha.1で、tatara形式のLayerStackに載せ替えたA型のNNUEの学習器は、載せ替え前の学習器の約半分の速さ（毎秒271,154局面）になった。
1回の更新にかかる60.42 msのうち54 msを、逆伝播で縮約を行う3つのカーネルが占めていた。
同じリリースの開発中に、HalfKP_vmが使うSimpleの構造の学習器では同じ型の遅さを直していた。しかしLayerStackの学習器の速さはその修正の前に測っており、修正の後には測り直していなかった。

## 原因

LayerStackとSimpleの逆伝播のカーネルは、どちらも同じwarpのレーンが別々の局面を読み、1つのブロックが1つの列だけを受け持つ形をしていた。
コミット`264b7fc4`はSimpleのカーネルだけを書き換え、LayerStackのカーネルは元の形のまま残った。
LayerStackは配列が小さく、この遅さでも学習を止めるほどではなかったため、記録には「原因は調べていない」とだけ残った。

## 以後の規則

GPUのカーネルの遅さを直したら、同じ書き方をしている別の構造や入力特徴集合のカーネルを探し、`bench-step --profile`で1回の更新の区間ごとの時間を測る。
同じ型の遅さがあれば同じ修正を当て、両方のカーネルを測り直してから記録する。
速さが落ちた記録に「原因は調べていない」と書いた場合は、その原因を調べる作業を、同じマイルストーンの残りの作業として扱う。

## 出典

- [A型LayerStackの学習器の逆伝播を速くする修正の測定](../measurements/adhoc/layerstack-backward-coalesce.md)
- [HalfKP_vmの動作の確認の記録](../measurements/nnue-tatara-framework-and-halfkp-vm/nnue-tatara-framework-and-halfkp-vm-halfkp-vm-check.md)
