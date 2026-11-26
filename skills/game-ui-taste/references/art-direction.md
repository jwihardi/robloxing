# Art direction and craft

Use this when creating or materially restyling a screen. For a faithful remake, measure the original and treat the following as diagnostic tools rather than stylistic mandates.

## Extract the visual language

Record the canvas size and UI scale; screenshot crops can mislead. Compare normalized positions (x/W, y/H), element heights relative to canvas, alignment axes and safe margins. Identify recurring units without forcing every element into an arbitrary grid. Separate the stable grammar from the screen-specific composition. Record exact asset filenames and fonts if known; do not guess them as fact.

| Style family | Useful observations | Common bad imitation |
|---|---|---|
| Nintendo / cozy | clear chunky silhouettes, friendly type, tactile surfaces, balanced negative space, playful accents with readable action hierarchy | pastel pill soup, oversized white outlines, every element bouncing |
| Rhythm / music | strong typography, beat-aligned emphasis, compact controller legend, readable timing/score in motion | unrelated neon dashboards, thick chrome around timing-critical lanes |
| Retro / pixel | grid-aligned sprites, constrained palette, sharp sampling, intentional limited type sizes | mixed pixel scales, fractional blurry positioning, random pixel font |
| Industrial / sci-fi | modular geometry, restrained semantic indicators, readable instrumentation, coherent panel construction | universal cyan glow, random diagonal cuts, decorative unreadable microtext |
| Fantasy / ornate | material motifs and silhouette, crafted ornaments, expressive titles, quiet supporting text | generic gold frames on everything, illegible script body text |
| Minimal / cinematic | large compositional relationships, clean type, controlled contrast, whitespace with purpose | generic modern web font and giant empty panels |

These are not templates or permission to clone. Use the actual reference, not the stereotype.

## Typography

Use existing approved fonts; inspect font asset licensing and glyph support. Separate title/display, action, body, numeric and caption roles. Pick the smallest number of meaningful roles, not a universal two-font rule. Establish font sizes as ratios at the reference resolution and verify at the target viewing distance. Use tabular figures for frequently changing aligned counters if available and consistent with the style. Preserve the source's casing, width, tracking, italic/condensed treatment and line height where matching. Do not use letter spacing to disguise wrong font choice. Check ascenders, descenders, baseline alignment, text stroke and shadow on actual backgrounds. Never globally replace fonts during polish without user direction or a clear defect.

## Composition and spacing

Identify the main focal point, then subordinate elements. Use tight grouping within a semantic unit, more space between groups. Avoid equally spacing every object across the screen. Align related text baselines and icon optical centers rather than blindly using bounding-box centers. Consistent insets and edges are more important than mechanical equal spacing. Compact input legends should group glyph and action; use the original anchor and ordering if available. Keep controls visually stable when values change. Empty space should protect focus or make hierarchy, not waste a controller travel path.

## Materials and color

Choose colors by role: base, text, supporting, accent, selected, disabled, success, warning, danger. Distinguish rarity, teams and resource colors where gameplay requires them; do not conflate them with interaction states. Test against bright, dark, busy and color-vision stress backgrounds. Use outlines, backing, shape or redundant cues where text and state must stay readable. Preserve an approved palette unless the user asks to change it.

If the style is glossy, establish one light direction, highlight placement, edge bevel and shadow softness. Use a consistent material family and scale. Stud patterns should be aligned, appropriately spaced, subordinate to text, and not randomly stretched. If the style is flat, do not add gradients or depth simply for polish. If ornate, reserve dense ornament for framing or important events, not every control. A nine-slice should retain corners and borders at multiple sizes; inspect actual sprite import and filtering.

## Assets and iconography

Prefer existing licensed project assets; inspect atlases, sprites, font files and rendering settings first. Use a consistent icon family: silhouette, stroke, fill, perspective, light direction, palette and intended display size. Use image generation or drawing tools when authorized and available for missing bespoke art, not as a substitute for functional controls or editable layout. Separate decorative art from labels/state. Maintain transparent edges, sprite padding and appropriate filtering; test at actual size. Keep control glyphs faithful to the input device and remapping system. Do not use emoji or arbitrary icon libraries as default final game art. For original assets record dimensions, nine-slice margins, alpha mode, license/source and intended state variants.

## Style contract example (illustrative, not defaults)

A tactile toy-building game: rounded chunky controls, shared top-left light, restrained clear-coat highlight, low-contrast stud motif on panel backs, existing playful display font retained, readable action font retained, saturated accent only for focus and primary action, compact grouped controller legend. Set actual dimensions from the project, and choose timing by interaction cadence. Keep glossy surfaces and studs while removing duplicate outlines, oversized shadows and redundant text.
