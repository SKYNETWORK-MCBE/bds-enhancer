## sendTransferAction

プレイヤーを任意のサーバーへ転送します。

### 引数

- player: Player - プレイヤー
- host: string - 転送先サーバーのホストアドレス
- port: number - 転送先サーバーのポート番号

## sendKickAction (WIP)

プレイヤーを追放します。

> [!NOTE]
> この機能は開発中です。
> 今後、プレイヤーの ID でキックできるようになる予定です。

### 引数

- playerName: string - プレイヤー名
- reason: string - 追放理由

## sendStopAction

サーバーを停止します

## sendReloadAction

mcfunction とスクリプトを再読み込みします。

## executeCommand

コンソール権限で任意のコマンドを実行します。

### 引数

- command: string - 実行するコマンド
- result?: boolean - `true`の場合, 結果を受け取れる

## executeShellCommand

任意のシェルコマンドを実行します。

### Arguments

- mainCommand: string - 実行ファイル
  - ex) `git pull`を実行する場合は`"git"`を入力する
- args?: string[] - コマンド引数
  - ex) `git pull`を実行する場合は`["pull"]`を入力する
- result?: boolean - `true`の場合, 結果を受け取れる
