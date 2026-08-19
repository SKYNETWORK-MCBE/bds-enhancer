# ScriptAPI エラーの sourcemap 解決 — 仕様・実装計画

最終更新: 2026-08-19

## 目的

BDS が出力する ScriptAPI の JavaScript スタックトレースを、各 Behavior Pack の
source map を使って TypeScript などの元ソース位置へ変換する。

最優先の要件は可用性である。sourcemap 解決機能の不具合、壊れた map、watch 中の
一時的な不整合、未知のログ形式、同名ファイルの曖昧さがあっても BDS と enhancer を
停止させず、元の ScriptAPI エラーを必ず表示する。

## 仕様（暫定合意案）

### 入出力とフォールバック

- enhancer が受け取った BDS のログレコードを対象にする。
- ScriptAPI エラーらしいスタックフレームだけを best-effort で解決する。
- 解決できたフレームは、元のエラー本文を保ったまま元ソース位置が分かる形にする。
- 1 フレームだけ失敗した場合、そのフレームは元の文字列のまま残し、他のフレームの
  解決は続ける。
- ログ全体の解析・map 読み込み・変換・整形のどこかで失敗した場合、そのログ全体を
  入力時の文字列のまま表示する。
- sourcemap 機能の失敗を理由に `panic!`、`unwrap()`、`expect()` しない。
- release profile の `panic = "abort"` は resolver 境界の `catch_unwind` を無効化するため
  撤廃し、予期しない panic でも元ログへ戻れる構成にする。
- map が存在しない通常の JavaScript パックもエラー扱いせず、元ログを表示する。
- sourcemap 用の診断ログを出す場合も、元の ScriptAPI エラーを先に失わず表示する。

### 複数パック

- active world は `server.properties` の `level-name` から特定する。
- `system_behavior_packs` 内のpackはBDSによって自動適用されるため、全packを探索する。
- active world の `world_behavior_packs.json` にある `pack_id` と `version` を、world pack
  およびdevelopment packが使用中かどうかのauthorityとする。
- active world 内の `behavior_packs` は、上記JSONにUUID/versionが一致するpackだけ探索する。
- BDS直下の `development_behavior_packs` も、上記JSONにUUID/versionが一致するpackだけ
  探索する。
- BDS直下の `behavior_packs` は内部用なので探索しない。
- active worldや `world_behavior_packs.json` を安全に読めない場合、world/development
  packを推測で採用せず、system packだけを候補にする。
- 各 pack の `manifest.json` と script module の `entry` を読み、生成 JavaScript と
  隣接する `.map` を候補として索引化する。
- BDSのスタックにpackを一意に特定できる情報があれば、それを最優先する。
- BDS 1.26.44.3のエラー先頭にある角括弧は `manifest.json` の `header.name` そのもの
  なので、文字列をversion部分へ分割せず、header nameと完全一致させて候補を絞る。
- 同じheader nameを持つactive packが複数あり、生成ファイルからも一意にできない場合は
  versionをエラーから判定できないため未変換にする。
- 同じ生成ファイル名を持つ複数候補を一意に決められない場合、誤変換を避けてその
  フレームを未変換のまま表示する。
- watch による map の更新をプロセス再起動なしで反映する。読み込み中・書き換え中の
  壊れた snapshot は採用しない。

### 表示形式

元のスタック行に、解決できた元位置を生成位置の直前へ挿入する。生成位置も調査用に
残す。

例（形式は未確定）:

```text
at callback (src/main.ts:8) (main.js:12)
```

BDS 1.26.44.3 の観測ログには生成列がない。そのため初期版は generated line 上で最初に
対応する segment を使って元の行を解決し、列は表示しない。行内に複数の元位置がある
場合は厳密な特定ができないため、行だけの best-effort 結果であることを仕様上の制約と
する。

### 性能と安全性

- 通常ログではファイル I/O をしない。
- map は必要時に遅延ロードし、更新時刻・サイズなどで再利用可否を判断する。
- source map の path は pack ディレクトリ外へ無制限に展開しない。
- `sourcesContent` があれば位置解決に利用できるが、初期版ではソース本文の表示は必須に
  しない。
- enhancer 自身の異常と BDS 子プロセスの終了は区別する。今回の機能が子プロセスの
  生存期間を変更しない。

## 受け入れ条件

