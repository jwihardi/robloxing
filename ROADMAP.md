# Roblox development harness roadmap

Build a lightweight Windows workspace where a creator can describe a game, direct a lead agent, and keep making progress on the same playable project over several sessions. Start with Roblox. Keep provider sessions, tasks, checkpoints, and artifact storage independent of the engine so a second engine can reuse them later.

## Release map

| Release | What the creator can do | Depends on |
|---|---|---|
| H1 · Single-agent harness | Open a project, assign a task, review a real change, and resume after closing the app. | A Windows repo and one working provider adapter |
| H2 · Basic orchestrated harness | Choose Claude, Codex, or both. Have a lead delegate, integrate work, and recover from failure. | H1 |
| R1 · First playable Roblox release | Take a small game from idea to a tested multiplayer round. Change the design the next day and preserve the working parts. | H2 plus a working Studio bridge |
| R2 · Creator production tools | Make maps, assets, UI, and reusable game systems across supported device types. | R1 plus the relevant authoring or service integration |
| R3 · Team release workflow | Collaborate, test a release candidate in staging, publish deliberately, and restore a place revision. | A stable revision model and test suite |
| R4 · Live game operations | Connect player outcomes and runtime issues to a revision, make a targeted change, and evaluate the result. | R3 and event instrumentation |

**First development target:** H2 is a usable general harness. **First creator-facing target:** R1 is the earliest release worth putting in front of Roblox developers. Use a small Roblox repo during H1 and H2 to expose integration problems early. For one developer familiar with systems programming, budget roughly 3–6 full-time weeks through H2 and another 3–6 through R1 as an initial planning estimate. Re-estimate after the Windows provider and Studio experiments below.

## H1 · One task, one agent, one recoverable change

