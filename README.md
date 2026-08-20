# bds-enhancer

[English](./README.md) / [日本語](./README_ja.md)

This is an external software for enabling server transfers between players in BDS.

## Original Repository  
[Lapis256/bds-enhancer](https://github.com/Lapis256/bds-enhancer)

We express our deepest appreciation to the original author. 　

## How to run

1. Download the latest `bds_enhancer.exe` from [Releases](https://github.com/Lapis256/bds-enhancer/releases).
2. Place `bds_enhancer.exe` in the same directory as `bedrock_server.exe`.
3. Launch `bds_enhancer.exe`.

## How to use

To use the features of this software, you need to use the ScriptAPI. Please prepare to use it on your own.

We provide a dedicated library for using the features, `bds_enhancer.js`.
Please download and use it from [Releases](https://github.com/Lapis256/bds-enhancer/releases).

[Library Documentation](./lib/doc.md)

## Script error sourcemaps

When a ScriptAPI add-on emits an error, bds-enhancer automatically uses an adjacent source map such as `scripts/main.js.map` and adds the original source position to each resolvable stack frame.

```text
at callback (src/main.ts:8) (main.js:12)
```

The generated JavaScript position is retained for troubleshooting. BDS does not include generated columns in ScriptAPI stack frames, so the original line is resolved from the first mapping on that generated line.

Active behavior packs are discovered as follows:

- Every pack in `system_behavior_packs` is included because BDS applies these packs automatically.
- Packs under the active world's `behavior_packs` directory are included only when their manifest UUID and version appear in `world_behavior_packs.json`.
- Packs in `development_behavior_packs` are subject to the same active-world UUID and version check.
- The BDS-level `behavior_packs` directory is internal and is not scanned.

The active world is read from `level-name` in `server.properties`. Source maps are read when an error occurs, so updates made by a build watcher are picked up without restarting bds-enhancer.

Source map resolution is best-effort. Missing, malformed, changing, oversized, ambiguous, or otherwise unresolvable maps never suppress the ScriptAPI error. The original stack frame is displayed unchanged and BDS continues running.

## Development

How to run for debugging

```
cargo run -- <Directory of BDS>
```

How to create a release build

```
cargo build --release
```
