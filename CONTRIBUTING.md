# Contributing

Issues and pull requests are welcome.

Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md). Report security problems privately, as
[SECURITY.md](SECURITY.md) describes, not in a public issue.

Before a pull request, run `bun run check` and the headless checks below, then open one of your builds
in the app and compare the sidebar against Path of Building itself.

Keep the numbers PoB's own. Do not reimplement calculations in Rust or TypeScript.

Add a code comment only where the code cannot show why, such as a PoB quirk, a workaround or an invariant
that is easy to break. Keep it to one short line.

## Build

You need:

- Rust stable. On Windows, the MSVC toolchain. The first build compiles LuaJIT.
- CMake and libclang. The first build of the app also compiles whisper.cpp for voice input. On
  Windows, the "C++ Clang tools" component of Visual Studio provides libclang.
- Bun 1.4 or newer. This is what CI uses.
- Tauri's platform prerequisites. Windows: WebView2. Linux: `libwebkit2gtk-4.1-dev`,
  `libappindicator3-dev`, `librsvg2-dev`, `patchelf`, `libssl-dev`, `libasound2-dev`,
  `libclang-dev`. macOS: the Command Line Tools, with the licence accepted
  (`sudo xcodebuild -license accept`); the system WKWebView needs nothing installed. See
  [MACOS.md](MACOS.md).
