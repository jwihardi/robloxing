# Game UI Taste

A portable Agent Skill for Claude Code, Codex and compatible agents. It combines aesthetic discipline with game-specific interaction design. It is a new, standalone adaptation inspired by [taste-skill](https://github.com/Leonxlnx/taste-skill) and [game-ui-ux](https://github.com/parodyband/game-ui-ux), not an upstream GitHub fork. Both upstream projects use the MIT license; their notices are retained in LICENSE. No upstream web starter kits or game art are bundled.

## Install

Extract this folder, keeping its name and contents intact, into the project skill directory. Use one or both depending on your agent:

- Claude Code: `.claude/skills/game-ui-taste/`
- Codex: `.agents/skills/game-ui-taste/`

For example, from the project root on macOS/Linux, after extracting elsewhere:

```sh
mkdir -p .claude/skills .agents/skills
cp -R /path/to/game-ui-taste .claude/skills/
cp -R /path/to/game-ui-taste .agents/skills/
```

On Windows PowerShell:

```powershell
New-Item -ItemType Directory -Force .claude/skills, .agents/skills | Out-Null
Copy-Item -Recurse /path/to/game-ui-taste .claude/skills/
Copy-Item -Recurse /path/to/game-ui-taste .agents/skills/
```

Replace the source path with the extracted location. Restart or reload your agent if it does not discover new skills. The expected final path ends in `game-ui-taste/SKILL.md`. Use only one active overlapping UI skill or explicitly give this one precedence. Do not install it as a web design skill.

## Prompt examples

- Use game-ui-taste in polish mode. Compare these screenshots against Animal Crossing. Preserve the existing font, gloss, studs, assets and composition. Make only high-impact corrections and verify the actual Roblox UI.
- Use game-ui-taste in faithful mode. Recreate the original rhythm game's results screen in the existing Bevy renderer. Match the typography, spacing, prompt legend, transitions and hierarchy; mark unknown measurements as estimates.
- Use game-ui-taste in original mode. Design a cohesive shop and inventory for our factory game. Research reference screens and show the proposed layout and art direction for approval before implementation.
- Use game-ui-taste to audit the current UI. Return a prioritized list of specific changes; do not modify files.

## Finish gate

The skill instructs the agent to capture and inspect actual screenshots, document its findings and test relevant interactions. The included Python 3 script verifies the review record and whether referenced evidence files exist. It does not inspect pixels, capture engine screenshots, enforce agent hooks or run game tests itself.

```sh
python /path/to/game-ui-taste/scripts/review_gate.py .game-ui-taste/review.json
```

Exit 0 = structurally complete review, 1 = unresolved checks or evidence, 2 = invalid record. Engine-specific captures are intentionally left to project tools. Offline/headless environments may not be able to verify the visual result; report that limitation rather than claiming a pass.

## Structure

- `SKILL.md`: core workflow, style modes, anti-slop rules and QA.
- `references/`: visual craft, screen patterns, runtime/input, review protocol.
- `assets/review.template.json`: review record starter.
- `scripts/review_gate.py`: dependency-free evidence validator.
