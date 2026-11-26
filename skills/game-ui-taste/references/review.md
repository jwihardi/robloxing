# Review protocol

A finish gate is useful only when its evidence is honest. The included script validates a record, not the art or runtime. It never certifies a game. Use the project’s real tests, capture facilities and image inspection.

## Process

1. Choose scope: `patch`, `new-screen`, `redesign`, `critique`, or `specification`. Copy `assets/review.template.json` into a working review directory. Set paths relative to that JSON file. Document target, constraints, references and acceptance criteria. For implementation, record the baseline if available; use comparable screen state and camera for before/after.
2. Capture after at target size and other relevant aspect ratios. Capture at least the default and a representative interaction state for substantial work; add gameplay background and stress captures when relevant. For a patch one before/after pair may suffice. Open actual images and judge them, not just metadata.
3. Critique using the checks below. Prioritize blocking operation and truth, then high-impact composition/style differences. Make a targeted fix and compare again. Document what changed and which requested qualities were preserved. A deliberate unchanged item is not a defect.
4. Record evidence per check. Use `pass`, `fail`, `untested`, `na` (not applicable). A `pass` requires an evidence note describing the actual observation; visual inspection requires a capture for implementation. For `na`, give a scope-specific reason. For `untested` or `fail`, describe the gap. Evidence paths refer to actual local files, never invented names.
5. Run the gate. Exit 0 means no unresolved entries in that review, not that the art is objectively excellent. Exit 1 means incomplete or failed; exit 2 means malformed record. Do not turn `untested` into `na` to make a review pass. Design-only work may legitimately have runtime checks `na` with a scope reason; describe it as design-only, not a tested build.

## Check matrix

| Check ID | What to inspect or exercise |
|---|---|
| identity | reference/style fidelity, intentional choices, protected typography/materials |
| hierarchy | focal point, reading order, relevant content groupings, world visibility |
| craft | typography, alignment, iconography, surface and spacing coherence |
| copy | no unnecessary labels/filler; functional text and real data intact |
| state-truth | authoritative values, previews, pending/success/error, no invented state |
| interaction | first focus, selection, primary actions, back/cancel, modal recovery |
| scaling | target viewport, safe area, relevant other ratios and long text |
| accessibility | legibility, non-color cues, supported inputs, reduced motion where relevant |
| stress | dynamic list, changed data, busy backgrounds, rapid input and interruptions as relevant |
| performance | project build/runtime cost, frame or allocation impact where measurable |

For critique and specifications, inspect supplied screenshots or design artifacts and trace scenarios. Do not claim actual input/runtime checks. For patch work, only checks truly unaffected by the change may be `na`; justify each. If runtime is unavailable for a changed implementation, mark affected checks `untested`, not `na`.

## Suggested review note

- Observed: selected button lacks visible focus against pale background.
- Change: retain existing glossy button shape/font, add contrasting focus edge using the existing selected-state color and shape cue; compact the glyph/action grouping.
- Preserve: all studs, typography, panel positions, action semantics.
- Evidence: opened after capture at intended resolution; exercised controller entry, focus movement, confirm and cancel; no focus trap observed in this flow.
- Limitation: touch and physical television testing unavailable.
