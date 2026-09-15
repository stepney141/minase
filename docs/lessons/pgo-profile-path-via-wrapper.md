# PGOのプロファイルはrustcラッパーで対象クレートにだけ適用する

## 症状

`.cargo/config.toml`の`rustflags`へ相対パスの`-C profile-use=pgo/minase.profdata`を置くと、依存クレートのコンパイルが「file does not exist」で失敗した。

## 原因

`rustflags`は依存クレートにも渡され、依存クレートのrustcはレジストリ内のパッケージディレクトリをcwdとして動くので、相対パスが解決できない。絶対パスは`git archive`で展開した別ディレクトリで再現できない。

## 以後の規則

プロファイルや入力ファイルをコンパイラへ渡すときは`build.rustc-wrapper`で`--crate-name`を見て対象クレートにだけ付け、パスはラッパー自身の位置から解決する。

## 出典

[合法手生成と利き計算の高速化](../plans/movegen-speedup.md)の単位D、[PGOの測定記録](../measurements/movegen-speedup-pgo-bench-depth5.md)。
