# 規則集合の代数的データ型化の設計書

## 実施状況

2026年8月25日に本設計書を起案した。
起案に先立ち、同日に利用者とClaudeが設計上の分岐を一つずつ確認して合意し、起案稿に対するcodexの1回目のレビューが、検証責務の層、L0の表示、`Game`の規則保持、エラー優先順、R0の適用範囲、公開APIの差分について指摘を返した。
その指摘を受けて、利用者は各排他群から必ず1つを明示する方針を選び、RULES.md第14版でP0とE0を追加し、第33条を全群明示の形に改めた。
本書はこれらを反映している。
2026年8月25日に実装を完了した。
`cargo test`、`cargo build --all-targets`、`cargo clippy --all-targets`は全て成功し、benchの探索ノード数は着手前後とも645,149だった。
指定した8局のmatch_runner煙試験も完走し、`engine_failures`は全項目0だった。

## 目的

現行の`Rules`は規則コードの`u32`ビット集合であり、コード間の排他（獅子規則のL0とL1、L1とL4、成り規則のP1とP2、反復規則のR1・R2・R3、駒枯れ規則のE2とE3）を`from_codes`の排他ペア表で実行時に検査している。
反復規則の欠落は`Game::new`が`GameBuildError`で別途検査し、`Rules::standard()`は反復規則を持たないために対局を構築できない値として存在している。
本マイルストーンは、排他群から必ず1つを選ぶという構造を代数的データ型で表現し、不正な組合せを構築不能にする。
規則の挙動は変えない。
入力の受理・拒否は、RULES.md第14版第33条第4項に従い、各群の明示を必須とする点だけを変える。

## 決定事項

### 型レベルの意味

「型レベル」は値の型を代数的データ型にすることを指し、規則を型パラメータ（typestateやconst generics）にする方式は採らない。
`--rules`で実行時に規則を切り替える要件があり、全組合せの単相化は測定なしの最適化にあたるためである。

### 規則コードの群

RULES.md第14版第33条第2項に従い、規則コードを次の4つの排他群と修飾規則に分ける。

- 獅子規則: L0またはL1。修飾規則はL2、L3、L4。L4はL0にだけ付く。
- 成り規則: P0、P1またはP2。修飾規則はP3、P4、P5、P6。P5とP1の排他は条文にないため設けない。
- 反復規則: R1、R2またはR3。R0はMinaseが提供しない記録用識別子であり、`RuleCode`にも含めない。
- 駒枯れ規則: E0、E2またはE3。修飾規則はE1。

`RuleCode`にP0とE0を追加し、`ALL`の順序を`L0, L1, L2, L3, L4, P0, P1, P2, P3, P4, P5, P6, R1, R2, R3, E0, E1, E2, E3`と定める。
この順序は群ごとにまとまっており、後述の排他報告順の根拠になる。

### 着手規則と対局規則の二層化

`Rules`を二層に分ける。
着手規則`MoveRules`は手生成と`Position`が参照する獅子規則と成り規則だけを持ち、対局規則`Rules`は`MoveRules`に反復規則と駒枯れ規則、E1を加える。

```rust
pub enum LionRule { L0 { l4: bool }, L1 }
pub enum PromotionRule { P0, P1, P2 }
pub enum RepetitionRule { R1, R2, R3 }   // 既存
pub enum ExhaustionRule { E0, E2, E3 }

pub struct MoveRules {
    pub lion: LionRule,
    pub l2: bool,
    pub l3: bool,
    pub promotion: PromotionRule,
    pub p3: bool,
    pub p4: bool,
    pub p5: bool,
    pub p6: bool,
}

pub struct Rules {
    pub moves: MoveRules,
    pub repetition: RepetitionRule,
    pub e1: bool,
    pub exhaustion: ExhaustionRule,
}
```

