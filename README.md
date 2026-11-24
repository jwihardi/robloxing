# Workshop

Workshop is a single-agent Windows desktop app for a Git project. Give Codex an assignment, follow its activity, and send direction while it works. Reopen the project to continue an interrupted run. The CLI can display the full diff.

## Run on Windows

Install Git, stable Rust with its C/C++ build tools, and the standalone Codex CLI. Run `codex login` through the provider and confirm `codex login status`. Workshop starts `codex.exe app-server` from PATH. Set `WORKSHOP_CODEX` to the full path to a standalone `codex.exe` to select another installation. Start Workshop from Explorer or a normal terminal.

```powershell
cargo run --locked --bin workshop
```

Use **Choose folder** for an existing repository root or a folder for a new project. For a new project, Workshop initializes Git and snapshots its contents, respecting Git ignores. The next launch opens the most recent project. You can also pass its folder as the first argument.

## Try the failing Luau fixture

Run these commands from this repository in PowerShell. The demo uses a fresh directory so the fixture stays reusable. Install Rokit for the pinned Luau and Rojo tools, or put the Luau CLI directly on PATH.

```powershell
$project = Join-Path $env:TEMP ("salvage-" + [guid]::NewGuid())
Copy-Item fixtures/salvage $project -Recurse
Push-Location $project
try {
    rokit install
    luau check.luau
} finally { Pop-Location }
cargo run --locked --bin workshop -- $project
```

The initial check fails on collecting two of five items. Assign **Fix the failing capacity test by editing src/Salvage.luau.** Once the agent edits the file, close the app, then reopen the same folder. Select the recovered run, read its activity, choose **Run check**, then **Accept change**. To inspect the patch first, close the window and use the CLI diff command below. Run `luau check.luau` in the project folder to verify the accepted result. Repeat with a fresh copy and an uncommitted comment in the same module.

## CLI

Build the CLI alone with `cargo build --locked --no-default-features --bin workshop-cli`. Example commands after building follow below.

```powershell
$cli = ".\target\debug\workshop-cli.exe"
& $cli --project $project run "Fix the failing Luau test."
& $cli --project $project list
& $cli --project $project diff 1
& $cli --project $project check 1 -- luau check.luau
& $cli --project $project accept 1
```

`run --model MODEL "task"` selects a provider model. Each run streams structured events to stdout. Type a direction or `/cancel` on stdin while it runs. Use `resume ID "direction"` to continue the same session in the same working copy. `retry ID` makes a fresh attempt from the project's current files. `discard ID` records the decision and retains the previous output. Close the desktop project before opening the same project in the CLI.

## What acceptance applies

- Each attempt gets a separate Git repository seeded from the creator's tracked and non-ignored working files. The starting point includes staged, unstaged and untracked work. A temporary index captures the snapshot.
- Review compares the starting point with the final working tree, including additions, deletions, binary files and commits made by the agent. The app stores the binary-capable patch, its SHA-256 digest, the base revision and the proposed tree.
- Acceptance verifies the saved patch, project revision and original contents of every affected path. A conflict leaves the creator's files available for a fresh attempt. Applying the patch changes the working files and preserves the creator's index. The project's Git attributes and line-ending rules apply.
- A journal entry precedes application. After an interruption the app compares each affected path with its original and proposed versions, applies any remaining changes, and records one acceptance decision.
- The Windows provider process and its descendants run inside a kill-on-close Job Object. Reopening a project captures the remaining work in an interrupted copy. A project lock coordinates desktop and CLI access.

Project records live under `%LOCALAPPDATA%\Robloxing\Workshop`. Override the parent with `WORKSHOP_HOME`. Each project contains a SQLite database and an `attempt-N` folder per run with the working copy, JSONL events, stderr, patches and check output. The `open` command prints the records path. Run checks only for projects whose code you intend to execute.

## Verification

Run `cargo test --locked --all-targets` for the offline recovery tests. The Windows probe notes include the real provider, Studio and desktop runs. [H1 verification](docs/h1-verification.md)