- Checkouts of [PathOfBuilding-PoE2](https://github.com/PathOfBuildingCommunity/PathOfBuilding-PoE2)
  and [PathOfBuilding](https://github.com/PathOfBuildingCommunity/PathOfBuilding) next to this repo.
  To sync one game from somewhere else, run `bun run sync:poe2` or `bun run sync:poe1` with
  `POB_SOURCE` set to that checkout.

```sh
bun install
bun run sync          # copy both games' PoB Lua and data into src-tauri/resources/pob and pob1. Run this first.
bun run sync:fast     # same, but keep the decoded tree art from the last sync
bun run tauri dev     # run the app with hot reload
bun run tauri build   # installers land in target/release/bundle/
bun run check         # type-check the frontend
```

CI type-checks and compiles on every push. Installers are built only when a `v*` tag is pushed; see
`.github/workflows/release.yml`.

If `NoDefaultCurrentDirectoryInExePath` is set in your shell, unset it before the first build. LuaJIT's
`msvcbuild.bat` needs cmd.exe to find `minilua` in the current directory.

## Layout

```
pob-redux/
  crates/pob-engine/   headless PoB: LuaJIT host shim, JSON bridge, worker engine pool, `pobctl` CLI
  crates/pob-sync/     copies a PathOfBuilding-PoE2 checkout into src-tauri/resources/pob
  src-tauri/           Tauri app: engine thread, commands, `pob://` asset protocol, MCP server
  src/                 Svelte frontend
  pob-sync.toml        upstream checkout path and pinned commit
```

## Translate the interface

The interface is translated with [Paraglide JS](https://paraglidejs.com). English lives in
`messages/en.json`; each other language is a file beside it, named for its locale. A key a language
does not have compiles to the English text, so a partial translation is safe to merge.

```sh
bun run i18n          # compile messages into src/lib/paraglide (bun run check and the dev server do this too)
```

To translate, copy a key and its English text into your language's file and replace the text. Keep
every `{placeholder}` exactly as it appears in English; the compiler rejects a message that invents
one. Leave proper nouns alone: Path of Building, PoB Redux, Discord, Wraeclast and key names such as
`Ctrl+B`. Where a string names something from the game, prefer the wording the game itself uses in
that language.

In the app, `m.some_key()` returns the text for the current language and re-renders when it changes.
Build a table of labels inside `$derived`, never at module scope, or it keeps the language it was
first built with.

To add a language, put its tag in `locales` in `project.inlang/settings.json`, add
`messages/<tag>.json`, and add the language's own name to `LOCALE_LABEL` in
`src/lib/state/locale.svelte.ts`.

Game data — passive names, item mods, gem names, the stats in the sidebar — comes from Path of
Building and stays in English whatever the interface language is.

## Update the bundled PoB data

The installer bundles PoB's Lua and game data. The app never downloads data while it runs, and PoB's
built-in updater is stubbed out. To move to a newer PoB:

1. Pull the PathOfBuilding-PoE2 checkout.
2. Put the new commit hash in `pob-sync.toml`. `bun run sync` refuses to run if the checkout's HEAD
   differs from that pin. Pass `--allow-commit-mismatch` to override.
3. Run `bun run sync`. Use the full sync when the tree changed, because tree art is decoded only with
   `--tree-assets`.
4. Run the checks below, then open one of your builds and compare the sidebar against Path of
   Building. `lua/bridge.lua` drives PoB's internals, so an upstream refactor can break it.
5. Build and release. CI reads the pin from `pob-sync.toml` and checks out that exact commit, so the
   repo alone describes a release.

Users get new data with a new app release. The status bar shows the bundled PoB version and commit.
Do not edit files under `src-tauri/resources/pob/`, because the next sync overwrites them.

## Run checks without the UI

`pobctl` drives the engine from the command line.

```sh
bun run ctl -- --pob-root src-tauri/resources/pob methods            # list bridge methods
bun run ctl -- --pob-root src-tauri/resources/pob call get_build     # call one method
bun run ctl -- --pob-root src-tauri/resources/pob stats build.xml    # load a build, print its sidebar
bun run ctl -- --pob-root src-tauri/resources/pob bench              # time full recalculations

# parallel scoring against the sequential path; mismatches must be 0
bun run ctl -- --pob-root src-tauri/resources/pob power build.xml --stat Life --pool 8
bun run ctl -- --pob-root src-tauri/resources/pob gems build.xml --group 1 --pool 8
```

With the app running and its MCP server on, `bun run mcp` calls the same tools an AI client would, and
`bun run check:tools` guards the assistant's tool lists against the live registry.

## Where builds are stored

PoB Redux reads and writes the same folder as Path of Building:
`Documents/Path of Building (PoE2)/Builds`. It reads PoB's `Settings.xml` to find a custom build path,
and never writes that file.

| Variable | Effect |
|---|---|
| `POB_REDUX_USER_DIR` | Parent folder for `Path of Building (PoE2)/`. Default: Documents |
| `POB_REDUX_POB_ROOT` | Use this PoB data folder instead of the bundled one |
| `POB_REDUX_OPEN` | Build XML to open at launch. The first argument does the same |
| `POB_REDUX_POOL` | Worker engines for node power and gem scoring. They boot on the first scan and are released two minutes after the last. Default: half the cores, at most 8 |
| `POB_REDUX_MCP` | Start the MCP server on this port at launch, whatever the saved setting says |

## Assistant memory

Assistant settings contain an opt-in **Assistant memory** switch, editors for the global and current
saved build documents, and a review queue. The switch defaults to off. The app stores approved
Markdown, revisions, the setting and proposal history together in `assistant-memory/store.json`
under Tauri's local app data directory (`dev.pobredux.desktop` on Linux, normally
`~/.local/share/dev.pobredux.desktop/`). Nothing is added to PoB XML. The store has format version 1;
each document is limited to 16,000 Unicode characters. Writes are serialized and atomically replaced.
Unreadable or unsupported stores are left untouched; build operations remain available.

The global document is shared by both games. Build documents use the game and a normalized absolute
file path. New and imported unsaved builds have only global memory. Opening a saved file restores its
notes. In-app rename and move (including folders) preserve the document; Save As copies notes to an
independent document and preserves any existing destination notes. Deleting a build in the app deletes
its document and invalidates pending proposals. External moves are not tracked. Documents with missing
build files remain visible and removable in assistant settings. Renaming or moving a document invalidates
pending proposals, so the user reviews a new proposal with the updated destination.

All assistant transports share these registry tools:

| Tool | Arguments and result |
|---|---|
| `memory_read` | No arguments. Returns `enabled`, current `game`/`build`, and `global`/`current` documents with opaque `id`, `revision` and Markdown `text`. Disabled memory returns no notes. |
| `memory_propose` | `document_id`, exact `revision`, replacement `text`, `reason`. Returns proposal `id`, `document_id`, and `status: pending`; never commits notes. Only global or the current saved build may be targeted. |
| `memory_proposal_status` | `proposal_id`. Returns identifiers and `pending`, `approved`, `rejected`, or `invalidated`. |

Approval, editing and deletion are separate UI commands and are never exposed as agent tools. Generic
chat write approval or its bypass cannot approve memory. Concurrent proposals use the original document
revision: approving one invalidates competitors. Manual edits and deletion also invalidate proposals.
Disabling memory blocks agent reads/proposals and invalidates every pending proposal, while keeping
user documents editable. Pending proposals survive app restarts and keep their original destination
when another build is opened.

API prompts and CLI messages (Claude, Codex and all ACP adapters) receive a refreshed memory snapshot
on each new message. MCP instructions ask external clients to read memory at the beginning and after
switching builds; those clients control whether their models follow the instructions. Notes are sent
to the assistant in use. Disabling cannot erase notes already in a conversation. Prefer durable user
preferences, goals and decisions; current user requests override notes and live facts come from PoB.
Memory emits `memory:changed`, never a build calculation change. Cloud sync, embeddings, semantic search,
CLAUDE.md import and automatic conversation summaries are outside this version.
