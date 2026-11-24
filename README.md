# Workshop

Workshop is a Windows desktop chat for working on a Git project with Codex. The lead can reply directly or assign parts of a task to workers. Replies stream into the conversation. Enter sends and Shift+Enter adds a line. Use **Chats** to search the current project’s conversations and reopen one. **Sidebar** toggles the chat list and opens it as a drawer in a narrow window. The desktop app keeps a successful direct chat connected for follow-ups and releases its connection when you switch chats, review changes or close the window.

## Run on Windows

Install Git, stable Rust with its C/C++ build tools, and the standalone Codex CLI. Open **Settings → Providers** to sign in. Workshop starts `codex.exe app-server` from PATH. Set `WORKSHOP_CODEX` to the full path to a standalone `codex.exe` to select another installation. Start Workshop from Explorer or a normal terminal.

```powershell
cargo run --locked --bin workshop
```

Use **Choose folder** for an existing repository root or a folder for a new project. For a new project, Workshop initializes Git and snapshots its contents, respecting Git ignores. The next launch opens the most recent project. You can also pass its folder as the first argument.

For automation, pass `--headless` first to run a command in the desktop executable without opening a window. It accepts the same commands, arguments and exit codes as `workshop-cli`. Set `WORKSHOP_HEADLESS=1` in a test process to make every invocation of `workshop.exe` there use the command-line interface. This example lists the project’s chats.

```powershell
.\target\release\workshop.exe --headless --project . list | Out-Host
```

## Working with agents

The Codex lead chooses how to split a task. New projects allow two workers at once. For example, the lead can assign a settings screen and a file importer in parallel, then start importer tests as soon as the importer passes its checks, even while work on the screen continues. Tasks that edit overlapping paths run in order.

Use **+** on the left of the composer to attach files, reference a folder, or paste text. Attachments appear above the draft and can be removed individually. Codex receives PNG, JPEG and WebP files as images, small UTF-8 files as text, and local paths for larger files, binary documents and folders. Keep the original files available when sending or retrying.

Open the model and effort pill beside Send. Select the header showing the effort and model to open the model list. Drag the slider or use the arrow keys to choose an effort level. New-chat choices set the project defaults. Changes within an existing chat apply to its next message. Workshop gets the available models and effort levels from the installed Codex provider.

Open **Subagents** beside **+**. Choose **Edit global defaults** to set allowed models and maximum effort across projects, or turn off **Use global defaults** to customize one chat. Turn it back on to follow future default changes. The lead can choose a different allowed model and a lower effort for each assignment. The parallel worker limit remains a project setting.

Choose **+ → Take screenshot**, type `/screenshot`, or ask “take a screenshot” to open the Windows region picker. Drag to send the selected area to the current chat. Escape cancels. The screenshot appears in the conversation and opens **Outputs** in the top right. Reopen Outputs to browse this chat’s screenshots, newest first. Select a thumbnail to see the full image. **View all screenshots** opens the same cross-project album as **Album** in the navigation rail. Screenshot commands and captured images stay out of provider requests. Use **Attach files** when you want the model to inspect an image.

## Themes

Open **Settings → Appearance** and select a theme to apply it across Workshop. Choose **New theme** to start with the current palette. Give it a name, paste color declarations or choose **Load file…** to read a local CSS file. The preview updates as you edit. **Save theme** applies it and remembers it across projects and restarts. Select a saved custom theme and choose **Edit theme** to change or remove it.

Workshop reads hex colors (`#rgb` or `#rrggbb`) from CSS custom properties. Paste a single `:root` block or the declarations inside it. You can start with just a few overrides. Missing variables inherit Workshop Dark. The default file lists every supported color, including surfaces, text, focus accents and usage meters. [Full theme template](themes/workshop-dark.css)

```css
:root {
  --canvas: #161922;
  --surface: #252a38;
  --accent: #9ddfc7;
}
```

## Provider settings

**Settings → Providers** checks the installed Codex and Claude Code CLIs and opens the provider's own sign-in flow. Accounts and extensions are shared with the corresponding CLI. Chat and worker execution currently use Codex. Use `WORKSHOP_CLAUDE` to select a specific Claude Code executable.

In **Settings → Plugins → Browse plugins**, search the selected provider’s official marketplaces by plugin name, ID or available description. Clear **Official marketplaces** to include your other configured sources.

**Plugins** lists the selected provider's installed plugins. Browse its marketplaces, choose **Install from source** to install by `plugin@marketplace`, or enter a marketplace repository and choose **Add marketplace**. **Skills** installs a local folder containing `SKILL.md` or a GitHub URL pointing to that folder. Removing a user skill moves it to a sibling `removed-skills` directory for recovery. Fresh provider sessions pick up changes.

