# VibeCraft

A small Rust Minecraft server foundation for the `26.1` protocol family.

## Current Features

- TCP listener on `0.0.0.0:25565` by default
- Server-list status response with MOTD, version label, protocol, max player count, and ping/pong latency support
- Offline-mode login with a minimal vanilla configuration flow, including required registries, tags, and enabled features
- Optional Mojang profile property lookup so clients can show player skins when available
- Play-state join into a superflat `minecraft:overworld` with view distance `6`
- Generated terrain with bedrock, dirt, grass, and a small spawn tree at chunk `0, 0`
- Dynamic chunk cache updates and fresh chunk streaming when players cross chunk borders
- Multi-player visibility through player info, entity spawn/despawn, metadata, movement, and head-rotation packets
- Keep-alive heartbeat while players are connected
- Creative inventory tracking for selected hotbar slot, slot edits, offhand swaps, and reconnect replay
- Binary player persistence for position, rotation, on-ground state, selected slot, and inventory under `data/players/`
- Binary block edit persistence under `data/world/blocks.bin`, with break/place actions, block update broadcasts, same-slab double-slab merging, tall blocks, beds, and basic toggleable block states
- Generated block placement table in `src/block_items.csv` with `1026` item-to-block mappings

The default version label is `26.1.2`. The default protocol number is `775`, matching the `26.1` protocol family.

World block edits save after each edit. Player data saves on disconnect, on clean shutdown, and every 30 seconds while players are online.

## Run

```sh
cargo run
```

## Configuration

Environment variables:

- `VIBECRAFT_ADDR`: bind address, default `0.0.0.0:25565`
- `MINECRAFT_VERSION`: displayed version name, default `26.1.2`
- `MINECRAFT_PROTOCOL`: displayed protocol integer, default `775`
- `VIBECRAFT_MOTD`: server list description
- `VIBECRAFT_MAX_PLAYERS`: max players, default `20`

Example:

```sh
MINECRAFT_VERSION=1.21.2 MINECRAFT_PROTOCOL=768 cargo run
```