- [x] 単一packの例外で生成位置から `src/*.ts` の行へ解決できる。
- [x] 2個以上のpackがあり、それぞれの例外を正しいmapへ解決できる。
- [x] mapなし、壊れたJSON、対応segmentなし、範囲外の行・列で元ログが残る。
- [x] watchがmapを書き換えてもenhancer/BDSが落ちず、元ログまたは更新後位置が表示される。
- [x] 同名`scripts/main.js`が曖昧なら、誤ったTypeScript位置を表示しない。
- [x] sourcemapと無関係な既存ログ・action・標準入力処理の挙動を変えない。
- [x] resolverの単体テストと、実BDSを使った手動確認の両方を記録する。

## 段階計画と進捗

### Phase 0: 実ログの観測と仕様確定 — 進行中

- [x] enhancer の現在の stdout 集約・表示経路を確認。
- [x] テスト pack の `main.js.map`、`manifest.json`、watch 構成を確認。
- [x] テスト pack の nested callback から意図的に例外を発生させ、生ログを採取。
- [ ] top-level / async（可能なら）のスタック形式を追加比較。
- [x] スタック内の `[header.name]` でpack候補を識別できることを確認。
- [x] 表示形式を「生成位置を残して元位置を追記」に決定。
- [x] world 固有 pack を含む探索対象とactive判定を確定。

### Phase 1: 純粋な resolver と失敗系テスト — 完了

- [x] ScriptAPI スタックフレーム parser を、非一致なら入力を返す純粋処理として実装。
- [x] source map VLQ 解決ライブラリに Sentry の Rust `sourcemap` crate 9.3.2 を選定。
- [x] pack/map index と一意候補選択を実装。
- [x] 正常系・部分失敗・全体失敗・曖昧候補の単体テストを追加。

### Phase 2: stdout パイプライン統合 — 完了

- [x] resolver を既存ログ表示直前へ統合。
- [x] 元ログを所有したまま変換し、失敗時に必ず元ログへ戻す境界を設ける。
- [x] 既存 action 検出、コマンド応答、ログ色分けへの回帰テストを追加。

### Phase 3: watch・複数 pack・実 BDS 検証 — 完了

- [x] map更新前後で位置が切り替わることを確認。
- [x] 2 pack fixtureで自動選択または安全な曖昧フォールバックを確認。
- [x] mapを一時的に欠損させてもBDS/enhancerが継続し、元エラーが見えることを確認。
- [x] 実行コマンド、観測ログ、未確認事項をこの文書へ追記。

### Phase 4: ドキュメントと仕上げ — 完了

- [x] README / README_ja に機能、探索規則、fallback を追記。
- [x] `cargo fmt`、`cargo test`、`cargo clippy` を実行。
- [x] release build で最終確認。

## 観測メモ

- テスト pack は `tsdown --watch`、`sourcemap: true` で生成される。
- 現在の map は `scripts/main.js.map` で、`file` は `main.js`、`sources` は
  `../src/main.ts`、`sourcesContent` を含む。
- 現在の `LogDelimiterStream` は BDS のログ prefix または 50 ms timeout でログを
  まとめる。複数行スタックが分割される可能性を Phase 0 で必ず観測する。
- BDS 1.26.44.3 で nested callback の例外は次の 1 レコードとして観測できた。

  ```text
  [2026-08-19 15:50:34:825 ERROR] [Scripting] [scriptapi-template v0.1.0] Error: BDS_ENHANCER_SOURCEMAP_PROBE    at throwNestedSourcemapProbe (main.js:4)
      at <anonymous> (main.js:8)
  ```

- フレームは生成ファイルのbasenameと1-based lineのみで、列・pack path・UUIDは
  含まれない。エラー先頭の角括弧はmanifestのheader nameであり、versionが見える場合も
  header name自体に含まれている文字列である。
- 上記の複数行スタックは現在の `LogDelimiterStream` でも同じ ERROR 色で出力され、
  少なくとも今回の callback 例外では 50 ms timeout による途中分割は起きなかった。
- 実ログ採取では既存 BDS と競合しない一時ポート・一時 world・offline mode を使用し、
  採取後に `server.properties` を元の値へ戻した。

## Phase 1 実装メモ

- `SourcemapResolver::discover` はsystem packを自動採用する。active worldのworld packと
  development packは `world_behavior_packs.json` のUUID/version参照に一致するものだけ
  採用し、BDS直下の内部用 `behavior_packs` は探索しない。
- 採用したpackの `manifest.json` を読み、script moduleのentryと隣接`.map`を索引化する。
  壊れたmanifest、server properties、world参照JSON、読めないrootは非致命的に無視する。
