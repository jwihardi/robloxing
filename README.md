# Workshop

Workshop is a Windows desktop chat for working on a Git project with Codex. The lead can reply directly or assign parts of a task to workers. Replies stream into the conversation. Enter sends and Shift+Enter adds a line. Reopen a chat from the sidebar to continue it. The desktop app keeps a successful direct chat connected for follow-ups and releases its connection when you switch chats, review changes or close the window.

## Run on Windows

Install Git, stable Rust with its C/C++ build tools, and the standalone Codex CLI. Open **Settings → Providers** to sign in. Workshop starts `codex.exe app-server` from PATH. Set `WORKSHOP_CODEX` to the full path to a standalone `codex.exe` to select another installation. Start Workshop from Explorer or a normal terminal.

```powershell
cargo run --locked --bin workshop
```

Use **Choose folder** for an existing repository root or a folder for a new project. For a new project, Workshop initializes Git and snapshots its contents, respecting Git ignores. The next launch opens the most recent project. You can also pass its folder as the first argument.

## Working with agents

The Codex lead chooses how to split a task. New projects allow two workers at once. For example, the lead can assign a settings screen and a file importer in parallel, then start importer tests as soon as the importer passes its checks, even while work on the screen continues. Tasks that edit overlapping paths run in order.

Use **+** on the left of the composer to attach files, reference a folder, or paste text. Attachments appear above the draft and can be removed individually. Codex receives PNG, JPEG and WebP files as images, small UTF-8 files as text, and local paths for larger files, binary documents and folders. Keep the original files available when sending or retrying.

Open the model and effort pill beside Send. Select the header showing the effort and model to open the model list. Drag the slider or use the arrow keys to choose an effort level. New-chat choices set the project defaults. Changes within an existing chat apply to its next message. Workshop gets the available models and effort levels from the installed Codex provider.

Open **Subagents** beside **+**. Choose **Edit global defaults** to set allowed models and maximum effort across projects, or turn off **Use global defaults** to customize one chat. Turn it back on to follow future default changes. The lead can choose a different allowed model and a lower effort for each assignment. The parallel worker limit remains a project setting.

Choose **+ → Take screenshot**, type `/screenshot`, or ask “take a screenshot” to open the Windows region picker. Drag to send the selected area to the current chat. Escape cancels. The screenshot appears in the conversation and opens **Outputs** in the top right. Reopen Outputs to browse this chat’s screenshots, newest first. Select a thumbnail to see the full image. **View all screenshots** opens the same cross-project album as **Screenshots** in the sidebar. Screenshot commands and captured images stay out of provider requests. Use **Attach files** when you want the model to inspect an image.

## Provider settings

**Settings → Providers** checks the installed Codex and Claude Code CLIs and opens the provider's own sign-in flow. Accounts and extensions are shared with the corresponding CLI. Chat and worker execution currently use Codex. Use `WORKSHOP_CLAUDE` to select a specific Claude Code executable.

In **Settings → Plugins → Browse plugins**, search the selected provider’s official marketplaces by plugin name, ID or available description. Clear **Official marketplaces** to include your other configured sources.

**Plugins** lists the selected provider's installed plugins. Browse its marketplaces, install by `plugin@marketplace`, or enter a marketplace repository and choose **Add marketplace**. **Skills** installs a local folder containing `SKILL.md` or a GitHub URL pointing to that folder. Removing a user skill moves it to a sibling `removed-skills` directory for recovery. Fresh provider sessions pick up changes.

When the lead delegates, its workers appear underneath it. Select a worker to see its assignment, file scope, latest activity, retries and reported token usage. **Stop worker** interrupts that assignment while its sibling can continue. **Stop response** interrupts the whole team. Start a new chat to use delegation on projects with conversations created before H2.

Workers edit separate Git copies. Workshop integrates their changes into the lead’s copy and checks the combined revision. Open **Project handoff** to see the proposed decisions and next task. **Accept change** applies the verified edit to your project and saves the handoff for future chats. A decision-only turn uses **Accept decisions**.

## Game studio

Open **Game studio** in the project sidebar. Each tab keeps a searchable history of its runs and outputs. Select **Reuse settings** to start a variation from an earlier brief, seed or source file. You can continue chatting during a run, cancel it, and retry a failed or interrupted run. Reopening a project restores its library.

| Area | What you can make | Requirements |
|---|---|---|
| Images | Describe concept art, an inventory icon or a sprite sheet. **Seamless tile** generates a 256 × 256 PNG from a material and repeatable seed. | Prompted images use the signed-in Codex CLI’s image generation capability. Tiles run locally. |
| Models | Generate an editable prop, choose a crystal, barrel or planter starter, or import a local Blender scene. Select **Inspect model** on a finished preview to explore the exported mesh. **Open in Blender** opens its editable source. **Re-export** saves your edits as a new version. | Creating and re-exporting models uses Blender with its glTF exporter. **From brief** also uses Codex. Inspection runs inside Workshop. |
| Ghidra | Choose a compiled game module and optionally filter its functions by name. Inspect the recovered C pseudocode and save the report. Each run attempts up to 64 matching functions. | An extracted Ghidra distribution and its supported JDK on PATH or in `JAVA_HOME`. |

In the model inspector, drag to orbit and scroll to zoom. The arrow keys and +/− also work. **Wireframe** shows triangle edges. **Overlaps** highlights coplanar faces that share surface area in orange, even behind another surface. The inspector reads static geometry and base material colors from GLB exports up to 64 MiB and 150,000 triangles. Dense models can show a partial overlap count when the comparison budget is reached.

Use **Locate Blender** to select the executable or **Locate Ghidra** to select the installation root. Workshop remembers these choices across projects. You can also set `WORKSHOP_BLENDER` to an executable and `WORKSHOP_GHIDRA` to an installation root. Standard Windows and Steam installations of Blender are detected automatically.

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

After a release build, run `.\tests\windows\picker_clicks.ps1 -StudioOnly` for the native game studio controls, including cancellation, retry, search, settings reuse, file export and chat attachments. The suite uses a local fixture provider. Omit the switch to include the desktop screenshot and composer regressions, which require the test window to be visible on screen.