When the lead delegates, expand its dispatch in the conversation or open **Activity** in the chat header to see all workers. Select a worker to see its assignment, acceptance criteria, dependencies, file scope, latest activity and reported usage. **Stop worker** interrupts that assignment while its sibling can continue. **Stop response** interrupts the whole team. Start a new chat to use delegation on projects with conversations created before H2.

Workers edit separate Git copies. Workshop integrates their changes into the lead’s copy and checks the combined revision. Open **Activity → Project handoff** to see the proposed decisions and next task. **Accept change** applies the verified edit to your project and saves the handoff for future chats. A decision-only turn uses **Accept decisions**.

## Game studio

Open **Studio** in the navigation rail. Select Images, Models or Ghidra in the adjacent sidebar. Each tool has a searchable run history beside its editor and preview. Select **Reuse settings** to start a variation from an earlier brief, seed or source file. You can continue chatting during a run, cancel it, and retry a failed or interrupted run. Reopening a project restores its library.

| Area | What you can make | Requirements |
|---|---|---|
| Images | Describe concept art, an inventory icon or a sprite sheet. **Seamless tile** generates a 256 × 256 PNG from a material and repeatable seed. | Prompted images use the signed-in Codex CLI’s image generation capability. Tiles run locally. |
| Models | Generate an editable prop, choose a crystal, barrel or planter starter, or import a local Blender scene. Select **Inspect model** on a finished preview to explore the exported mesh. **Open in Blender** opens its editable source. **Re-export** saves your edits as a new version. | Creating and re-exporting models uses Blender with its glTF exporter. **From brief** also uses Codex. Inspection runs inside Workshop. |
| Ghidra | Choose a compiled game module and optionally filter its functions by name. Inspect the recovered C pseudocode and save the report. By default, each run attempts 64 matching functions with up to 12 seconds per function. | An extracted Ghidra distribution and its supported JDK on PATH or in `JAVA_HOME`. |

In the model inspector, drag to orbit and scroll to zoom. The arrow keys and +/− also work. **Wireframe** shows triangle edges. **Overlaps** highlights coplanar faces that share surface area in orange, even behind another surface. The inspector reads static geometry and base material colors from GLB exports up to 64 MiB and 150,000 triangles. Dense models can show a partial overlap count when the comparison budget is reached.

**Settings → Tasks** saves independent defaults for image generation, 3D model design, and decompilation across projects. For the two design tasks, choose the allowed Codex models and maximum reasoning effort. Select one model to use it every time. In **Game studio → Run settings**, turn off **Use task defaults** to customize the next run. Retrying a run or reusing its settings restores its original choices.

Decompilation runs locally. Set **Functions per run** (1–256) and **Seconds per function** (1–120). Turn on **Generate an AI summary** to send up to 48 KiB of saved pseudocode to Codex, using the decompilation task’s own model and effort settings. Open **Summary** beside **Report** to read it. If Codex is unavailable, the pseudocode stays accessible and **Run log** records the summary error.

Use **Locate Blender** to select the executable or **Locate Ghidra** to select the installation root. Workshop remembers these choices across projects. You can also set `WORKSHOP_BLENDER` to an executable and `WORKSHOP_GHIDRA` to an installation root. Standard Windows and Steam installations of Blender are detected automatically.

In **Ghidra**, search the project history by binary name, function, entry point, filter or input SHA-256. **Run log** shows the progress of the selected run, including which functions were saved or skipped, any failure and whether Workshop reused an earlier result. **Report** opens the saved pseudocode. Both remain available after restarting Workshop or moving the original binary. Older runs retain their original tool logs.

Repeating a request for the same binary contents, normalized filter, function limit and per-function timeout reuses a completed report after checking its SHA-256. The new run records the source run ID. **Run fresh** runs the selected analysis again, for example after updating Ghidra. Reports created before input fingerprints were recorded remain searchable and readable.

**Save as…** exports the PNG, GLB or C report to a destination you choose. **Attach to chat** adds the selected file to the composer for your next message. In a new chat, you can also ask the lead to find a finished image or model by title and use it in a reviewable project change. Studio originals stay in the project’s Workshop data folder.

## CLI

Build the CLI alone with `cargo build --locked --no-default-features --bin workshop-cli`. Example commands after building follow below.

```powershell
$project = Join-Path $env:TEMP ("workshop-demo-" + [guid]::NewGuid())
$cli = ".\target\debug\workshop-cli.exe"
& $cli --project $project run "Create a README.md with a one-sentence description of this project."
& $cli --project $project list
& $cli --project $project diff 1
& $cli --project $project accept 1
```

