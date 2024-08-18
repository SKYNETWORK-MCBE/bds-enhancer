## sendTransferAction

Transfers a player to any server.

### Arguments

- player: Player - The player
- host: string - The host address of the destination server
- port: number - The port number of the destination server

## sendKickAction (WIP)

Kicks a player out.

> [!NOTE]
> This feature is under development.
> It is planned to be able to kick by the player's id in the future.

### Arguments

- playerName: string - Player name
- reason: string - Reason for expulsion

## sendStopAction

Stops the server.

## sendReloadAction

Reloads mcfunction and scripts.

## executeCommand

Executes any command with console privileges.

### Arguments

- command: string - The command to execute
- result?: boolean = false - If true, the result can be received

## executeShellCommand

Executes any shell command.

### Arguments

- mainCommand: string - Specify an executable file
  - ex) Type `"git"` when you run `git pull`
- args?: string[] - Specify command arguments
  - ex) Type `["pull"]` when you run `git pull`
- result?: boolean - If true, the result can be received
