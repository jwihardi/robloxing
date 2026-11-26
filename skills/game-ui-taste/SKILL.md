---
name: game-ui-taste
description: >-
  Design, implement, audit, restyle and polish in-game interfaces with authored art direction and high visual craft. Use for HUDs, main menus, inventories, shops, dialogue, character selection, results, settings, controller navigation, mobile game UI, Roblox interfaces, and faithful remakes. Activate especially when a game UI looks generic, web-like, AI-generated, cluttered, cheap, inconsistent, or unlike a supplied reference. Works in the existing game engine. Not for marketing sites, generic SaaS, or unrelated editor tooling.
license: MIT
metadata:
  version: '1.0.0'
---
# Game UI Taste

Make the interface feel authored for this game, not assembled from a UI kit. The player should recognize the game's identity from an unbranded screenshot. A good interface is both a usable instrument and part of the art direction. Do not equate quality with minimalism, maximalism, expensive effects, or any one style.

## 0. Non-negotiable rules

1. **Preserve intent.** Explicit user requirements, approved style, supplied references, game rules and engine constraints outrank all defaults here. Never silently change typography, proportions, gloss, studs, texture, ornamental borders, or other requested characteristics. A polish request is not authorization for a redesign.
2. **No generic layout by reflex.** Do not begin with a website hero, SaaS card grid, pill tabs, dashboard sidebar, or a centered modal over every game. Begin with what the player needs to see and do. A card game, diegetic terminal or deliberately web-like game may legitimately use these devices.
3. **Reference before invention.** Inspect actual game screenshots and existing assets before setting style. Do not claim to have inspected a reference that you have not opened. References are evidence, not a source of production assets unless permitted.
4. **Make the hierarchy visible.** Establish an unmistakable focal element and reading order. Use scale, placement, silhouette, contrast and rhythm before adding containers, text or decoration.
5. **No fake substance.** No redundant captions, eyebrow labels, marketing copy, invented game mechanics, dummy prices, fake rewards, invented stats, fake loading progress or success states. Keep real instructions, descriptions, worldbuilding, subtitles and accessibility cues when useful.
6. **Use the game's visual vocabulary.** Reuse appropriate fonts, sprite assets, materials, icons and interaction conventions. No generic icon set or system font merely because it is easy. No imitation of a different game's signature treatment unless requested.
7. **Ship in the requested runtime.** Do not substitute an HTML mockup for Roblox, Unity, Godot, Unreal or Bevy implementation. Preserve architecture, networking, inputs, localization and authoritative state.
8. **Verify at play scale.** Inspect a real runtime capture whenever possible. A static mockup is not proof of interaction, performance, or responsive layout. Never fabricate QA or hide failures.

## 1. Choose scope and design mode

Read project instructions, existing UI code/scenes/prefabs, asset folders, tokens, fonts and navigation. Inspect the current screen if one exists. Identify runtime, input modes, target devices, screen family, constraints, and whether the user wants critique, design, implementation or polish. Do not expand the task unnecessarily.

- **Faithful:** match an existing game's or supplied reference's grammar. Extract and preserve its composition, typography, spacing and graphic vocabulary; list the intentional differences. Modernization does not imply generic contemporary styling.
- **Polish:** keep the composition, mechanics, approved assets and stylistic character; target the specific visual defects. Prefer surgical fixes over replacing the system.
- **Original:** derive an identity from the game world, audience, genre and pacing. Compare genuinely different compositions when uncertain; settle one direction before implementation.

Use these design axes as a vocabulary, not random numbers. Set each deliberately from references; inherit established values for faithful/polish work.

| Axis | 1 | 5 | 10 |
|---|---|---|---|
| Composition | strict grid | structured with asymmetry | expressive / spatial |
| Information density | sparse | balanced | information-rich |
| Surface treatment | flat | tactile layering | elaborate material/ornament |
| Typography expression | utilitarian | distinct roles | highly expressive display type |
| Motion expression | restrained | responsive | theatrical, event-driven |
| World integration | overlay | thematic | diegetic / spatial |

Do not default to all 5, all low, or all high. These are descriptive, not a score to maximize. Strong identity can be flat or ornate. Use `references/art-direction.md` for detailed examples. For a new full screen or system, record a small plan; ask approval before implementation only if the user requests an approval step or a major irreversible design choice cannot be inferred. For a minor patch, proceed directly.

## 2. Reference research and visual diagnosis

For substantial original work, seek 3–5 closely relevant reference screens (same screen function, genre, platform or art direction). For faithful work prioritize the actual target game and existing project art. For minor polish one supplied screenshot or baseline may be enough. If tools are offline, use available local references; state gaps rather than inventing them.

