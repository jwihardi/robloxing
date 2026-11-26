# Workshop

Workshop is a Windows desktop chat for working on a Git project with Codex, Claude Code, Google Antigravity, or Gemini CLI. The lead can reply directly or assign parts of a task to workers. Chat providers run with full tool access. Replies stream into the conversation and render Markdown, links, tables, file attachments and inline images. External image previews load when you click **Load image**. Enter sends and Shift+Enter adds a line. Use **Chats** to search the current project’s conversations and reopen one. **Sidebar** toggles the chat list and opens it as a drawer in a narrow window. The composer stays available for follow-ups after edits have been applied, including when you reopen an accepted chat.

## Run on Windows

**[Download Workshop for Windows x64](https://github.com/jwihardi/robloxing/releases/latest/download/workshop-windows-x64.zip)**. Extract the ZIP to a folder and open `workshop.exe`. Keep `workshop-cli.exe` beside it for terminal commands and automation. [All releases and checksums](https://github.com/jwihardi/robloxing/releases)

Install Git and the CLI for each provider you want to use. Open **Settings → Providers** to sign in. Workshop starts the selected CLI and streams replies over its native protocol. Set `WORKSHOP_CODEX` to the full path to a standalone `codex.exe` to select another installation.

To build from source, install stable Rust with its C/C++ build tools and run the following command in the repository folder.

```powershell
cargo run --locked --bin workshop
```

Use **Choose folder** for an existing repository root or a folder for a new project. For a new project, Workshop initializes Git and snapshots its contents, respecting Git ignores. The next launch opens the most recent project. You can also pass its folder as the first argument.

For automation, pass `--headless` first to run a command in the desktop executable without opening a window. It accepts the same commands, arguments and exit codes as `workshop-cli`. Set `WORKSHOP_HEADLESS=1` in a test process to make every invocation of `workshop.exe` there use the command-line interface. This example lists the project’s chats.

```powershell
.\workshop.exe --headless --project . list | Out-Host
```

## Working with agents

To delete a conversation, hover over its row in **Chats** and click the trash icon, or choose **Chat options → Delete chat**. Confirm the title to remove the conversation and its private working copies. Accepted project files, saved assets and album screenshots stay available. From the CLI, use `workshop-cli --project PATH delete-chat ID` with the ID from `list`.

Give Workshop the outcome and any constraints. The lead handles quick questions directly. Automatic delegation is on by default. For a divisible project task the lead can assign independent work in parallel and add dependent work as results arrive. Each dependent worker receives the integrated files and its prerequisites’ findings, including read-only reviews. Overlapping edit scopes run in order.

Open **Settings → Tasks → Workers** to set scheduling for the current project. Turn off **Auto-delegate project tasks** to use workers only when you ask. **Allow two workers in this project** controls whether independent assignments can run together. The CLI equivalents are `workshop-cli team --auto-delegate false` and `workshop-cli team --parallelism 1`.

Workers can report unmet acceptance criteria and return a concrete finding to the lead. An unresolved owner holds dependent work in the queue. When the owner is repaired, its waiting tasks become eligible automatically. Workshop checks intermediate patches before releasing dependencies and runs the configured project command on the joined revision. With no configured command the automatic check is `git diff --check`. The lead and workers can also run the project’s own behavioral checks in their provider sessions.

If the lead finishes with an unresolved assignment or a failing final check, Workshop sends it the evidence in the same conversation. It can delegate a repair or fix the integrated copy itself. The transcript shows each automatic repair pass. Workshop stops after two extra passes, sooner if the same failure recurs on the same revision, the user stops an assignment, or the lead reports an external blocker. Verified changes reach the project automatically. To configure a project check, pass its executable and arguments as a JSON array to `workshop-cli team --check-args`.

Use **+** on the left of the composer to attach files, reference a folder, or paste text. Attachments appear above the draft and can be removed individually. The selected provider receives PNG, JPEG and WebP files as images, small UTF-8 files as text, and local paths for larger files, binary documents and folders. Keep the original files available when sending or retrying.

Type `/goal` on its own or at the end of a draft, then click **Goal** or press Enter or Tab. The composer keeps your text so you can edit the objective and press Enter to start. Escape or clicking **Goal** returns to a regular message with your text intact. You can also use **+ → Set a goal** or send `/goal Add a dark theme and verify both window sizes` directly.

The goal appears above the conversation. Select it to check progress, pause, resume, edit, or remove it. Workshop posts **Goal complete** in the chat after verification and pauses when the agent needs input or stops making progress. On the CLI, use `workshop-cli run "/goal your objective"` to start a goal and `workshop-cli resume ID "/goal"` to read its status.

Open the model and effort pill beside Send. Select the header showing the effort and model to open the model list. Drag the slider or use the arrow keys to choose an effort level. New-chat choices set the project defaults. Changes within an existing chat apply to its next message. Workshop gets the available models and effort levels from each installed CLI. Choose a Claude model for a Claude lead. Start a new chat to switch providers, or change models from the same provider between turns.

In **Settings → Tasks**, open **Workers**, **Image generation**, **Model design**, **Sound effects** or **Decompilation**. Switch providers on or off for that task and select models beneath their names. **Allow any model** clears an explicit selection. The model grid uses the full page. If a catalog is too large for the window, use the previous and next arrows to see the rest. Turn on **Generate an AI summary** to choose a model for a local decompilation report.

The same task permissions appear under **Settings → Providers**. Expand a provider to change its permitted tasks across projects. Codex and Antigravity support image generation. Turning off every provider for a task disables its AI routing.

Use **Settings → Tasks → Workers** to set global allowed models and maximum effort. For one project, `workshop-cli team --worker-model claude:sonnet` sets its worker model and `--worker-effort low` sets its effort limit. Each field you omit continues to use its existing setting. Run `workshop-cli team` to see the effective `worker_policy` or `workshop-cli team --inherit-workers` to restore the global defaults. To customize one chat, open **Subagents** beside **+** and turn off **Use global defaults** or **Use project defaults**. Turn it back on to follow future default changes. The lead can choose a different allowed model and a lower effort for each assignment. The parallel worker limit remains a project setting.

Open **Settings → Tasks → Workers → Level design** to choose models and a maximum effort for map layouts, blockouts, traversal and encounter pacing. These choices narrow the worker limits for each chat. To reserve that work for Claude, select the Claude models you want here and allow them under **General** or in the chat’s **Subagents** menu. Automatic dispatch prefers an eligible Claude model and targets Medium for level design. Routine workers target Low. Both respect the configured maximum and the model’s supported effort levels.

While the lead is working, Enter or **Steer now** sends a correction to the current run. For a separate follow-up, open **Send options** beside the send arrow and choose **Send after this run**. Messages wait above the composer in the order you added them. You can remove one before it starts. Each follow-up starts after the preceding run finishes its checks. Stopping a run or encountering a failure pauses delivery. The queue stays with the chat after restart. Choose **Send next** to continue.

To address a single worker, expand its dispatch or open **Activity**, select the worker, and choose **Send direction**. The composer names the recipient. Send your text or attachment there, or choose **Return to lead** to keep editing the draft. Directions appear in the chat, and the lead receives them with the worker results.

Choose **+ → Take screenshot**, type `/screenshot`, or ask “take a screenshot” to open the Windows region picker. Drag to send the selected area to the current chat. Escape cancels. The screenshot appears in the conversation and opens **Outputs** in the top right. Reopen Outputs to browse this chat’s images, models and screenshots, newest first. Select a thumbnail to see the full image. **View all screenshots** opens the same cross-project album as **Album** in the navigation rail. Screenshot commands and captured images stay out of provider requests. Use **Attach files** when you want the model to inspect an image.

Choose **+ → Draw a sketch** to open a drawing pad above the message box. Draw with the mouse or pen and type your explanation in the composer. **Undo stroke** removes the last mark. **Done** folds the sketch into the draft so you can edit or remove it before sending. Send a sketch by itself or with a caption. The selected model receives the drawing as a PNG, and clicking it in the conversation enlarges it.

To give feedback on an image, choose **Annotate** under a generated image in chat, on a finished image in **Tools → Saved assets**, or in the full-size screenshot viewer. Mark the area you mean, type the change you want, and send. Workshop keeps the original and sends a marked-up PNG at the source resolution. Undo removes pen strokes while keeping the underlying image.

Choose **Compare** under a returned image to drag a divider between versions. The arrow keys move the divider. Home and End reveal either side. **Show markup** brings back the strokes you sent, and **Annotate original** or **Annotate revision** starts another edit. For a text-only follow-up, Compare shows the previous image from that chat.

Enable **Settings → Notifications → Notify me when I’m needed** for Windows alerts while Workshop runs in the background. An alert opens the chat or decompilation report that needs attention. A blocked run, a new image or 3D model, and changes or decisions ready for review can trigger an alert.

## Themes

Open **Settings → Appearance** and select **Workshop Dark**, **Nord**, **Catppuccin Mocha**, or **Solarized Light** to apply it across Workshop. Choose **New theme** to start with the current palette. Give it a name, paste color declarations or choose **Load file…** to read a local CSS file. The preview updates as you edit. **Save theme** applies it and remembers it across projects and restarts. Select a saved custom theme and choose **Edit theme** to change or remove it.

Workshop reads hex colors (`#rgb` or `#rrggbb`) from CSS custom properties. Paste a single `:root` block or the declarations inside it. Missing variables inherit Workshop Dark. If you set `--accent`, symbols and keywords also use that color unless you give them their own values. [Full theme template](themes/workshop-dark.css)

```css
:root {
  --canvas: #161922;
  --surface: #252a38;
  --accent: #9ddfc7;
  --symbol: #a7bdff;
  --keyword: #d4aff2;
}
```

## Provider settings

**Settings → Providers** shows the installed providers and opens their sign-in flows. Accounts and extensions are shared with the corresponding CLI. Codex, Claude, Antigravity, and Gemini CLI can each lead a chat or run its workers. With unrestricted subagent settings, an assignment that omits its model uses the lead’s provider. Select a single allowed worker model to fix the team pairing. Use `WORKSHOP_CLAUDE` to select a specific Claude Code executable.

For a personal Google account, install Antigravity CLI and sign in under **Settings → Providers → Antigravity**. Workshop detects the standard Windows installation even when `agy` is outside PATH. The picker reads your signed-in catalog and groups each Gemini family’s effort variants on one slider. Google-hosted Claude and GPT models appear under their own Antigravity names. The usage meter follows the selected model’s quota group. Set `WORKSHOP_ANTIGRAVITY` to use another executable. [Antigravity CLI](https://antigravity.google/docs/cli/acp/)

For a headless chat, run `workshop-cli models --provider antigravity`, then `workshop-cli run --provider antigravity "Describe this project"`. Choose a family and effort with `--model gemini-3.8-flash --effort low` if your account lists that model. The same model choices are available for worker and task settings.

For an organization account or API key on the separate Gemini CLI, install it with `npm install -g @google/gemini-cli`. Under **Settings → Providers → Gemini CLI**, choose **Sign in**, then select **Gemini Default** or a specific Gemini model in a new chat. **Gemini project tools → Trust project** allows Gemini to load that project’s configuration and Workshop worker tools in its isolated copies. **Reset trust** restores the CLI’s own trust rules. For headless use, run `workshop-cli gemini-trust true` in the project, `workshop-cli models --provider gemini` to list models, and `workshop-cli run --provider gemini "Describe this project"` to chat. Gemini manages reasoning effort. Set `WORKSHOP_GEMINI` to an executable or the npm JavaScript entry point to select another installation. [Gemini CLI authentication](https://geminicli.com/docs/get-started/authentication/)

Workshop’s Claude picker shows the model versions reported by your installed Claude Code CLI. The default row shows which model Claude Code recommends for your setup. Selecting a family such as Sonnet follows that alias when Claude Code updates its version. [Claude Code model configuration](https://code.claude.com/docs/en/model-config)

For Claude headless use, list selection IDs, supported effort levels, and resolved model IDs with `workshop-cli models --provider claude`. For example, `workshop-cli run --provider claude --model sonnet "Describe this project"` starts a Claude chat. To pair a Codex lead with a Claude worker in a new project, use `workshop-cli team --lead-model gpt-6.2-sol --worker-model claude:sonnet`. Claude runs in the isolated Git copy with its native edit permissions. Its account meter uses the limit information the CLI reports.

In **Settings → Plugins → Recommended**, choose Codex, Claude Code or Antigravity and search by task, such as “multiplayer,” “Blender” or “playtest.” Each game development skill installs individually for that provider. **In Skills** identifies an existing folder with the same skill name whose source Workshop cannot confirm. You can inspect its location under **Settings → Skills**. [Recommended game development skills](#recommended-game-development-skills)

Recommended also includes **Frontend Design** for interface layout, **Caveman** for terse replies and **Image to Sculpt** for turning reference art into a Blender sculpt. Image to Sculpt requires Python with its scientific packages and Blender 5.2+. Its generated views use a paid fal API key. [Image to Sculpt setup](https://github.com/gokayfem/image-to-sculpt#install)

In **Settings → Plugins → Browse plugins**, search the selected provider’s official marketplaces by plugin name, ID or available description. Clear **Official marketplaces** to include your other configured sources.

**Settings → MCP servers → Blender MCP** shares a local stdio connection with Codex, Claude Code, Antigravity and Gemini CLI. **Connect** imports an enabled Blender entry from Codex or tries the installed `blender-mcp` command. **Browse…** selects a different server executable. Workshop checks the tool list before saving the connection. To inspect or edit a live scene, start the Blender Lab add-on in Blender, then ask the agent about that scene in chat. For a private background scene, Blender also supports `blender --background scene.blend --command blender_mcp`. [Blender Lab MCP](https://www.blender.org/lab/mcp-server/)

The CLI uses the same setting. Run `workshop-cli blender-mcp connect`, `test`, `status` or `disconnect`. For a virtual environment, use `workshop-cli blender-mcp connect --command PATH_TO_PYTHON --arg=-m --arg=blmcp`. Disconnecting turns off Workshop’s shared connection for new sessions. Each provider still manages its own configured servers.

**Plugins** lists the selected provider's installed plugins. Browse its marketplaces, choose **Install from source** to install by `plugin@marketplace`, or enter a marketplace repository and choose **Add marketplace**. Antigravity browses Google’s published plugin directory and supports installing by plugin name, GitHub URL, or local path. **Skills** installs a local folder containing `SKILL.md` or a GitHub URL pointing to that folder. Removing a user skill moves it to a sibling `removed-skills` directory for recovery. Fresh provider sessions pick up changes.

When the lead delegates, expand its dispatch in the conversation or open **Activity** in the chat header to see all workers. Each worker shows its model and reasoning effort under its title. Select a worker to see the allowed models and maximum effort for that run, its assignment, acceptance criteria, dependencies, file scope, latest activity and reported usage. **Stop worker** interrupts that assignment while its sibling can continue. **Stop response** interrupts the whole team. Start a new chat to use delegation on projects with conversations created before H2.

Workers edit separate Git copies. Workshop integrates their changes into the lead’s copy and checks the combined revision. Open **Activity → Project handoff** to see the decisions and next task. Workshop saves the handoff automatically, including for turns that produce only decisions.

**Chats → Work queue** shows what needs your attention across the current project. Each row gives the chat, provider, worker activity and next action. Open a row to answer a blocker, resume work or review a change. The queue follows the attempt you most recently started or resumed, including an older retry. A plain chat reply drops out of the queue when it finishes. From another terminal, `workshop-cli --project PATH status --json` reads the same queue while the desktop is running. Use `--offset` and `--limit` to page through it.

After a shutdown interrupts an active goal, open the chat’s **Goal paused → Resume goal** or run `workshop-cli --project PATH recover-goal`. Both continue in the saved working copy with the original provider and last recorded progress. Pass a chat ID to the command to select a particular goal. Headless recovery skips manually paused goals and outstanding requests for your input. Verified changes are applied automatically when the turn finishes.

## Recommended game development skills

For game audio, **Game Audio Design** covers mixing and effect variation. **ElevenLabs Sound Effects** adds hosted text-to-audio for Foley and ambience and requires an ElevenLabs API key and credits. Both are available under **Settings → Plugins → Recommended**. [Game Audio Design](https://github.com/gamedev-skills/awesome-gamedev-agent-skills/tree/main/skills/disciplines/audio-design) · [ElevenLabs Sound Effects](https://github.com/elevenlabs/skills/tree/main/sound-effects)

The install counts below are rounded figures reported by the skills.sh directory for each individual skill on November 25, 2026.

| Skill | When to use it | Recorded installs |
| --- | --- | ---: |
| Roblox Studio Workflow | Edit an existing place or Rojo project and verify it in Studio. | [12K+](https://www.skills.sh/gamedev-skills/awesome-gamedev-agent-skills/roblox-studio-workflow) |
| Roblox Luau | Write typed scripts and decide what belongs on the client or server. | [15K+](https://www.skills.sh/gamedev-skills/awesome-gamedev-agent-skills/roblox-luau) |
| Roblox Networking | Design multiplayer remotes and test replication with two clients. | [12K+](https://www.skills.sh/gamedev-skills/awesome-gamedev-agent-skills/roblox-networking) |
| Roblox UI | Implement a HUD or shop in ScreenGui with touch and gamepad navigation. | [12K+](https://www.skills.sh/gamedev-skills/awesome-gamedev-agent-skills/roblox-ui) |
| Game UI & UX | Lay out an inventory or menu that stays readable across screen sizes. Pairs with Roblox UI. | [21K+](https://www.skills.sh/gamedev-skills/awesome-gamedev-agent-skills/game-ui-ux) |
| Roblox Building | Build props and maps in Studio and check scale, collisions and traversal. | [4.4K](https://www.skills.sh/tabooharmony/roblox-brain/roblox-building) |
| 3D Modeling | Plan topology, unwrap UVs and prepare levels of detail for a game mesh in Blender or Maya. | [1.3K](https://www.skills.sh/omer-metin/skills-for-antigravity/3d-modeling) |
| Gameplay Prototyping | Build and playtest one mechanic to decide whether to keep it. | [15K+](https://www.skills.sh/gamedev-skills/awesome-gamedev-agent-skills/prototype-fast) |
| Game Feel | Give actions more impact with timing, hit-stop, shake and squash-and-stretch. | [21K+](https://www.skills.sh/gamedev-skills/awesome-gamedev-agent-skills/game-feel) |

## Images, sound effects and 3D models in chat

Describe what you want in the composer. For example, ask “Draw a moonlit harbor icon” or “Build a low-poly copper lantern I can rotate.” The lead chooses a generator, uses the providers allowed in Settings and posts the result in the conversation. **Stop response** also stops the active generation.

- **Images** arrive as PNGs. Enlarge, save or annotate an image and send feedback in the same chat. Prompted artwork uses Codex or Antigravity. Seamless moss, basalt and sand tiles can use the local generator with a repeatable seed.
- **Sound effects** arrive as playable WAVs with a waveform. Try “Make a coin pickup with two rising chimes,” then “Make the second chime softer.” The lead can pass the first sound’s asset ID to revise its design. **Settings → Tasks → Sound effects** controls the designer’s provider, model and maximum effort. By default Workshop synthesizes up to 12 seconds of layered tones and filtered noise locally. For hosted Foley and ambience, set `ELEVENLABS_API_KEY` in Workshop’s environment, restart, and enable **Use ElevenLabs SFX v3** on that tab. This sends the sound brief to ElevenLabs and uses your credits. You can save a WAV from chat, replay it in Outputs, or reuse it from Saved assets.
- **3D models** arrive with a preview, **Inspect model** and **Save model**. The lead can commission a design, use a crystal, barrel or planter starter, or export an attached Blender scene. Ask for a variation to create a new version. Blender builds each GLB and its editable scene.
- **Outputs** collects the sound effects, images, models and screenshots for the current chat. **Tools → Saved assets** searches generated assets across chats in the project, including work made in earlier versions. Ask the lead to add an asset to the project when it should be included in a reviewed change.

In the model inspector, drag to orbit and scroll to zoom. The arrow keys and +/− also work. **Wireframe** shows triangle edges. **Overlaps** highlights coplanar faces that share surface area in orange, even behind another surface. The inspector reads static geometry and base material colors from GLB exports up to 64 MiB and 150,000 triangles. Dense models can show a partial overlap count when the comparison budget is reached.

**Settings → Providers** controls which tasks each provider can handle. **Settings → Tasks** sets allowed models and maximum reasoning effort for image generation, model design and local sound effect design. You can request a particular model or effort in your message. The lead stays within those settings.

### Reusable Blender asset packs

Ask in chat, “Create a Harbor pack with a blue-green buoy and a weathered dock crate. Keep the fittings chunky and use the same painted iron.” In another chat, try “Inspect the Harbor pack, reuse its crate, and make a matching lantern.” Leads and workers share the project's library. They can inspect a rendered preview, palette and dimensions, copy the exact editable collection or GLB into their working copy, and use a particular pack member as the visual reference for a new design.

Open **Tools → Asset packs** to browse the art direction and preview each model. **Use in chat** adds the selected pack and member to your draft. **Export pack** creates a portable folder with the style guide, Blender catalog, marked collections, packed textures, GLBs, previews and JSON manifests. Add the exported folder as a library in Blender's **Preferences → File Paths → Asset Libraries**, or choose **Import pack** in another Workshop project. Import verifies each file before making the pack available.

External agents can use the same library headlessly while the desktop is open. These commands publish the named collection from an existing Blender scene and inspect its files.

```powershell
workshop-cli --project . packs create harbor --name "Harbor" --style "Weathered cedar, blue iron, human-scale fittings."
workshop-cli --project . packs add harbor crate .\props.blend --collection "Dock crate" --name "Dock crate" --description "Stackable cedar cargo crate."
workshop-cli --project . packs show harbor crate
workshop-cli --project . packs export harbor .\Libraries
```

## Decompilation

Open **Tools → Decompilation**, choose a binary and optionally filter functions by name. Use an extracted Ghidra distribution with its supported JDK on PATH or in `JAVA_HOME`. **Run settings** lets you override the defaults for one analysis. **Reuse settings** restores an earlier analysis configuration.

Decompilation runs locally. Set **Functions per run** (1–256) and **Seconds per function** (1–120). Turn on **Generate an AI summary** to send up to 48 KiB of saved pseudocode to the selected provider, using the decompilation task’s own model and effort settings. Open **Summary** beside **Report** to read it. If the selected provider is unavailable, the pseudocode stays accessible and **Run log** records the summary error.

Use **Locate Blender** to select the executable or **Locate Ghidra** to select the installation root. Workshop remembers these choices across projects. You can also set `WORKSHOP_BLENDER` to an executable and `WORKSHOP_GHIDRA` to an installation root. Standard Windows and Steam installations of Blender are detected automatically.

In **Decompilation**, search the project history by binary name, function, entry point, filter or input SHA-256. **Run log** shows the progress of the selected run, including which functions were saved or skipped, any failure and whether Workshop reused an earlier result. **Report** opens the saved pseudocode. Both remain available after restarting Workshop or moving the original binary. Older runs retain their original tool logs.

Repeating a request for the same binary contents, normalized filter, function limit and per-function timeout reuses a completed report after checking its SHA-256. The new run records the source run ID. **Run fresh** runs the selected analysis again, for example after updating Ghidra. Reports created before input fingerprints were recorded remain searchable and readable.

**Save as…** exports the PNG, GLB or C report to a destination you choose. **Attach to chat** adds the selected file to the composer for your next message. In a new chat, you can also ask the lead to find a finished image or model by title and use it in a reviewable project change. Saved originals stay in the project’s Workshop data folder.

## CLI

Build the CLI alone with `cargo build --locked --no-default-features --bin workshop-cli`. Example commands after building follow below.

```powershell
$project = Join-Path $env:TEMP ("workshop-demo-" + [guid]::NewGuid())
$cli = ".\target\debug\workshop-cli.exe"
& $cli --project $project run "Create a README.md with a one-sentence description of this project."
& $cli --project $project list
& $cli --project $project diff 1
```

The same studio jobs are available from the CLI. For example, use `studio texture Moss --seed 42`, `studio image "An isometric forest shrine"`, `studio prop "A low-poly lantern with a copper roof"`, `studio model Barrel "Dock barrel"`, `studio blend scene.blend`, or `studio decompile game.dll --filter TileCost`. Use `studio history Models` to inspect a project’s model runs.

`studio decompile` streams progress to stderr and prints the completed run as JSON on stdout. Pass `--fresh` to bypass reuse. Set `--function-limit` and `--seconds-per-function` to change local limits. Use `--summary` or `--summary=false` to override the saved summary preference. Use `studio sound "Two rising coin chimes"` for headless sound generation. Image generation, sound effects, prop design and enabled summaries accept `--model` and `--max-effort` for a single run. The read-only commands below also work while the desktop project is open. Leads and workers from either provider can perform the same lookups using `workshop_find_reports` and `workshop_read_report`.

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

On stdin, plain text steers the lead, `/steer-worker ID MESSAGE` directs one worker, and `/queue MESSAGE` adds a follow-up. The chat records a direction when Workshop sends it to the provider. If delivery is interrupted, the transcript shows the original direction with a delivery notice. After a stopped run, `workshop-cli send-next ID` continues that chat’s saved queue. If startup stopped before the provider saved a session, **Send next** reopens the original request in its working copy and delivers the follow-ups in order. **Retry** carries that queue into the new attempt.

## How chat edits reach your project

- Each attempt gets a separate Git repository seeded from the creator's tracked and non-ignored working files. The starting point includes staged, unstaged and untracked work. A temporary index captures the snapshot.
- Review compares the starting point with the final working tree, including additions, deletions, binary files and commits made by the agent. The app stores the binary-capable patch, its SHA-256 digest, the base revision and the proposed tree.
- Completed chat turns automatically apply verified edits. Follow-ups use the same chat and apply just their new edits. Workshop verifies the saved patch, project revision and original contents of every affected path. A conflict leaves the creator's files available for a fresh attempt. Applying the patch changes the working files and preserves the creator's index. The project's Git attributes and line-ending rules apply.
- A journal entry precedes application. After an interruption the app compares each affected path with its original and proposed versions, applies any remaining changes, and records one acceptance decision.
- The Windows provider process and its descendants run inside a kill-on-close Job Object. Reopening a project captures the remaining work in an interrupted copy. A project lock coordinates desktop and CLI access.

Project records live under `%LOCALAPPDATA%\Robloxing\Workshop`. Override the parent with `WORKSHOP_HOME`. Each project contains a SQLite database and an `attempt-N` folder per run with the working copy, JSONL events, stderr, patches and check output. The `open` command prints the records path. Run checks only for projects whose code you intend to execute.

## Verification

Run `cargo test --locked --all-targets` for the offline recovery and scheduler tests. The Windows records cover the provider, Studio, native desktop and process cleanup. [H1 verification](docs/h1-verification.md) · [H2 verification](docs/h2-verification.md)

After a release build, run the Windows UI tests from PowerShell.

For asset packs, run `py -3 tests/blender_packs.py --cli target/release/workshop-cli.exe --blender "<path to blender.exe>" --keep-in "<new test folder>"`. This publishes two textured collections in parallel and opens them in a clean Blender process. Pass `-PackFixture "<test folder>/portable/harbor"` to the picker suite to exercise importing, browsing, exporting and generating a matching model through chat.

```powershell
.\tests\windows\picker_clicks.ps1
.\tests\windows\cancel_cycles.ps1
```

Each test runs on its own private Windows desktop. The tests exercise Workshop, its native file dialogs, and its screenshot picker there. The screenshot test selects pixels from a generated image. Use `-StudioOnly` with `picker_clicks.ps1` for the chat asset suite or `-SketchOnly` for drawing, image markup, and message delivery.

For an ad hoc UI check, dot-source `tests/windows/TestDesktop.ps1` and call `Invoke-WorkshopUiTest -Script .\my-check.ps1`. Inside the check, use `[TestDesktop]::StartApp($absoluteWorkshopExe, $absoluteProjectPath)` and `[TestDesktop]::WaitForWindow($process)` to drive the window on the private desktop. Script output and failures return to the caller.