- エラーのheader nameと、stack frameの生成ファイル名が一致する候補だけを
  解決する。複数候補のうち実際に解決できるものが1個だけなら採用し、0個または2個
  以上なら元frameを残す。
- map は最大64 MiBとし、読み込み前後のsize/mtimeが変わった場合はwatch更新中とみなして
  採用しない。
- BDSが列を出さない場合、generated line上の最初のmapped tokenを利用する。列0を仮定
  するとtsdown mapでは前行tokenを返すことが実fixtureで判明したためである。
- mapのsource pathをpack相対で正規化し、pack外へ抜けるpathは表示しない。
- Rust 1.85互換のため、Sentry `sourcemap`が許容する`url`を2.5.2へ固定した。新しい
  `url`が引くICU依存はRust 1.88を要求する。
- resolver全体を`catch_unwind`境界で囲った。releaseでも有効にするため既存の
  `panic = "abort"`を撤廃した。
- 2026-08-19: 実際のtsdown watch mapをfixture化したテストを含む15テストを追加。
  `cargo test`成功。既存Clippy 4件を個別allowした検査では新規警告なし。

## Phase 2 実装・実機確認メモ

- resolverはaction判定とコマンド結果転送の後、stdoutへ色付き表示する直前だけに適用した。
  join/spawn検出にも変換前の元ログを渡す。
- action、ERROR level、`NO LOG FILE! - `除去、コマンド結果payload、join/spawn event、
  無関係な通常ログについて回帰テストを追加した。
- 実BDS 1.26.44.3で次の表示を確認した。生成位置とERROR色を残し、元位置が追記された。

  ```text
  at throwNestedSourcemapProbe (src/main.ts:4) (main.js:4)
  at <anonymous> (src/main.ts:9) (main.js:8)
  ```

- 最初の実機試行では角括弧をname/versionに誤分割して未解決になったが、元エラーはそのまま
  表示されBDSも継続した。角括弧がmanifestのheader nameそのものだと修正後、解決に成功。
- sandboxではonline modeのMinecraft services接続がtimeoutしたため、実機確認中だけ
  `online-mode=false` / `allow-list=false`を使用し、確認後に元のtrueへ戻した。
- 2026-08-19: Phase 2完了時点で `cargo test` 21件成功。

## Phase 3 watch・復旧確認メモ

- 同じresolverインスタンスでmapのsource pathを書き換え、index再生成なしで新しい位置へ
  解決できるテストを追加した。
- mapを壊した状態では元ログへ戻り、正常mapへ戻した後は同じresolverで再解決できる
  テストを追加した。
- 2 pack fixtureではheader nameと生成ファイルから各mapを選択でき、同一候補が複数なら
  未変換になることを再確認した。
- 実BDSを起動したままtest addonへ空行を追加し、watchがmapを更新した後に`reload`した。
  生成位置は同じまま、元位置が次のように切り替わった。

  ```text
  before: at throwNestedSourcemapProbe (src/main.ts:4) (main.js:4)
  after:  at throwNestedSourcemapProbe (src/main.ts:5) (main.js:4)
  ```

- 稼働中に`main.js.map`を一時退避して`reload`すると、元の`(main.js:4)`フレームがそのまま
  表示され、BDSは継続した。map復元後の次の`reload`でTypeScript位置表示へ復帰した。
- 検証には既存debug版と競合しないrelease版、19142/19143、一時worldを使用した。
  検証用BDSだけを停止し、test addonの空行、map、server.propertiesは元へ戻した。
- 2026-08-19: Phase 3完了時点で `cargo test` 23件成功。

## 最終決定

- resolver固有の診断ログは通常表示しない。解決失敗時の利用者向け結果は元エラーである。
- BDSが生成列を出さない場合は生成行内の最初のmappingを使う制約をREADMEへ明記する。

## Phase 4 最終確認

- README / README_jaへ表示例、active pack探索規則、watch反映、fallback保証を追記した。
- 既存Clippy警告4件を整理し、BDS stdout終了後に子プロセスを`wait()`するようにした。
- `cargo fmt -- --check`: 成功。
- `cargo test`: 24件成功。
- `cargo clippy --all-targets -- -D warnings`: 成功（allowなし）。
- `cargo build --release`: 成功。
- 仕上げ後の表示調整として、解決済みframeの生成JS位置だけANSI dimを適用した。
  `\x1b[22m`でintensityのみ戻すため、ERRORの赤色は維持する。未解決frameは装飾しない。
