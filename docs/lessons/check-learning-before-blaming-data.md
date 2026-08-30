# 教師不足と判定する前に学習曲線を確認する

## 症状

評価関数マイルストーンは、100万局面で学習したP型NNUEが学習PSTに`H0`となった敗因を教師不足と判定し、教師局面を1桁増やす世代反復を次期マイルストーンとして起案した。
しかし1,106万局面で同じ学習器を20エポック回しても検証損失は0.00005しか動かず、約20時間の生成と学習は判定の検証以外に何ももたらさなかった。

## 原因

学習器の第1層の初期化がclipped ReLUを94.5%飽和させて勾配を止めており、ネットはデータ量にかかわらず学習できていなかった（[nnue-first-layer-init-saturation.md](nnue-first-layer-init-saturation.md)）。
学習できないネットの損失が動かないのは当然であり、その停滞は教師不足の証拠にならない。
教師不足は、学習が成立しているネットが早期に過学習へ転じるときに初めて観測され、実際に初期化を直した学習器は6エポックで最良に達した後に過学習した。

## 以後の規則

学習の停滞をデータ量のせいにする前に、訓練損失が下がり続けることと検証損失が最良を過ぎて上昇へ転じる（過学習する）ことを学習曲線で確認し、どちらも見られなければ学習器の欠陥をまず疑う。
データ増量を対策として起案するのは、過学習が観測された後に限る。

## 出典

- [plans/evaluation.md](../plans/evaluation.md) の世代0の判定と [measurements/nnue-gen0-nodes100k.md](../measurements/nnue-gen0-nodes100k.md)。
- [plans/evaluation-gen1.md](../plans/evaluation-gen1.md) の「教師不足の判定の検証結果と次期候補」節。
- [nnue-first-layer-init-saturation.md](nnue-first-layer-init-saturation.md)。
