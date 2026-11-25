# H2 Windows verification

The Codex lead and workers were exercised with the Windows release build, standalone Codex CLI 0.172.0 and the same signed-in account. Both roles used `gpt-6.2-sol`. All projects used for these runs were disposable Git folders.

## Claude Code and mixed teams

The signed-in Windows Claude Code CLI (2.1.222, Claude Pro) and Codex each took both roles in disposable Git projects. Each request asked one worker to create a one-line greeting and the lead to record its capitalization. The native provider processes ran headlessly. Each pairing produced a worker attempt, an integrated review and the expected file after acceptance. The source project had no greeting before acceptance.

| Lead | Worker | Result |
|---|---|---|
| Claude Sonnet | Claude Sonnet | Accepted |
| Claude Sonnet | GPT-6.2 Sol | Accepted |
| GPT-6.2 Sol | Claude Sonnet | Accepted |
| GPT-6.2 Sol | GPT-6.2 Sol | Accepted |

A fresh Codex chat correctly recalled the accepted Claude checkpoint. Claude identified the color of an attached image, repeated it on a warm follow-up, and recalled it after its process restarted. During a separate live Claude response, steering interrupted the original output and produced the requested replacement. Stopping a second response persisted a cancelled attempt. Studio exported a Claude-designed cube as a one-mesh GLB. The Windows fixture also exercises a stopped Claude worker while its sibling completes, resume without duplicating its sibling, and a conflicting edit with a Claude lead and Codex workers.

## Delegation and continuity

A single release request asked Workshop to implement two Python APIs, add edge-case tests, document usage and verify the result. It did not specify workers or a plan. With automatic delegation on, the real lead assigned the label and duration APIs to two workers. The event stream showed both running at the same time. Both finished on their first attempt. The lead integrated the APIs, updated the README and reached review without a follow-up. Before acceptance the source project still contained only the original README and acceptance script. After one acceptance, all 10 unit tests, the public acceptance script and the examples extracted from the README passed independently on Python 3.8.10. The acceptance script was byte-for-byte unchanged.

The deterministic release scenario also exercised recovery. A read-only reviewer found a misspelling in a worker’s output. On the same user request, Workshop returned that finding to the lead, which assigned a correction and retried the review. The dependent release-note worker received the reviewer's result and completed. Separate cases cover a single-file repair, an unavailable command runner, stopping during a final check and halting after repeated failures.

Two real workers created `left.txt` and `right.txt` in separate project copies. The lead integrated their edits and proposed a decision to keep one word per file, with a reading guide as the next task. Acceptance applied both files together. A fresh Codex chat used the saved handoff, wrote `GUIDE.md` and reached acceptance in 44 seconds. The workers reported 54,038 and 54,115 total provider tokens respectively, including context.

The first delegation exposed a Windows path-length failure opening the worker database. Shortening its storage path let the same lead resume the same assignment keys. The two-file change took 96 seconds from the first recorded event to acceptance, including the repair and one resume instruction. The resumed segment took 42 seconds. Each change required one explicit acceptance.

## Process measurements

The table gives endpoint private working set and private committed memory in MiB. CPU is the percentage of one logical core used across each roughly six-second sample. Workshop and Studio were measured separately. The worker benchmark included the lead, active workers and their installed provider tools, with a requested ten-second wait before each file edit. The Studio measurements used the same empty place, first open and then in play mode.

| Process tree | Interval | Private working set | Private commit | CPU | Codex servers |
|---|---:|---:|---:|---:|---:|
| Workshop open, no chat active | 5.63 s | 6.3 | 7.2 | 0.0% | 0 |
| Workshop, lead and one worker | 5.74 s | 962.3 | 1,650.6 | 72.4% | 2 |
| Workshop, lead and two workers | 5.80 s | 1,431.1 | 2,449.3 | 87.0% | 3 |
| Studio open | 5.82 s | 1,887.5 | 2,738.1 | 32.8% | 0 |
| Studio in play mode | 5.71 s | 2,432.2 | 3,374.5 | 39.4% | 0 |

The native app accepted the two-file feature in 97.4 seconds with parallelism set to one, including both intentional waits. There was one submission, one acceptance and zero corrective messages. In the two-worker run, the right worker emitted no reply before the benchmark’s six-minute completion deadline. After reopening the same chat and issuing one resume instruction, Workshop reused the left worker’s completed edit, retried the right worker and accepted the correct contents of both files. The full run took 548 seconds including the interruption and 137.9 seconds from resume to acceptance. Worker attempt counts were one for left and two for right. Provider response time dominated this sample.

One sample per configuration gives a baseline for later comparisons. CPU covers descendants present at both endpoints and can miss short-lived processes. Memory figures are samples, so the peak can be higher.

Initial regression budgets on this machine are 16 MiB each for idle Workshop working set and commit, 1.25 GiB working set and 2.25 GiB commit with one active worker, and 1.75 GiB working set and 3 GiB commit with two. Compare the same installed provider tool configuration and sample interval when testing later releases.

## Recovery, cancellation and UI

- Eight Rust tests passed, covering dependency validation, paging, interrupted integration, replaying acceptance and preserving creator edits.
- A deterministic Windows stdio provider exercised conflicting edits, stopping one worker while its sibling finished, serializing shared scopes, and halting retries after the same check failed twice. The tests drove the release CLI and real Git repositories.
- With a new project’s default settings, two worker processes started concurrently. A third worker read the first worker’s integrated file and finished while the second worker was held open. A fourth worker waited for both branches, read both results, and produced the final file. One acceptance applied all four edits.
- Windows UI Automation saved parallelism through **Agent settings**, started a two-worker chat, expanded a worker and invoked **Stop worker**. The stopped worker showed **Blocked** and its sibling showed **Finished**.
- Ten task/cancel cycles in one native Workshop window released all six captured descendants each time. Private commit after cancellation ranged from 8.50 to 9.21 MiB and ended 0.71 MiB above the first cycle. A longer run would be needed to characterize long-term growth.

To rerun the offline and deterministic checks on Windows, use a Rust toolchain, Git, Python and the .NET Framework 4 C# compiler.

```powershell
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release
python tests/windows/team_probe.py --cli target/release/workshop-cli.exe
.\tests\windows\cancel_cycles.ps1
```
