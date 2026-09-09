# FMの配置変化と手番依存性の診断

## 目的

既存FM候補が対局で弱くなる原因を絞るため、保存された教師局面と候補の対局局面で、静的なFM補正、着手差の配置成分、および手番成分を比較する。
採否測定ではなく、新しい学習や対局は実施していない。

## コマンドライン

独立したローカル複製に作ったfm-evalブランチのworktreeで実行した。
以下のarchive_dirは元リポジトリの読み取り専用入力を指し、生成物は作業中のworktreeのdataへ置く。

```sh
archive_dir=/home/stepney141/board-games/minase/data/fm-eval-archive
cargo build --offline --release --bin pst_probe --bin match_positions
target/release/match_positions --run-dir "$archive_dir/matches/fm-adoption-stc" --max-pairs 16 --pst nets/pst.bin --output-dir data/fm-match-positions
python tools/train/pst/fm_strength_diagnostics.py --positions "$archive_dir/fm-cand-lam1e-4/diagnostics/representatives.bin" --probe target/release/pst_probe --pst nets/pst.bin --output-dir data/fm-strength-representatives-v2 --sample-size 6 --seed 1 --moves
python tools/train/pst/fm_strength_diagnostics.py --positions "$archive_dir/fm-cand-lam1e-4/diagnostics/diagnostic-samples.bin" --probe target/release/pst_probe --pst nets/pst.bin --output-dir data/fm-strength-saved-v2 --sample-size 512 --seed 1 --moves
python tools/train/pst/fm_strength_diagnostics.py --positions data/fm-match-positions/positions.mnsd --probe target/release/pst_probe --pst nets/pst.bin --output-dir data/fm-strength-match-v2 --sample-size 512 --seed 1 --moves
python tools/train/pst/fm_strength_diagnostics.py --positions "$archive_dir/fm-cand-lam1e-4/diagnostics/diagnostic-samples.bin" --probe target/release/pst_probe --pst nets/pst.bin --output-dir data/fm-strength-saved-static-all --sample-size 91841 --seed 1
python tools/train/pst/fm_strength_diagnostics.py --positions data/fm-match-positions/positions.mnsd --probe target/release/pst_probe --pst nets/pst.bin --output-dir data/fm-strength-match-static-all --sample-size 7002 --seed 1
python tools/train/pst/fm_distribution_comparison.py --saved-dir data/fm-strength-saved-static-all --match-dir data/fm-strength-match-static-all --output data/fm-distribution-comparison.json
```

出力先は新規作成を要求するので、再測定では既存ディレクトリを上書きせず別の出力先を指定する。
抽出番号、入力とバイナリのSHA-256、および生の評価値は各ディレクトリのreport.json、probe.json、sample.binへ保存した。
主要な集計と来歴は[測定データ](fm-search-diagnosis.json)にも保存している。

## エンジン

FM候補は既存採否測定の`168e0dc4a8615695f22edf11a5c56da37b04166c`に含まれる重みであり、ファイル全体のSHA-256は`962e22432f622e526c4f4bfcbad2f053994002afb6a6a4381a9ad25b4a337cb1`である。
FMの潜在次元は32、教師探索値の比率は1、補正量の正則化係数は0.0001である。
比較するPSTは同じ重みファイルに固定された2端点PST部分であり、基準`b0153cf27df655a32cb9a255fd380b21bba002b5`の評価と一致することが[既存診断](fm-candidate-diagnostics.md)で確認されている。
診断実装はfm-evalの`fb491b4`を起点とし、通常探索の評価式や枝刈りを変更していない。
規則はengine-defaultであり、入力MNSDの規則が意味的に一致しない場合は診断を拒否する。

## 環境

2026年9月9日にLinux、Intel Core Ultra 7 265KF上で実行した。
Rustは1.98.0、診断はPython 3.14.7とNumPy 2.5.2であり、評価と集計はCPUで行った。
計算結果は思考時間に依存せず、新規の対局や学習、速度比較は実施していない。
既存の学習関連テストには元リポジトリのPython 3.12仮想環境を使用した。
Rustの全614テスト、Pythonの全65テスト、フォーマット検査、およびClippyが通過した。
符号を反転する変更と手番照合を除く変更が、それぞれ対応テストで検出されることも確認した。

## 結果

### 標本と再現性

既存の代表6局面では全392手のFMによる着手差の標準偏差113.055、最大絶対値417センチポーンを再現した。
保存標本は91,841局面から512局面をシード1で復元抽出なしに一様抽出した。
対局標本はペア番号順の先頭16ペアの32局を勝敗で選別せず再生し、候補の手番でセンチポーン探索値のある7,002局面から同様に512局面を抽出した。
32局すべてで着手の合法性、手番、終局理由、および勝者が元記録と一致し、候補の非センチポーン評価73件を除外した。
評価欠損は0件だった。