1. **Prove the process boundary.** From a small Rust CLI, run a real task through Codex app-server on Windows and collect structured events. Repeat with Claude Code's programmatic interface. Probe model selection, authentication, cancellation, session resume, and child-process cleanup against the installed versions. Implement one adapter end to end first. Use each provider's documented sign-in or billing flow. [Codex app-server](https://learn.chatgpt.com/docs/app-server) · [Claude programmatic runs](https://code.claude.com/docs/en/headless) · [Claude credential rules](https://code.claude.com/docs/en/legal-and-compliance)
2. **Persist the work.** Use SQLite for projects, milestones, tasks, attempts, decisions, events, and artifact references. Record the base revision and provider session ID for each attempt. Keep full logs and media on disk. A project can start from an existing Git repo or a new project with an initial snapshot.
3. **Ship a small desktop UI.** Start with Rust and Slint. Show the active milestone, task list, current action, and reviewable diff. Include submit, steer, cancel, retry, accept, and discard. Preserve pre-existing creator edits. Give every change an isolated working copy and record exactly what acceptance will apply.
4. **Recover cleanly.** On restart, reconcile incomplete work with the saved patch and revision. Identify interrupted attempts, preserve their output, and allow resumption or a new attempt. Make applying an already accepted patch safe to retry.

**Exit demo:** Ask one agent to fix a failing Luau module test. Close the app after it edits the file, reopen it, inspect the diff, rerun the test, and accept the fix once. Repeat with a pre-existing user edit and confirm it survives.

## H2 · Add orchestration and bound its resource use

| Build | Behavior required for the basic harness |
|---|---|
| Lead and worker roles | The creator picks each role's provider and model. The lead owns design decisions, task dependencies, and acceptance criteria. A worker gets the relevant files, interfaces, edit scope, and one concrete deliverable. The lead can handle a small task directly. |
| Scheduler | Persist tasks as queued, running, blocked, ready for review, or finished. Run one worker by default and permit two independent workers concurrently. Start processes on demand. Treat shared-file changes as dependent work. |
| Integration | Workers return patches against known revisions. Apply compatible patches to an integration worktree and rerun checks on the combined state. Route a conflict or failing check back to its owner with the failing output. |
| Continuity and control | Store accepted design decisions and the next unfinished task. Hand that compact record to either provider. Show active workers, retry count, and provider-reported usage when available. Stop repeated identical failures and return the blocker to the lead. |
| Resource discipline | Use event-driven I/O, bounded in-memory logs, and paged history. Queueing tasks should create zero idle worker processes. Closing a project should release its workers and tools. |

**Exit demo:** Run the same small feature with Claude alone, Codex alone, and a mixed lead/worker pair. Reverse the mixed roles once. Interrupt one worker, create a conflicting edit, and recover without duplicating changes. Continue the next milestone from the other provider with a fresh conversation.

**Measure on Windows:** Record private working set, private committed bytes, and CPU over the same idle interval for the app alone, one worker, two workers, and a Studio playtest. Record Studio alone as the baseline. After ten task/cancel cycles, check for surviving descendants and sustained memory growth. Track wall time and creator interventions per accepted feature alongside resource use. Set numerical RAM budgets from this first release build.

## R1 · Connect the harness to a playable Roblox game

Use a **two-player salvage game** as the reference project. One room, one collectible, one airlock, and one round are enough to exercise world edits, server logic, client UI, and replication.

1. **Attach to the right place.** Detect Studio, enumerate exposed capabilities, select a place, and verify its identity. Read the live instance tree and map scripts on disk to their Studio objects. Respect a project's existing Rojo or Script Sync setup. For a fresh project, pick one supported sync path first. Display connection and sync failures in the task that needs them. [Studio MCP](https://create.roblox.com/docs/studio/mcp) · [Roblox coding harness guide](https://create.roblox.com/docs/ai/coding-harness) · [Rojo](https://create.roblox.com/docs/projects/rojo)
2. **Translate an idea into milestones.** The lead proposes the player loop, win/loss condition, target inputs, and first playable scope. Save design choices such as shared versus individual rewards. Let creators change direction. Show which tasks and checks a decision invalidates.
3. **Edit scripts and the world together.** Workers can build the round service and HUD in parallel once the interface is defined. Give one task exclusive control of the active Studio place for scene changes or playtesting. Show the script diff alongside added, removed, and changed instances. Checkpoint Studio-owned content before editing. Restore from a matching checkpoint in a test copy and detect intervening manual changes before applying a restore.
4. **Test the integrated revision.** Start a solo session, then a server with two clients. Save scenario setup, player actions, assertions, and client/server logs. Use built-in Studio MCP capabilities where available and a thin Studio plugin for plugin-only functions such as automated multiplayer tests. Verify the revision and sync state before starting each run. [StudioTestService](https://create.roblox.com/docs/reference/engine/classes/StudioTestService) · [Studio testing modes](https://create.roblox.com/docs/studio/testing-modes)
5. **Review and iterate.** Give the creator a launch action, the changed files and objects, and a result for each acceptance check. Save requested screenshots and clips to disk with the run and revision. Return a small artifact reference to agents and load selected frames when needed. Probe installed capture capabilities and add a Windows window-capture fallback only where needed.

**Exit demo:** Both players reach the collectible at the same time. The server awards it once, both HUDs agree, the airlock ends the round, and the next round resets. Tomorrow, change to individual rewards, switch lead provider, and rerun the relevant scenarios. Restore the earlier room and scripts in a test copy. Have a creator play the whole loop and flag anything confusing or dull even if every assertion passed.

## R2–R4 · Roblox-specific capability backlog

Each row delivers an end-to-end creator workflow. Choose R2 rows by the needs of the first real game. Each row has an observable completion condition.

| Release and workflow | What the harness should help creators do | Acceptance example |
|---|---|---|
| R2 · World and level design | Edit terrain, models, pivots, collisions, lighting, spawns, tags, and attributes. Preview spatial changes and test traversal. | Add a second room and verify a player can reach the objective from each spawn. Review and undo the room as one change. |
| R2 · Art, animation, and audio | Import meshes and textures, map rigs and animations, place sounds and VFX, and track asset IDs, ownership, permissions, and package versions. Integrate existing DCC tools when a project uses them. | Replace a placeholder door with an animated asset. Verify scale, collision, sound, and asset availability in the staging experience. [Importer](https://create.roblox.com/docs/studio/importer) · [Packages](https://create.roblox.com/docs/projects/assets/packages) |
| R2 · Gameplay systems | Help build round state, movement, camera, NPC navigation, interactions, inventories, and progression. Index each system's scripts, remotes, scene objects, and tests. | An NPC traverses two rooms, handles an obstructed route, and resets on the next round. |
| R2 · UI, input, and accessibility | Preview HUDs on phone, tablet, and desktop. Exercise touch, mouse/keyboard, and gamepad flows. Check safe areas, text scaling, focus order, audio cues, and localization. | Complete the first round on touch and gamepad. Confirm translated text fits and the objective is understandable with audio off. |
| R2 · Multiplayer and persistence | Validate server-owned state, replication, late join, reconnect, save/load, schema migration, and retry behavior against isolated test data. Add cross-place travel and matchmaking when the game needs them. | Two clients claim one reward. A disconnect and rejoin preserves the correct balance and an older test save migrates successfully. |
| R2 · Performance and QA | Maintain repeatable scenarios. Exercise device and network conditions, compare client/server frame time, memory, and network usage, and attach profiler evidence to regressions. Use physical-device checks for hardware-sensitive signoff. | Compare the same busy room across revisions, identify a frame-time regression, and rerun after the fix. [Performance tools](https://create.roblox.com/docs/performance-optimization/identify) · [Network Simulator](https://create.roblox.com/docs/studio/network-simulator) |
| R3 · Team collaboration | Respect Team Create edits, Git branches, package ownership, and partially managed places. Record who changed which revision and hand off a milestone with its outstanding decisions. | An artist changes the map while an agent fixes a script. Both changes survive integration and a teammate can reproduce the result. |
| R3 · Platform services and commerce | Cover badges, passes, developer products, purchase fulfillment, social features, and moderation or text filtering for creator-built player communication. Track experience-specific IDs and dependencies. | Present a test receipt twice and verify one grant. Verify the staging experience uses its own configuration. |
| R3 · Release and recovery | Assemble release notes, screenshots, metadata, place and package versions, and release checks. Promote a tested candidate to staging, review the destination, then publish with creator approval. Link to Creator Dashboard for steps requiring its UI. | Rehearse a multi-place release and restore the prior place revision. Verify saved player data remains compatible or has a separate recovery plan. [Version history](https://create.roblox.com/docs/projects/version-history) |
| R4 · Analytics, discovery, and live operations | Define onboarding funnels and economy events, review retention and performance, plan seasonal events, and test configuration changes. Associate findings with the release that introduced them. Track store-page assets and experiment hypotheses. | Find the onboarding step where players leave, change that step, and compare eligible cohorts against a preselected metric. [Analytics](https://create.roblox.com/docs/production/analytics/get-started) · [Experiments](https://create.roblox.com/docs/production/experiments) |

## First implementation tickets, in order

- [x] Record idle Windows and Studio baselines. Create a small Git-backed Roblox fixture with a test that fails predictably.
- [x] From Rust, run one Codex task against the fixture and stream structured events. Cancel it and inspect child-process cleanup.
- [x] Repeat the probe with Claude. Document capabilities or authentication differences that affect the adapter.
- [ ] Select the target Studio instance, verify a known script syncs, and run one solo assertion. Record the exact capabilities exposed by the installed build.
- [x] Persist a task, attempt, base revision, and patch. Recover after forcibly closing the harness and accept the change once.
- [x] Put that working loop behind one native project window. Complete H1 before adding a worker scheduler.

The [H1 verification record](docs/h1-verification.md) covers the exit demo, provider probes and Studio capabilities. The Studio experiment verified the built place and solo assertions. Live plugin sync remains to be checked.

At R1, compare the harness with using each provider directly on the same fixture and task sequence. Record time to a playable revision, manual interventions, regressions after a follow-up change, usage, and peak local memory. Use those results and feedback from creators bringing their own projects to choose the first R2 investments.