Look at Game UI Database (https://www.gameuidatabase.com/), Interface In Game (https://interfaceingame.com/), official game footage and project assets when available. Search for specific screens, not vague moodboards. Inspect images at their original proportions; for motion use footage when available. Do not scrape around access restrictions. Keep source links and notes separate from shipped UI; do not import reference screenshots or third-party art without permission.

For each relevant reference, identify what it actually does: focal point, content arrangement, typeface roles and sizes relative to canvas, alignment lines, whitespace rhythm, shape and corner treatment, layering and material, icon silhouette, state colors, prompt placement, transitions, and behavior against the world. Record what to borrow as principles and what NOT to borrow. If matching a reference, distinguish measured values from estimated ones.

Compare the current screenshot to the target at equal display scale. Identify the 3–5 highest-impact differences. Diagnose causes (e.g. same visual weight everywhere, mixed icon styles, weak type hierarchy, wrong panel density, incoherent light direction) rather than vague adjectives. Do not remove desired stylization to solve a different defect.

## 3. Establish player task and composition

Write a compact decision contract outside the interface: player goal, information needed now, primary and secondary actions, source of truth, inputs, risks/recovery, emotional tone. Identify the dominant content: world, map, character, item, board, list, score or dialogue. Choose one coherent layout around it.

- Action HUD: preserve the play field, predictable scan positions, reticle and telegraphs. Use event emphasis sparingly; suppress irrelevant chrome.
- Menu: establish the game's visual identity with a readable action hierarchy and focused selection. Avoid treating main menus as marketing pages.
- Inventory/shop/loadout: prioritize comparisons and item artwork. Group related information with alignment, spacing or deliberate slots; keep price and consequence at point of action.
- Results/progression: one clear moment of recognition, then meaningful performance and next actions. Avoid a grid of identical metric cards unless the game explicitly needs it.
- Settings: familiar controls and predictable layout; clarity may outweigh expressive decoration. Preserve apply/revert and confirmation where needed.

Read `references/screen-patterns.md` for additional patterns. Draw a structural sketch or use a short coordinate/anchor description if it helps. Select a focal element, major alignments, scale relationships and intended negative space; do not just list components. Do not create a decorative header for every grouping. Keep high-frequency information stable. Reserve spectacle for high-value moments.

## 4. Build a coherent art system

Before implementation, derive a compact style contract with real values from the project: reference resolution, safe-area policy, typography roles, palette roles, spacing steps, border/stroke treatments, shape language, icon style, materials, shadow direction, interaction states, motion timings and audio roles. Record source or rationale for unusual decisions. Prefer the project's existing tokens over inventing new ones. Avoid universal pixel values; establish consistent ratios and optical alignment. A deliberate exception can be valuable.

Use `references/art-direction.md` for type, layout, color, material, asset and icon checks. Use actual game art, rendered item imagery, atlas sprites, nine-sliced panels or vector geometry where appropriate. If existing assets are insufficient, specify the missing assets and create only what tools/permissions allow. A temporary placeholder must be identified and may not be described as final. No new network dependencies or broad asset replacement without a reason.

Apply the visual grammar to ordinary, selected, pressed, focused, unavailable and exceptional states, not just the hero screenshot. Treat these as distinct meanings. Selection must be readable without relying on hover or color alone. Check against actual gameplay or representative backgrounds. Preserve correct glyphs and input prompts.

## 5. Anti-slop pass (context-sensitive)

Review every occurrence of these failure patterns. Remove or revise only when unsupported by the art direction or task:

- Floating card soup; repeated identical containers enclosing every number, label and icon; unnecessary nested frames; excessive blank padding.
- All headings and buttons styled as equally important; giant headings or microtype without a role; arbitrary font swapping; random typefaces, weights and casing.
- Random gradients, neon, glass, bloom, bevels, shadows, outlines, pill corners or decorative stickers used as substitutes for hierarchy. Authored glossy, studded, textured, chunky or ornate designs are valid and must be preserved when intentional.
- Generic thin-line icons mixed with chunky game art; inconsistent icon scale, perspective, line weights, palette or lighting; ambiguous symbols with no label when needed.
- Dashboard-like KPI blocks, eyebrow headings, redundant instructions, decorative captions, text repeating visible state; fictitious state to fill empty slots.
- Equidistant elements without meaningful grouping; overly distributed controller help bars; arbitrary asymmetry; symmetrical centering with no reason; inconsistent margins and optical alignment.
- Uniform corner radii across unrelated surfaces; tiny inset borders stacked indiscriminately; inconsistent light direction; oversized soft shadows on every element.
- Permanent animation on ordinary elements, delayed routine inputs, repeated celebrations or effects obscuring critical information. No gratuitous motion for the sake of looking polished.
- Using a generic web template, flat modern style or stock assets to overwrite an established visual language.

For each proposed change state the observed issue, the specific change, and what should remain. If a requested style uses a listed device, retain it and refine the execution instead of banning it. Never replace missing content with filler.

## Mandatory Reference Acquisition

Web search is a discovery mechanism, not visual evidence.

When a search returns marketplace listings, game galleries, videos, or articles containing UI previews, the agent MUST attempt to acquire and inspect the actual images.

Acquisition fallback order:

1. Download original image URLs.
2. Extract gallery images and responsive image sources from HTML.
3. Use browser automation to render JavaScript and lazy-loaded content.
4. Capture element-level browser screenshots.
5. Extract relevant frames from accessible gameplay footage.
6. Request user-provided references only after available automated methods have failed.

Store acquired references locally alongside their source URLs. Open each reference with a visual inspection tool before drawing design conclusions.

Do not claim that references were reviewed merely because their descriptions were read.

## Interpreter Discovery

Before declaring Python unavailable, test:

- `py`
- `python`
- `python3`
- Project-local virtual environments
- Installed Python executable paths

On Windows, distinguish Microsoft Store execution aliases from functioning Python installations.

Use whichever interpreter successfully executes the validation script. Do not assume that failure of the `python` command means Python is unavailable.

## 6. Implement and verify behavior

Read `references/runtime-and-input.md` before substantial implementation. Use engine-native layout, text, sprites, safe areas, focus navigation and animation. Keep style parameters reusable, separate presentation from game state, and avoid unnecessary per-frame rebuilds. Do not replace valid infrastructure or add dependencies just to satisfy this skill.

Define all affected states and transitions: entry, idle, focus, hover where applicable, press, selection, disabled, pending, error, empty/full, success, back/cancel, interrupted actions, device change. Preserve focus identity across dynamic lists. Use real state and valid game actions; never indicate confirmed success before the authoritative result. Keep operations possible with supported inputs, including no-hover devices.

Motion should have a function: acknowledgment, spatial continuity, warning or reward. Press feedback is immediate; transitions are interruptible; do not hold input hostage to ornamental animations. Define focus vs selection, prompt glyph switching, audio trigger policy and reduced motion behavior. For multiplayer, preserve client/server validation.

## 7. Screenshot-driven finish gate

For a meaningful implementation, create `.game-ui-taste/` in the target project (or an agreed scratch location). Copy `assets/review.template.json` there as `review.json`. Before major edits capture baseline screenshots when possible. Set the intended scope, reference notes, applicable checks, style contract, and acceptance criteria. Use the exact runtime or an engine capture/test facility; do not make an unrelated browser recreation to pass the gate. Read `references/review.md` for the matrix and workflow.

1. Run the project or the relevant scene; capture after screenshots at the reference canvas plus representative supported viewports. For polish, capture the same state/camera before and after; for a new screen document baseline unavailability. Use filenames that encode screen, state and viewport.
2. Open the captures yourself with an image-reading tool. Compare the reference, baseline and result at actual size. Inspect silhouette, text, alignment, density, surface coherence and state clarity. Check normal and focused/selected states, busy background, long text or other relevant stress conditions. Do not declare victory from code or file existence alone.
3. Fix the highest-impact defects, recapture and re-inspect. For substantial work do at least one critique/refinement cycle, or document specific evidence that the first capture already meets the target. Do not make arbitrary changes to satisfy a quota.
4. Exercise actual inputs, focus/back, game-state correctness, scaling/safe area, accessibility and relevant runtime performance. Record failures, evidence and untested cases. For design-only deliverables use scenario traces and label runtime checks untested. For a small patch scope the matrix to the changed behavior.
5. Run `python <skill>/scripts/review_gate.py .game-ui-taste/review.json` if Python is available. The script checks the review record and evidence paths; it cannot judge visual quality or independently verify tests. A pass never substitutes for human/agent screenshot inspection. Missing runtime tools = unverified, not passed. Do not bypass the gate by falsifying evidence or excluding applicable checks. If blocked, deliver what is useful and explicitly report the blocker.

Critical defects include clipped essential information, hidden or misleading state, unreadable action prompts, inaccessible critical paths, focus traps, incorrect purchases or previews, and unintended identity changes. Resolve before calling work ready; otherwise report incomplete.

## 8. Handoff

Deliver the requested artifact first. Briefly identify actual changes, preserved design elements, reference sources, runtime checks actually performed, and unresolved issues. Include before/after capture paths for substantial visual work when available. Do not inflate a mockup into a tested implementation. Keep internal plans out of the player-facing interface. Stop at the requested scope.
