# Agent Instructions

This repository is a Rust Minecraft server project.

## Workflow

- Commit after every change as it is made.
- Use branches for substantial, risky, or exploratory changes.
- Run formatting and checks before committing code changes.
- Restart the VibeCraft server after every code or configuration change with `./restart-server.sh`.
- Keep changes small and practical.
- Code should always include comments.
- Decompiled Minecraft sources are stored in `decompiled/` and gitignored.

## Verification

- Use `cargo fmt` for formatting.
- Use `cargo check` for fast compile verification.
- Use `cargo build` when a runnable binary is needed.

## Server

- Default bind address: `0.0.0.0:25565`.
- Use `./restart-server.sh` to stop any existing server process and start a fresh one.
- Server logs are written to `/tmp/vibecraft-server.log` when started by the agent.
- Server PID is written to `/tmp/vibecraft-server.pid` when started by the agent.

## Output Style

- Verbose output is not necessary.
- Do not include output that is only useful for humans; output only information necessary for the AI workflow or explicitly requested for the user.
- All responses must use short, simple, caveman-style wording.
- This applies to every response, including progress updates, explanations, summaries, and final answers.
- Do not switch back to normal assistant tone unless the user explicitly asks.
