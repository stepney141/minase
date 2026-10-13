# PSTの比較診断の記録

本書は、基準と候補のPSTを比較する`minase-trainer diag pst-compare`が保存するファイルと、報告の欄を定める。
この診断を学習後の手順のどこで使うかは[PSTとFMの学習手順](../../guides/pst-training.md)にある。

## 比較の条件

`diag pst-compare`は`--base`と`--candidate`の整数PST、`--candidate-float`の実数PSTを比較する。
教師の分類、先読み、λの上書き、対局単位の分割を学習と共有し（[学習データと教師値](data-and-teachers.md)）、両モデルの固定した探索用駒価値が一致することを要求する。
各診断の実装は`crates/minase-trainer/src/diagnostics/`にあり、診断をPythonから移植した経緯は[Pythonの学習器の撤去の設計書](../../plans/python-trainer-retirement.md)にある。

## 保存するファイル

`--output-dir`には新規ディレクトリを指定する。
診断で不合格となった場合も、その時点までの`report.json`と作成済みの資料を残し、終了コード1を返す。

| ファイル | 内容 |
|---|---|
| `report.json` | 基準と候補の比較結果を保存し、失敗時だけ`error`を加える。 |
| `diagnostic-indices-generationN-bandB.npy` | 教師分類Nと位相帯Bの検証標本の通算番号を、昇順のリトルエンディアンi64配列として保存する。空の帯も長さ0の配列を持つ。 |
| `diagnostic-samples.bin` | 全帯の標本を通算番号の昇順に並べたMNSDを保存する。 |
| `representatives.bin` | 初期局面と、各位相帯で最小の通算番号を持つ代表局面をMNSDで保存する。 |
| `removals.bin` | 代表局面から非王駒を1枚ずつ除いた記録を、代表局面順、升順に保存する。 |

## 教師分類と位相帯の比較

`report.json`は`sample_size`、`seed`、`teacher_classes`、`lookahead`、`lambda_override`で条件を記録する。
この診断の`teacher_classes`は7項目に`lookahead_gamma`、`lookahead_plies`、適用した`lambda_override`、実効`lambda`を加えた形であり、検査和は16進文字列である。
`teacher_ks`は分類順の教師用K、`generation_checksums`は分類順の教師ネットの検査和、`k`は基準と候補の出力用K、`piece_values`は固定した47個の探索用駒価値を持つ。
`generation`という欄名も教師の分類番号を表す。
`rescore_exclusions`の`mate_band`、`tactical`、`depth_incomplete`、`total`は、このコマンドではすべて0である。

`bands`は補間係数φを5等分した帯と教師の分類の組ごとに比較する。
φは盤上総駒数から2を引き、0〜90に制限した値を90で割ったものである。
各要素は`generation`、`band`、区間表記`range`、`training_records`、`validation_records`、標本数`samples`、標本ファイル名`indices_file`を持つ。
`validation_loss.base`と`validation_loss.candidate`は、その分類と帯の全検証局面の平均損失であり、空の帯では`null`と理由`reason`を記録する。

各帯から最大`sample_size`件をSplitMix64で非復元抽出し、`base`と`candidate`に教師探索値との標本比較を記録する。
`mae_raw_cp`は生の教師探索値との平均絶対誤差、`mae_scaled_cp`は教師値を`候補の出力用K / 教師用K`倍した後の平均絶対誤差である。
この換算比は基準との比較にも使う。
`correlation`はPearson相関係数であり、定義できない場合は`null`と`correlation_reason`を記録する。
λ=0の分類は教師探索値との比較だけから除き、最上位の`teacher_comparison_excluded`には該当する全検証局面数、各帯の同名欄には該当する標本数を記録する。
検証損失、量子化検査、整数評価の照合にはその分類も含める。

`quantization`は全帯の標本における`mean_absolute_error_cp`、`max_absolute_error_cp`、`samples`、`limit_cp = 2.0`を持つ。
`rust_agreement`は独立した整数PST参照とエンジンの評価が全件一致した件数を`base`と`candidate`ごとに記録する。
平均量子化誤差が2 cpを超えた場合や整数評価が一致しない場合は診断を停止する。

## 終局結果と局面操作

`outcome_metrics`は教師の混合比に依存せず、全検証局面を各モデルの出力用Kで確率へ変換して終局結果と比較する。
`records`、`games`、`output_k`とともに、局面平均の`bce_position_mean`、対局内平均を対局間で等しく平均した`bce_game_mean`を記録する。
対局の識別子は分割と同じ規則を使い、複数ファイルに現れる同一棋譜IDも1対局として集計する。
`sign_agreement`は引き分けと評価0を除いた符号一致率であり、`sign_records`と`sign_matches`を分母と一致件数とする。
`sign_excluded`は除外総数、`draw_records`と`zero_score_records`は重複を含む理由別件数であり、対象がなければ平均と率を`null`にする。

`representatives`は`label`、通算番号`index`、`piece_count`、`phase_numerator`と、基準と候補の評価を持つ。
初期局面の`label`は`"initial"`、`index`は`null`である。
`evaluations`と各除去の`delta_cp`はPST部分、`total_evaluations`と`total_delta_cp`はエンジンの評価全体を表す。
除去の各要素には`square`の升番号、`piece_byte`のMNSD駒コード、手番側0と相手側1の`relative_color`、駒状態番号`state`も記録する。
除去で位相も変わるため、差分をその駒固有の価値と同一視しない。

`pst_removal_sign_reversals`は基準と候補のPST除去差分の厳密な符号反転件数であり、非0なら不合格となる。
`rust_removal_agreement`はモデルごとの一致件数`checked`と復元不能件数`skipped`を記録する。
復元不能な除去記録も`removals.bin`へ残し、該当要素の`skipped.base`と`skipped.candidate`に理由、`total_delta_cp`に`null`を保存する。

`promotions`は代表局面の合法な成り手を実際に適用した結果であり、`move`、`capture`、`promote`、`terminal`と評価差をモデル別に持つ。
着手前の評価をbefore、着手後を着手前の側から評価した値を`after_same`、反対側からの値を`after_opposite`とすると、`delta = −after_opposite−before`、`placement = after_same−before`、`turn = −(after_opposite+after_same)`である。
各欄の`_pst`版はPSTだけの値、`after`は着手後の`board`、`stm`、`lion`、`eval`、`eval_pst`を持つ。
成り手がない局面は`promotion_reason`に理由を記録し、独立参照との照合件数を`rust_promotion_agreement`へ集計する。

`derived_piece_values`は対象の駒状態`states`、固定した探索用駒価値`fixed`、基準と候補それぞれの`middlegame`と`endgame`の全升平均から導出した値を並べる。
欄名は`base_middlegame`、`base_endgame`、`candidate_middlegame`、`candidate_endgame`であり、端点ごとの静的な駒価値と固定値との差を確認できる。
