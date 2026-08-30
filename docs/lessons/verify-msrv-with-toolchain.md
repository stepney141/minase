# 最小対応版は実際のツールチェーンで検査する

## 症状

`Cargo.toml`は必要なRustバージョンを`rust-version`で宣言しておらず、Rust 1.87.0では未安定のlet chainを理由に10件の`E0658`で全ターゲットの検査が失敗した。
let chainは1.88.0で安定化された構文なので最小対応版は1.88と推定できたが、1.88そのもので検査した実測はなく、利用者はどの版からビルドできるかをメタデータから判断できなかった。
crateルートの`unsafe_code`禁止などのlint属性はバイナリや統合テストへ継承されず、方針がリポジトリ全体に及んでいなかった。

## 原因

使用する言語機能から必要版を推定するだけで、宣言も宣言した版での実測もしていなかった。
lintはCargoの`[lints]`ではなくcrateルート属性で指定していたため、ライブラリ以外のターゲットに適用されなかった。

## 以後の規則

`rust-version`を宣言または引き上げるときは、その版のツールチェーンを導入して全ターゲットを検査してから確定し、lint方針は`Cargo.toml`の`[lints]`で全ターゲットへ適用する。

## 出典

- [plans/rust-design-audit-remediation.md](../plans/rust-design-audit-remediation.md)
- [rust-design-audit-2026-08-26.md](../rust-design-audit-2026-08-26.md) の「Rustバージョンとパッケージ契約」「lint方針の適用範囲」
- コミット 8b48929
