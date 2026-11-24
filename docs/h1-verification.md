# Windows H1 verification

Recorded November 23, 2026 on Windows 11 x64. The live demos used disposable Git projects copied from `fixtures/salvage`. The repository fixture retains its original failing test.

## Exit demo

| Exercise | Observed result |
|---|---|
| Fresh project | The original module failed the first assertion. A real Codex turn changed the module in its isolated copy. The app-server acknowledged a steering message and interrupted the turn after the edit. |
| Cancel and resume | All 24 process IDs in the observed provider subtree had exited one second after cancellation. A new harness process resumed the recorded provider thread. The supplied Luau test printed `Salvage checks passed`. Two CLI accept calls succeeded. |
| Pre-existing creator edit | A creator comment was appended to the same module before assigning the task. The harness process was forcibly terminated immediately after the agent wrote the fix. |
| Desktop recovery | The reopened window showed **Interrupted**, the exact saved diff and the acceptance controls. **Run Luau check** passed. **Accept change** updated the project. The same-file comment survived. Repeating acceptance in the CLI succeeded, and the test passed in the creator's folder. |
| Recovery regressions | Offline Rust tests cover a crash during application, a concurrent project open, changed destination contents, corrupt patch bytes, a binary addition, separate staged and unstaged versions, a pre-existing unborn Git index and one durable acceptance decision. |

## Provider boundary

**Codex.** The app-server uses newline-delimited JSON over stdio. The installed 0.168.0 CLI returned a successful ChatGPT sign-in status. A nested launch from the packaged desktop agent inherited a registered-Core request and its shell reported that the child had no package identity. The end-to-end runs used the standalone 0.172.0 Windows CLI in an ordinary process environment, with `workspace-write` and the user's existing provider login. The explicit model selection was `gpt-6.2-astra`. Thread start, steering, interruption, resume and turn completion were all observed. Full local events are in each attempt's `events.jsonl`. [Codex app-server](https://developers.openai.com/codex/app-server/)

**Claude Code.** Version 2.1.222 accepted the programmatic print, stream-JSON, model and resume flags. The `sonnet` selection resolved to `claude-sonnet-5` in its init event. Both the initial and resumed task returned an authentication error, and `claude auth status` reported signed out. Timed cancellation exited successfully. A successful authenticated edit and resume remain unverified for Claude. H1 uses Codex for the production adapter. The probe invokes the user's installed Claude binary and delegates credentials and billing to the provider. [Programmatic runs](https://code.claude.com/docs/en/headless) · [Credential rules](https://code.claude.com/docs/en/legal-and-compliance)

To repeat the Claude probe, copy the fixture to a disposable folder and run the example there. Its stdout is the original provider event stream. Use the returned session ID with `--resume` or add `--cancel-after 1` to check termination.

```powershell
cargo run --locked --no-default-features --example claude_probe -- --copy $project --model sonnet "Fix the salvage module test."
```

## Roblox Studio experiment

Rojo 7.7.1 built the fixture into a local place. Studio 0.748.0.7481526 exposed that place by name and instance ID. The module under `ReplicatedStorage.Salvage` matched disk, and a solo server loaded it. A second build from the accepted project included both the new calculation and the creator comment. On its solo server, `remaining(5, 2) == 3` and `remaining(2, 4) == 0` both passed. Both play sessions were stopped. This verified the disk-to-place build path. Continuous Rojo plugin sync is a separate R1 integration.

The installed MCP proxy advertised these operations. The experiment exercised discovery, state, script read, Luau execution and play controls.

| Group | Exposed operations |
|---|---|
| Session and inspection | `list_roblox_studios`, `get_studio_state`, `search_game_tree`, `inspect_instance`, `script_search`, `script_read`, `script_grep` |
| Editing and play | `multi_edit`, `execute_luau`, `start_stop_play`, `get_console_output`, `character_navigation`, `user_keyboard_input`, `user_mouse_input`, `screen_capture` |
| Assets | `search_asset`, `insert_asset`, `upload_image`, `store_image`, `generate_avatar`, `auto_setup_avatar`, `generate_animation`, `generate_material`, `generate_mesh`, `generate_procedural_model`, `generate_scene`, `generate_texture`, `segment_mesh`, `wait_job_finished` |
| Other | `http_get`, `open_cloud_invoke`, `skill`, `list_skills`, `create_skill`, `edit_skill`, `subagent` |

## Local baseline

Five-second observations on the developer's normal Windows desktop. Process memory comes from Windows `WorkingSetPrivate` and `PrivateBytes` counters. Process CPU is the change in process CPU seconds divided by elapsed seconds, with one fully occupied core equal to 100%. The host CPU counter uses all logical CPUs.

| Observation | Private working set | Private bytes | CPU |
|---|---:|---:|---:|
| Host, Workshop and Studio closed | 16,777 MiB available system memory | — | 19.7% system-wide average |
| Workshop debug build, accepted task open, provider stopped | 7.0 MiB | 8.0 MiB | 1.6% of one core |
| Studio, reviewed place open in edit mode after stopping play | 2,096 MiB | 3,085 MiB | 49.5% of one core |

These are single-machine observations with background desktop activity. The comparable release-build worker measurements belong to H2.
