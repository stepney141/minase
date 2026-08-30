# 不正状態を作れる公開型を避ける

## 症状

`PieceCode::new`は成駒としてしか現れない8種類を未成の駒として構築でき、`PositionBuilder::put`がそれを受理し、SFEN出力が`unreachable!`で停止した。
`SetupPosition`は解析器が1以上9,999以下に制限する手数を公開フィールドで持ち、利用者が0や10,000以上を直接代入できた。
`SearchLimits`は制限が1つもない状態、0の制限、無期限探索と他の制限の併用を構築でき、`SearchSnapshot`は局面と規則の不一致や不合法なルート着手を構築でき、公開の探索入口はこれらを`assert!`で処理していた。
公開されていた`make_move_unchecked`と`unmake_move`は、誤った着手や対応しない`Undo`で盤面、占有集合、ハッシュ値の対応を静かに壊せた。

## 原因

公開型のフィールドが公開され、不変条件が解析器や利用箇所の`assert!`に分散していて、型の構築時には検査されなかった。
その結果、解析器が受理する値の範囲と公開型が表現できる値の範囲が一致せず、不正状態が公開APIだけで作れた。

## 以後の規則

不変条件を持つ公開型はフィールドを非公開にし、全条件を検査して`Result`を返すコンストラクタだけで構築できるようにし、uncheckedな更新と巻き戻しトークンは`pub(crate)`に限定し、0を認めない数値は`NonZero`型で受ける。

## 出典

- [plans/rust-design-audit-remediation.md](../plans/rust-design-audit-remediation.md)
- [rust-design-audit-2026-08-26.md](../rust-design-audit-2026-08-26.md) の「駒コードとSFEN出力の不一致」「未検証着手と合法手の混在」「探索条件と探索局面の不正状態」「SetupPositionの公開フィールド」
- コミット 8b48929