The same studio jobs are available from the CLI. For example, use `studio texture Moss --seed 42`, `studio image "An isometric forest shrine"`, `studio prop "A low-poly lantern with a copper roof"`, `studio model Barrel "Dock barrel"`, `studio blend scene.blend`, or `studio decompile game.dll --filter TileCost`. Use `studio history Models` to inspect a project’s model runs.

`studio decompile` streams progress to stderr and prints the completed run as JSON on stdout. Pass `--fresh` to bypass reuse. Set `--function-limit` and `--seconds-per-function` to change local limits. Use `--summary` or `--summary=false` to override the saved summary preference. Image generation, prop design and enabled summaries accept `--model` and `--max-effort` for a single run. The read-only commands below also work while the desktop project is open. Codex leads and workers can perform the same lookups using `workshop_find_reports` and `workshop_read_report`.

```text
workshop-cli --project . studio reports tile_cost
workshop-cli --project . studio read-report 12 --part functions
workshop-cli --project . studio read-report 12 --offset 480
workshop-cli --project . studio read-report 12 --part log
workshop-cli --project . studio read-report 12 --part summary
```

Search returns run IDs and up to eight matching function locations per run. Use the saved byte offset to jump to a function, `next_offset` to continue a long report, and `next_before` with `reports --before` for older runs. The full Ghidra process output is available under `--part tool-log` for runs that launched Ghidra.

`run --model MODEL "task"` selects a provider model. Each run streams structured events to stdout. Type a direction or `/cancel` on stdin while it runs. Use `resume ID "direction"` to continue the same session in the same working copy. `retry ID` makes a fresh attempt from the project's current files. `discard ID` records the decision and retains the previous output. To run a project-specific command in the reviewed copy, use `check ID -- COMMAND [ARGS...]`. Close the desktop project before opening the same project in the CLI.

Use `team --parallelism 2` to enable the second worker. Use `team --check-args` with a JSON argument array such as `["cargo","test","--locked"]` to set the combined-revision check. The default is Git’s whitespace check. Worker state events include IDs for `/stop-worker ID` on stdin. After a turn, `agents ID` lists that chat’s assignments and handoff.

## What acceptance applies

- Each attempt gets a separate Git repository seeded from the creator's tracked and non-ignored working files. The starting point includes staged, unstaged and untracked work. A temporary index captures the snapshot.
- Review compares the starting point with the final working tree, including additions, deletions, binary files and commits made by the agent. The app stores the binary-capable patch, its SHA-256 digest, the base revision and the proposed tree.
- Acceptance verifies the saved patch, project revision and original contents of every affected path. A conflict leaves the creator's files available for a fresh attempt. Applying the patch changes the working files and preserves the creator's index. The project's Git attributes and line-ending rules apply.
- A journal entry precedes application. After an interruption the app compares each affected path with its original and proposed versions, applies any remaining changes, and records one acceptance decision.
- The Windows provider process and its descendants run inside a kill-on-close Job Object. Reopening a project captures the remaining work in an interrupted copy. A project lock coordinates desktop and CLI access.

Project records live under `%LOCALAPPDATA%\Robloxing\Workshop`. Override the parent with `WORKSHOP_HOME`. Each project contains a SQLite database and an `attempt-N` folder per run with the working copy, JSONL events, stderr, patches and check output. The `open` command prints the records path. Run checks only for projects whose code you intend to execute.

## Verification

Run `cargo test --locked --all-targets` for the offline recovery and scheduler tests. The Windows records cover the provider, Studio, native desktop and process cleanup. [H1 verification](docs/h1-verification.md) · [H2 verification](docs/h2-verification.md)

After a release build, run the Windows UI tests from PowerShell.

```powershell
.\tests\windows\picker_clicks.ps1
.\tests\windows\cancel_cycles.ps1
```

Each test runs on its own private Windows desktop. The tests exercise Workshop, its native file dialogs, and its screenshot picker there. The screenshot test selects pixels from a generated image. Use `-StudioOnly` with `picker_clicks.ps1` for the shorter Studio suite.

For an ad hoc UI check, dot-source `tests/windows/TestDesktop.ps1` and call `Invoke-WorkshopUiTest -Script .\my-check.ps1`. Inside the check, use `[TestDesktop]::StartApp($absoluteWorkshopExe, $absoluteProjectPath)` and `[TestDesktop]::WaitForWindow($process)` to drive the window on the private desktop. Script output and failures return to the caller.