列挙子名とフィールド名は規則コードのままにし、既存の`RepetitionRule`とRULES.mdの語彙に揃える。
各フィールドのrustdocに条文番号と一行の意味を書く。
両構造体と列挙体は全フィールドを`pub`とし、`Clone`、`Copy`、`PartialEq`、`Eq`、`Hash`、`Debug`を導出し、crate rootから再公開する。
ビルダーやセッターは設けない。
不正な組合せは型で構築不能なので、構造体リテラルと`..MoveRules::standard()`による更新構文で構築してよい。
`MoveRules::standard()`はL0・P0の値を返す正当な定数であり、`Rules::standard()`は存在しない。
`Rules::ENGINE_DEFAULT`（L0、P0、R1、E0）と`Rules::LISHOGI`（L1、L2、P0、P3、R1、E1、E3）を定数として置く。

各層の利用者は次のとおりである。
`Position::make_move_unchecked`、`MoveGenerator`、`parse_extended_sfen`、`Searcher`と`SearchSnapshot`、perft、手生成のテストは`MoveRules`を受ける。
`Game`、審判層、プロトコル層、`bench`、`match_runner`、`random_play`、`tests/lishogi_replay.rs`は`Rules`を受け、探索とSFENへは`rules.moves`を渡す。
`Position`が読む規則コードはP1・P2・P5だけであり、探索と評価は反復・駒枯れ規則を参照しないことを確認済みである。

`Game`は現在`MoveGenerator`だけを所有し、`Game::rules()`と`AdjudicationContext`はそこから完全な規則を復元している。
`MoveGenerator`が`MoveRules`だけを持つようになるため、`Game`は完全な`Rules`を別フィールドで保持し、`AdjudicationContext::new`へ`Rules`と生成器の両方を渡す。
反復規則が型で保証されるので、`Game::new`と`Game::from_position`は`Result`を返さなくなり、`GameBuildError`は削除する。

### 検証の層

規則文字列の処理は、字句解析と意味検証の二段に分ける。

字句解析`parse_rule_set(&str) -> Result<Vec<RuleCode>, String>`は現行のまま、プリセット名の展開（大文字小文字非区別、単独指定のみ）とコードの綴り検査だけを行う。
プリセットは`Rules`定数からコード列へ戻して展開する。

意味検証`Rules::from_codes(&[RuleCode]) -> Result<Rules, RulesError>`が唯一の意味検証点になる。
`RulesError`は`Duplicate(RuleCode)`、`Conflicting { first, second }`、`Missing(RuleGroup)`の3列挙子とし、`RuleGroup`は`Lion`、`Promotion`、`Repetition`、`Exhaustion`の4列挙子とする。
検査は次の順で行い、現行の報告内容と順序を保つ。

1. 入力順に重複を検査し、最初の重複を`Duplicate`として返す。
2. `RuleCode::ALL`順に各コードを群のスロットへ割り当てる。同じ群のスロットが既に埋まっていれば`Conflicting { first: 既存, second: 新規 }`を返す。L4は群のスロットを埋めない独立のフラグとして扱い、L1のスロットが埋まった状態でL4に達すれば`Conflicting { first: L1, second: L4 }`とする。したがって`[L4, P0, R1, E0]`は次の手順で`Missing(Lion)`になり、`[L1, L4, P0, R1, E0]`は排他違反になる。codexが現行17コードの全部分集合で照合したところ、この走査は現行の排他ペア表が返す最初の違反と全件一致した。P0とE0を上記の位置に置くことで、複数の排他違反があるときにL群、P群、R群、E群の順で報告される優先順も保たれる。
3. 空いている群があれば、L、P、R、Eの順で最初の群を`Missing`として返す。`Display`は`missing lion rule`のように群名を含める。

`Engine::new`と`EngineCommand::SetRules`は現行どおり`Vec<RuleCode>`を受け、受信時に`from_codes`で検証して、失敗時はpendingを維持する。
`protocol/engine.rs`が`Game::new`の失敗で反復欠落を検出していた経路は、`from_codes`の`Missing`に置き換える。
互換の対象は「エンジン境界での最終的な受理・拒否」とし、`from_codes`単体の契約変更（`[L1]`が成功から`Missing`へ変わる）は許容する。

### 表示と記録

