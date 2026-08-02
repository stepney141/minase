# 指し手正準化設計書　第1版

## 実施状況

フェーズAとフェーズBは2026年7月31日に完了した。テストは122件全緑、clippy警告ゼロである。差分オラクル（`src/movegen/oracle.rs`、コミットc316a44の旧生成器を移植）は、条文配置12局面とシード固定乱数512局面の計524局面で新生成器との集合一致を確認した。オラクルは照合完了後に削除した（第1コミット 5907a48 のツリーに保存されている）。

## 背景と目的

現行の指し手生成は、獅子・角鷹・飛鷲の2段階移動を経路単位で列挙する。空の升を経由するDoubleは、同じ結果局面に到達するStep・Jump・じっとと重複し、裸の獅子1枚で88手（Step 8、Jump 16、Double 64）を生成する。本設計書は、この重複を生成の源で断ち、正準形25手（Step 8、Jump 16、じっと1）へ移行するための仕様を定める。

正準化の基準は「第1段階で捕獲しないDoubleはすべて冗長」である。到達升で駒を取る手であっても、第1段階が非捕獲ならJumpと結果局面も合法性も一致する。`rules.rs` の `is_tsukegui` はmidでの実捕獲を要求するため、空mid経由で獅子を取る手はJump捕獲と同一の足判定に落ちることを確認済みである。

## 決定事項

1. 空midのDoubleは生成しない。Doubleが存在するのは第1段階で敵駒を捕獲する場合だけである。
2. Move型はenumを廃し、単一structに全統合する。StepとJumpの区別（麒麟・鳳凰の跳びを含む）は型から消える。
3. shogiops基準のperft照合、および自己perft回帰値を含む、perft数値を正しさの基準とするテストはすべて撤去する。
4. 安全網は3本とする。移行期限定の差分オラクル、不変条件のプロパティテスト、計測用に存続するperft CLIである。
5. 将来のUSI/lishogi互換記法は、冗長経路表記も受理してパース時に正準形へ潰す（本フェーズでは未実装）。
6. 型再設計と生成の重複除去は不可分なので一括で行う。midのOption化自体が、Jumpと空midのDoubleを同じ値に潰すためである。

## 新しいMove型

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Move {
    pub from: Square,
    pub mid: Option<Square>,
    pub to: Square,
    pub promote: bool,
}
```

意味は次のとおり定める。通常の移動・走り・跳びは `mid: None` で `to != from`。経路捕獲つき2段階移動は `mid: Some(m)` で、mは必ず敵駒の升。居喰いは `mid: Some(m)` かつ `to == from`。じっとは `mid: None` かつ `to == from`。

不変条件は「midがSomeならば、その升で必ず捕獲する」の1本である。`Move { from, mid: None, to: from }` は獅子類のじっと以外では意味を持たないが、Game層は生成済み合法手との照合で着手を検証するため、型レベルでの排除は行わない。

`capture_candidates` は `[mid, if to != from { Some(to) } else { None }]` となる。これまでStep/Jumpは捕獲候補をスロット0に置いていたが、統合後は到達升の捕獲がスロット1に移る。スロット順序に依存する消費側（undo復元、`captured_lions`、`is_tsukegui` のfirst/second判定）をすべて監査すること。

## 生成規則

獅子は次の4種を生成する。

1. Step: `king_steps(from) & !own` の各升へ `{from, None, to}`（通常手生成側、従来どおり）。
2. Jump: `lion_jumps(from) & !own` の各升へ `{from, None, to}`。
3. 経路捕獲Double: 敵駒のいる隣接升midごとに、局所オキュパンシ更新後の `king_steps(mid) & !own` の各升へ `{from, Some(mid), to}`。`to == from`（居喰い）を含む。
4. じっと: 空（駒がない）の隣接升が1つでもあれば `{from, None, from}` を1手だけ。

角鷹・飛鷲は方向限定で同じ原理を適用する。方向ごとに、第1升が敵駒なら居喰い `{from, Some(first), from}` と、第2升が自駒でなければ `{from, Some(first), second}` を生成する。第2升が自駒でなければ跳び `{from, None, second}` を第1升の占有と無関係に生成する。じっとは、いずれかの方向の第1升が空である場合に `{from, None, from}` を全体で1手だけ生成する（第11条7項・8項。敵駒がある方向は捕獲になるため、じっとの根拠にならない）。

## 影響箇所

`src/mv.rs` は型定義とUndo、capture_candidates。`src/movegen/normal.rs` は成り分岐ヘルパと全生成経路。`src/movegen/lion.rs` は上記生成規則への書き換え。`src/rules.rs` は `is_tsukegui`・`lion_capture_is_legal`・`VirtualBoard::after_move` のパターン更新。`src/position.rs` はapply/undo/captured_squares。`src/game.rs` はR1攻撃的着手判定とlion_taken処理。`src/sfen.rs` は局面のみを扱うため無変更。`src/bin/perft.rs` と `MoveGenerator::perft` は計測用に存続する。

テストの機械的移行は、`Step{f,t,p}` と `Jump{f,t,p}` を `Move{f, None, t, p}` へ、`Double{f,m,t,p}` を `Move{f, Some(m), t, p}` へ写す。ただし空midのDoubleを前提とする箇所（じっと列挙ヘルパ、経路重複を数える断定）は、条文の意味を保ったまま正準形へ書き直す。じっとの断定は「空き隣接升があるとき正確に1手」となる。

## 検証戦略

perft数値の断定は撤去する。対象は `verification.rs` のshogiops照合（裸獅子88手、初期局面深さ2=1296等）と自己回帰値（初期深さ3=52,599、深さ4=2,134,748、複合獅子深さ2=6,380/8,632）のすべてである。SFEN変換テストは残す。

代わりの安全網を次のフェーズで導入する。第1に、コミット `c316a44` 時点の旧生成器を `#[cfg(test)]` のオラクルモジュールへ移植し、旧出力を正準化写像（Step/Jump→mid None、空midDouble→to==fromならじっと・それ以外はmid None、敵midDouble→mid Some）で潰した集合と、新生成器の出力集合の一致を、条文テストの局面群とランダム局面で照合する。オラクルは移行完了後に削除する。第2に、ランダム局面での不変条件（生成手に重複なし、midがSomeなら敵駒、到達升は自駒でない、apply→undoで局面とZobristが完全復元）をプロパティテストとして恒久的に維持する。

## 実施フェーズ

フェーズAは、型再設計・生成書き換え・全消費箇所の追随・既存テストの移行・perft数値断定の撤去を一括で行い、cargo test全緑・clippy無警告で終える。フェーズBは、差分オラクルとプロパティテストを追加する。フェーズCは、オラクルでの照合に問題がなければ文書を更新し、最終的にオラクルを削除する。

## やらないこと

指し手記法（USI等）の実装、探索部、Move型のスマートコンストラクタによる型レベル不変条件の強制、および対局サービス向けの入力検証は本計画の対象外である。
