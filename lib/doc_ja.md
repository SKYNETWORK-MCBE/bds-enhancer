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
- result?: boolean = false - trueの場合,結果を受け取れる