`Rules -> Vec<RuleCode>`の変換を一つ用意し、`RuleCode::ALL`順に採用コードをすべて（L0、P0、E0を含む）並べる。
この変換は`Display`とプリセット定数の照合テストに使う。
`Engine`と`random_play`は現行どおり入力由来のコード列を保持し、USIの`state rules`の表示と記録に使う。
`match_runner`の`RuleSetArgument`は、入力原文と解析済みコード列を別々に保持し、原文をエンジン起動と`RuleSet`オプションへ、`from_codes`で作った`Rules`を審判層へ、コード列を測定記録の表示へ使う。
minase-guiは`state rules`の文字列を先後で完全一致比較するだけで型には依存しないため、この表示契約を変えなければ影響はない。

### `contains`の撤去

`Rules::contains(RuleCode)`と`repetition_rule()`、`mate_adjudication_enabled()`などのビット判定アクセサは削除し、利用側はフィールドを直接読む。
互換層として残すと`match`の網羅性検査が効かず、ビット集合時代の書き方が温存されるためである。
利用箇所は約20ファイル・98箇所に及ぶ。

### 入力契約の変更

RULES.md第14版第33条第4項により、規則コード列は各群から1つを明示しなければならない。
`R1`だけの指定は`Missing(Lion)`で拒否され、`engine-default`は`L0,P0,R1,E0`を意味する。
現行の`match_runner`はプリセット名を展開したコード列を旧コミットへ渡すため、P0・E0導入コミットより前のコミットを相手にする測定では、`--rules`にプリセット名ではなく展開後のコード列が渡ると起動に失敗する。
これに対処するため、`match_runner`は前節のとおり`--rules`の入力原文をそのまま両エンジンへ転送し、各コミットが自分の語彙で展開する形に改める。
コードを列挙した指定を旧コミットへ渡せない制限は、docs/sprt.mdに明記して受け入れる。
ハーネスに新旧判定や互換変換は入れない。

### 既存文書の更新

第14版の入力契約と矛盾する記述が、規範として参照される既存文書に残っている。
実装と同じコミットで次を更新する。

- spec-firstマトリクス（`docs/plans/spec-first-tests/matrices/`のd2、d3、d6、d8）とdocs/plans/protocol-layer.mdの受理条件、プリセット展開、`Game::new`での反復欠落検査の記述を、4群の明示、`Missing`、原文転送の契約に改める。
- docs/plans/movegen.mdとd2マトリクスに残る第33条の旧項番参照を第14版の項番へ直す。
- docs/plans/evaluation.mdの将来測定のHaChu規則列にP0を加える。
- docs/plans/random-play.mdの代表入力例に各群の基底コードを補う。
- docs/plans/match-harness.mdの実測済みHaChu測定のコマンドは記録として保持し、第13版当時の入力であり第14版以後はP0を加える旨を注記する。

## 適用範囲外

R0を含む記録用の規則集合の表現、拡張SFENへの規則の埋め込み、およびGUI側の変更は扱わない。
拡張SFENは規則コードを含まないため、再読込時に同じ`MoveRules`を外部から与える現行の契約を維持する。

## 完了基準

棋力に無関係な純粋リファクタリングとして扱い、SPRTは行わない。
完了基準は次の4点である。

- `cargo test`が全通過し、プロトコル層の規則コード受理・拒否テスト（重複、排他違反、群の欠落、プリセットの単独指定）が第33条第4項と第5項のとおりになる。
- `cargo run --release --bin bench`の探索ノード数が着手前と一致する。
- `from_codes`から排他ペア表が消え、`Rules::standard()`、`Option<RepetitionRule>`、`GameBuildError`、`contains`が存在しない。
- `match_runner --rules engine-default`で本変更のコミットと直前コミットの対局が起動し、`engine_failures`が0である。

## 進め方

設計書に対してcodexのレビューを求め、指摘を反映したうえで実装をcodexに委譲する。
実装は`core/rules.rs`の型定義と解析、`Position`・手生成・SFEN・探索の`MoveRules`化、`Game`・審判層・プロトコル層・バイナリの`Rules`化、`match_runner`の原文転送、テストの書換えの順で進め、各段階で`cargo test`を通す。
