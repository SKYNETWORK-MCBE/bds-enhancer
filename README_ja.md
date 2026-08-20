# bds-enhancer

[English](./README.md) / [日本語](./README_ja.md)

BDS でプレイヤーのサーバー間転送などを可能にするための外部ソフトです。

## オリジナルリポジトリ  
[Lapis256/bds-enhancer](https://github.com/Lapis256/bds-enhancer)

原作者様に深く感謝申し上げます。

## 実行方法

1. [Releases](https://github.com/Lapis256/bds-enhancer/releases)より最新の`bds_enhancer.exe`をダウンロードします。
2. `bedrock_server.exe`と同じディレクトリに`bds_enhancer.exe`を配置します。
3. `bds_enhancer.exe`を起動します。

## 使い方

このソフトの機能を使用するには ScriptAPI を使用する必要があります。各自利用できるよう準備してください。

機能を利用するための専用ライブラリとして、`bds_enhancer.js`を用意しています。
[Releases](https://github.com/Lapis256/bds-enhancer/releases)からダウンロードし利用してください。

[ライブラリのドキュメント](./lib/doc_ja.md)

## Scriptエラーのsourcemap解決

ScriptAPIアドオンがエラーを出したとき、bds-enhancerは`scripts/main.js.map`のようなJavaScriptに隣接するsource mapを自動的に読み、解決できたstack frameへ元ソース位置を追加します。

```text
at callback (src/main.ts:8) (main.js:12)
```

デバッグ用に生成JavaScript位置も残します。BDSのScriptAPI stack frameには生成列が含まれないため、生成行内の最初のmappingから元ソース行を解決します。

有効なBehavior Packは次の規則で探索します。

- `system_behavior_packs`内はBDSによって自動適用されるため、すべてのpackを対象にします。
- active world内の`behavior_packs`は、manifestのUUID/versionが`world_behavior_packs.json`に定義されているpackだけを対象にします。
- `development_behavior_packs`にも同じactive worldのUUID/version判定を適用します。
- (BDS直下の`behavior_packs`は内部用なので探索しません)

active worldは`server.properties`の`level-name`から特定します。source mapはエラー発生時に
読み込むため、build watcherによる更新はbds-enhancerを再起動しなくても反映されます。

sourcemap解決はbest-effortです。mapの欠損、壊れたJSON、更新途中、サイズ超過、候補の曖昧さなどでソースを解決できなかった場合、元のエラー文をそのまま表示します。

## 開発

デバッグ実行方法

```
cargo run -- <BDSのディレクトリ>
```

リリースビルドの作成

```
cargo build --release
```
