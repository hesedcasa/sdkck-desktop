# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`sdkck-desktop` is a Tauri v2 desktop app for the [sdkck](https://github.com/hesedcasa/sdkck) CLI. It keeps sdkck's plugin model: plugins stay oclif packages that need Node to run. Everything around running them (the command catalog, search, permission rules, plugin listing, runtime discovery) is done natively in Rust. Commands run either cold (`node <sdkck>/bin/run.js …`) or in a pre-booted warm Node worker.

## Common Commands

- **Dev app:** `npm run app:dev` (Vite on :1420 plus the Tauri window)
- **Release build:** `npm run app:build` (installers under `target/release/bundle`); `npx tauri build --debug --no-bundle` builds a debug binary with the frontend embedded
- **UI only:** `npm run build` (typecheck + Vite build to `dist/`); `npm test` (node:test over `ui/src/lib/*.test.ts`)
- **Rust:** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all` (`rustfmt.toml`: width 130)
- **Warm-runner tests:** `SDKCK_TEST_ROOT=<prefix>/node_modules/sdkck cargo test -p sdkck-core --test warm` (skipped when unset; CI installs sdkck from npm for them)
- **Benchmark / parity:** `cargo run --release -p sdkck-core --example bench -- <argv>`; `cargo run -p sdkck-core --example dump -- <sdkck root> [--verbose]` prints catalog ids (diff against `sdkck commands --json`)

`tauri::generate_context!` embeds `dist/`, so `npm run build` must have run before `cargo build -p sdkck-desktop` or clippy on the workspace.

## Architecture

- `crates/sdkck-core` — all logic, no Tauri dependency:
  - `paths.rs`: oclif's `Config` dir resolution (`SDKCK_<CATEGORY>_DIR` > macOS cache > `XDG_*_HOME`/`LOCALAPPDATA` > home defaults, joined with `sdkck`).
  - `catalog.rs`: builds the command list from the root manifest (which also holds JIT placeholder commands), bundled plugins (`oclif.plugins`, globs resolved Node-style up the ancestors' `node_modules`), and user/link plugins from `<dataDir>/package.json`. Precedence: core < user < link < jit; dynamic commands only fill ids nobody else has. JIT entries are dropped once their plugin is installed. A `Fingerprint` (mtime+size of every file read, plus the config dir) revalidates the cache cheaply.
  - `dynamic.rs`: mirrors `@hesed/api2cli`'s `createOperationCommand` and `@hesed/mcp-client`'s `createMcpToolCommand` arg/flag shapes. Keep these in sync when those plugins change (flag order, `body-` prefix, output-flag shadowing, `(required)` prefix). MCP tools are listed only when `<dataDir>/node_modules/@hesed/mcp-client/dist/index.js` exists, which is the same condition under which `bin/run.js` registers them.
  - `permission.rs`: port of `@hesed/permission`'s `permission-config.js` (matching, deny-wins, default-deny, legacy migration, v1 file format). The plugin's prerun hook stays the enforcement point; the app only filters and edits.
  - `agent_vault.rs`: mirrors sdkck's `shouldIntercept`. Any non-`Off` result forces cold runs. Misconfigured also counts, so the CLI's fail-closed error is surfaced.
  - `runner.rs`: cold spawn and the warm `Worker` (JSON-lines protocol; worker replies are prefixed `\x1eSDKCKD `). Sinks are never called while the worker lock is held, because they call back into the engine.
  - `engine.rs`: the facade. Settings/history JSON in the app data dir, a managed sdkck install under `<appData>/runtime`, and decisions about when warm is allowed (`COLD_ONLY` prefixes, JIT commands, interception). After every run, a stale catalog fingerprint recycles the worker.
- `crates/sdkck-core/worker/worker.mjs`: embedded with `include_str!` and written to `<appData>/worker/` at spawn. It replicates `bin/run.js`'s `Config.load` patch (API/MCP dynamic commands, `persistDynamicCommands`); **keep it in sync with sdkck's `bin/run.js`**. It also snapshots and restores globals that the init hooks wrap per run, gives commands an empty stdin, turns `process.exit` into a thrown exit, and exits on uncaught errors (the app then restarts it). At boot it runs a silent `--version` to load the init hooks, which is not counted by telemetry.
- `src-tauri/src/lib.rs`: thin async wrappers (`spawn_blocking`) over `Engine`. The engine is built on a background thread (importing the login-shell env can take a moment), and commands wait on an `EngineCell`. Run output is emitted as `run-event`; the sdkck install log is emitted as `setup-log`.
- `ui/`: React 19 + Vite. `lib/types.ts` mirrors the serde types (camelCase). `lib/runs.tsx` keeps every run's streamed state, because events can arrive before `invoke` resolves with the run id. `lib/argv.ts` is pure (Node-testable).

## Gotchas

- **Desktop apps lack the terminal's environment.** On macOS/Linux, `loginShellEnv` (default on) merges `$SHELL -lic env` output, with a 5 s timeout. Process env wins, except `PATH`, which is taken from the shell. On Windows, strip every `Path` spelling before setting `PATH` (env keys are case-insensitive there).
- **Never run plugin installs or JIT first-runs warm.** The worker's loaded config would keep ghost entries. This is the same constraint described in the webui repo's CLAUDE.md.
- Permission rules added from the app seed `{"pattern":"*"}` when no file exists, matching the CLI's `permission disallow` on a fresh config. Otherwise one deny rule would flip everything to default-deny.
- Output is ANSI-stripped in Rust. Children get `FORCE_COLOR=0`/`NO_COLOR=1`.

## Conventions

- Node ≥ 22 for sdkck itself; `.nvmrc` pins 24 for the UI toolchain. npm only.
- PR titles follow Conventional Commits (CI-enforced).
