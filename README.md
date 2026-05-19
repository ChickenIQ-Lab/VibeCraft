# VibeCraft

A small Rust Minecraft server foundation.

This currently implements the Minecraft server list protocol:

- TCP listener on `0.0.0.0:25565` by default
- Status response with MOTD, version label, player counts, and sample players
- Ping/pong latency support
- Offline-mode login and configuration flow
- Superflat overworld spawn around `0, 65, 0`

The requested version label is set to `26.1.2`. The default protocol number is `775`, matching the `26.1` protocol family.

## Run

```sh
cargo run
```

## Configuration

Environment variables:

- `VIBECRAFT_ADDR`: bind address, default `0.0.0.0:25565`
- `MINECRAFT_VERSION`: displayed version name, default `26.1.2`
- `MINECRAFT_PROTOCOL`: displayed protocol integer, default `0`
- `VIBECRAFT_MOTD`: server list description
- `VIBECRAFT_MAX_PLAYERS`: max players, default `20`

Example:

```sh
MINECRAFT_VERSION=1.21.2 MINECRAFT_PROTOCOL=768 cargo run
```
