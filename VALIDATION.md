# Validation

What has been verified, how, and what still needs a Windows machine or a CI run. Updated with each milestone.

_Status: in progress — filled in as checks complete._

## Verified on the development Mac (no Windows available)

| Check | Command | Result |
|---|---|---|
| Lyric core unit tests | `cargo test -p lunalyr-core` | pending |
| App controller unit tests | `cargo test -p lunalyr --lib` | pending |
| Windows target type-check (whole app incl. platform layer) | `cargo check -p lunalyr --target x86_64-pc-windows-msvc` | pending |
| Renderer unit tests | `npm test` | pending |
| TypeScript typecheck / production build | `npm run typecheck`, `npm run build` | pending |
| Renderer visual check | `demo.html` in a browser | pending |
| Shell smoke run on macOS (stand-in platform) | `cargo run -p lunalyr --features custom-protocol` | pending |

## Needs Windows (CI or a real machine)

| Item | How it will be checked |
|---|---|
| | |
