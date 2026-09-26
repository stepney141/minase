# Magic bitboardの一次資料

12×12盤へ適用するなら、飛車や角の全方向を1表にまとめる方式より、縦線、横線、各斜線へ分割する方式を先に比較する価値がある。
大盤用のFairy-Stockfishにも表の分割例があり、Stockfishとやねうら王には表を使わず減算で利きを求める実装もある。
これらは実装可能性の根拠であり、minaseでの高速化率を示すものではない。
2026年9月21日に確認した固定コミットのソースを以下に記す。

## 参照表の添字

走り駒の利きは、移動方向上の占有状態から事前計算した表を引く方法で求められる。
**Magic bitboard**は、必要な占有ビットをマスクし、選んだ定数との乗算とシフトで表の添字を作る方式である。
Stockfish 17では、64ビット版の添字を `((occupied & mask) * magic) >> shift` としている。
**PEXT（Parallel Bits Extract）**は、マスクされたビットを下位へ連続して抽出する命令であり、同版では乗算の代わりにPEXTで添字を作る経路もある。
両者は同じく参照表を使うが、添字の計算方法が異なる。[Stockfish 17の添字計算](https://github.com/official-stockfish/Stockfish/blob/e0bfc4b69bbe928d6f474a46560bcc3b3f6709aa/src/bitboard.h#L67-L87)

乗算用の定数は、異なる利きが同じ添字へ写らないことを全占有状態で確かめて選ぶ。
同じ利きになる状態同士の衝突は許せるが、任意の定数を選ぶだけでは正しい表にならない。
Stockfish 17は、盤端を除いたマスクの部分集合を列挙し、参照値との不一致があれば定数を選び直している。[Stockfish 17の表生成](https://github.com/official-stockfish/Stockfish/blob/e0bfc4b69bbe928d6f474a46560bcc3b3f6709aa/src/bitboard.cpp#L145-L217)

## 大盤への分割

Fairy-Stockfishの大盤用実装は12×10盤を128ビットで表すため、minaseの144升へそのまま移せる実装ではない。
ただし、128ビットのPEXTを下位64ビットと上位64ビットに分けて実行し、下位マスクのビット数だけ上位結果をシフトして連結する処理は、複数ワードへの拡張例になる。[盤サイズとビットボードの定義](https://github.com/fairy-stockfish/Fairy-Stockfish/blob/2e591089558a5afa72ab5a22192208e71848a30c/src/types.h#L81-L116)、[12×10盤の座標定義](https://github.com/fairy-stockfish/Fairy-Stockfish/blob/2e591089558a5afa72ab5a22192208e71848a30c/src/types.h#L494-L564)

同実装は飛車の表を水平用と垂直用へ分割している。
ソース中では、分割しない大盤用の表は100 MBを超え得ると説明しており、占有ビット数に対する指数的な表の増大を避ける設計である。
乗算を使う経路では128ビットの積から添字を取り出すため、PEXTと同様、64ビットの式を無変更で使っているわけではない。[表の分割](https://github.com/fairy-stockfish/Fairy-Stockfish/blob/2e591089558a5afa72ab5a22192208e71848a30c/src/bitboard.cpp#L62-L72)、[128ビットの添字計算](https://github.com/fairy-stockfish/Fairy-Stockfish/blob/2e591089558a5afa72ab5a22192208e71848a30c/src/bitboard.h#L121-L145)

minaseの[ビットボード](../../src/core/board/bitboard.rs)は、16ビット幅の段を4段ずつ3ワードへ格納する。
この配置から、横線はシフトとマスクで12ビットへ取り出せる一方、縦線と斜線は最大3ワードから占有ビットを集める必要がある。
上記の連結方式を3ワードへ拡張すればPEXTで添字を作れるが、抽出回数と表の参照コストを含めた比較が必要になる。
3ワードを単純な論理和で重ねると別の段が同じ位置へ写るため、その操作だけでは占有情報を保存できない。

## 算術による利き計算

Stockfishの確認時点の実装は、命令集合によって方式を選んでいる。
64ビットARMではビット反転と減算によるHyperbola Quintessenceを使い、AVX2（Advanced Vector Extensions 2）では同方式の複数方向を並列に計算する。
後者は横線だけを6ビットの占有状態から小さな表で求め、それ以外をバイト反転と減算で処理する。
これらに該当しない経路には乗算によるmagicが残るため、このソース自体が単一方式の普遍的な優位性を示すわけではない。[Stockfishの方式選択と計算](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/attacks.h#L30-L165)

やねうら王の確認時点の実装も、9×9盤の飛車と角にmagicの表を使わない。
Qugiyのアルゴリズムとして、占有状態の反転、マスク、減算、排他的論理和を組み合わせて利きを計算する。
これは64升を超える盤にも算術方式を実装できる例だが、81升用の配置と操作を144升へ移した際の速度は別途確かめる必要がある。[やねうら王の飛車と角の利き](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/bitboard.cpp#L852-L922)

## minaseで比較する候補

最初の比較候補は、現在の192ビット配置を維持した線ごとの小表である。
横線にはシフトとマスクを使い、縦線と斜線には占有ビットの抽出を使う構成なら、大きな整数乗算や全盤の配置変更を伴わずに参照表の効果を調べられる。
乗算によるmagicを試す場合も、線ごとのマスクと定数を生成し、全ての対象占有状態で参照実装と照合する。
比較には、利き線表と減算またはビット走査を使う現行方式と、反転と減算による別の算術方式を含める。
この優先順位は上記の実装例とminaseの配置からの推論であり、採用判断には実測が必要である。

PEXTを試す際は、Rustの呼び出し条件も満たす必要がある。
公式の[`_pext_u64`](https://doc.rust-lang.org/core/arch/x86_64/fn._pext_u64.html)は、x86-64の `bmi2` 命令拡張を要求する。
Rust Referenceは、`#[target_feature]` の関数を通常の関数から呼ぶ場合、コンパイラのフラグで同じ命令拡張を有効にしていても `unsafe` が必要と説明している。
したがって、minaseの `unsafe_code = "forbid"` を維持した直接呼び出しを、`target-cpu=native` だけで可能と見なすことはできない。[Rustの呼び出し制約](https://doc.rust-lang.org/reference/attributes/codegen.html#attributes.codegen.target_feature.safety-restrictions)