対局由来MNSDの探索値は候補自身の時間制御下の記録であり、独立した教師値として使わない。
ヘッダのteacher_nodes=0は固定ノード教師が存在しないことを表す診断上の記録であり、探索予算を表さない。
局面単位で抽出するため長い対局は重くなり、7,002局面を独立した7,002対局とは扱わない。

| 標本 | 局面数 | 非終局の静かな手 | 非終局の捕獲または成り | 終局手 |
|---|---:|---:|---:|---:|
| 代表 | 6 | 379 | 13 | 0 |
| 保存 | 512 | 37,703 | 2,446 | 0 |
| 対局 | 512 | 37,548 | 2,800 | 0 |

ここで静かな手は捕獲でも成りでもない手を指し、futility pruningの全適用条件を満たすという意味ではない。
MNSDに反復履歴と駒枯れの猶予状態は保存されないため、上表の終局判定は各局面から新たに対局を開始して計算したものである。
元の対局全状態での終局判定を置き換えるものではない。

### 手番成分と配置成分

着手前の視点をt、相手の視点をo、着手前後の配置をBとB'とする。
各評価について、着手差D=-E_o(B')-E_t(B)、配置成分P=E_t(B')-E_t(B)、手番成分T=-E_o(B')-E_t(B')を計算し、D=P+Tを確認した。
以下は各成分の「FM込み評価からPSTだけの評価を引いた差」を測り、各局面で静かな手の絶対値を平均した後、局面へ等重みを付けて平均した値である。
単位はセンチポーンである。

| 標本 | 配置成分 | 手番成分 | 着手差 |
|---|---:|---:|---:|
| 保存512局面 | 40.80 | 85.76 | 93.03 |
| 対局512局面 | 61.83 | 153.79 | 165.48 |

手番成分の平均絶対値は配置成分より大きく、対局標本ではさらに増えている。
盤上駒数による5帯のうち帯1から4でも対局側の手番成分が大きく、帯0では逆だった。
FM込み評価全体の手番成分の絶対平均も、静かな手に等重みを付けた場合、保存標本のPST55.60に対してFM74.39、対局標本のPST68.75に対してFM145.49へ増えている。

ただし、手番成分だけが着手順位の不安定さの主因だとは言えない。
同じ局面内の静かな手について、FM寄与の標準偏差を局面ごとに求めて平均すると、保存標本では配置55.31に対して手番44.67、対局標本では配置79.79に対して手番63.43であり、配置成分の方が大きい。
同じ局面の全候補へ共通に加わる成分は着手順位を変えないため、平均絶対値と候補間の変動は区別する必要がある。
また、成分は相殺し得るうえ、分解は着手後の配置で視点を交換する経路に依存するので、独立した因果寄与率とは解釈しない。

### 対局でFM補正が正側へ偏る

静的評価を保存91,841局面と対局7,002局面の全件で比較すると、対局側ではFM補正の98.51%が正だった。
対局側は候補の手番だけを取り出した標本であり、補正は候補自身の評価を高くする方向に偏っている。

| 標本 | FM補正の平均 | 中央値 | 正の補正の割合 |
|---|---:|---:|---:|
| 保存91,841局面 | +0.92 | 0 | 49.82% |
| 対局7,002局面 | +956.25 | +895 | 98.51% |

粗い局面構成の差を調整するため、駒数の5帯とPST評価500センチポーン刻みで共通層を作り、対局側の層頻度で保存側の平均を標準化した。
共通103層は対局7,002局面の100%と保存82,033局面の89.32%を覆い、標準化後の保存側平均は−60.09、対局側は+956.25、差は+1,016.34センチポーンだった。
この層分けで調整した後も大きな差が残るが、500センチポーン幅の層内のPST評価差や細かな駒配置は調整していない。

PST評価が[-1,000, 1,000)センチポーンの範囲でも、共通20層で保存38,300局面と対局3,058局面を比較できた。
標準化後の保存側平均は−4.76、対局側は+828.37、差は+833.13センチポーンであり、20層すべてで対局側が大きかった。
各層の保存局面は最低447件、対局局面は最低8件であり、この中央範囲の差は保存側の極端に疎な層だけに依存するものではない。

## 結論

今回の測定は、FMが自分の補正を高くする配置へ探索を誘導するという仮説を具体的に支持する。
保存された教師局面で補正が有効でも、候補が実際に選ぶ局面では補正の分布が大きく変わるため、平均的な検証誤差だけで候補を選ぶ方法には限界がある。
対戦相手を含む生成条件も異なるので、FM探索単独の因果効果を識別した結果ではない。
ただし大きな正の補正が実際の強さを正しく反映している可能性までは、この静的診断から排除できない。
自己選択による誤評価かを確定するには、到達局面とその候補手を独立した十分深い教師探索で評価し直す必要がある。

次に優先するのは、今回の対局標本と局面構成を揃えた保存標本を独立教師で再評価し、補正が大きい局面での過大評価と候補手順位の誤りを測ることである。
手番成分を抑える改変や枝刈りの緩和は、その結果と分けて検証する。
今回の測定だけを根拠にFMの反対称化、学習データの増量、または探索定数の変更を採用しない。
