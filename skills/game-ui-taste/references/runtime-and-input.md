# Runtime and input implementation

Inspect the actual project and version before choosing APIs. These are implementation directions, not promises about a specific engine version.

| Runtime | Inspect and prefer |
|---|---|
| Roblox | ScreenGui inset/safe-area settings, UDim2 anchors and scale/offset, UIListLayout/UIGridLayout and constraints, UIScale where appropriate, sprite ImageLabels/Buttons and slice settings, GuiService selection, existing input action system, TweenService and project audio. Test device emulator and actual play when available. Validate economic/gameplay actions on server. |
| Unity | Existing uGUI or UI Toolkit stack, CanvasScaler or panel scaling, anchors/layout groups, sprite slicing, TextMeshPro or existing text system, EventSystem/input actions, Animator/tweens and existing focus/navigation. Do not migrate UI systems unasked. |
| Unreal | Existing UMG/Slate/CommonUI architecture, DPI curve, Safe Zone, anchors, brush margins, focus/input routing, invalidation and widget animations. Avoid needless bindings that update every frame. |
| Godot | Existing Control tree, containers/anchors, Theme overrides, nine-patch resources, focus neighbors, InputMap, stretch policy and tweens. |
| Bevy | Existing UI or custom render pipeline, layout and scale settings, sprite/text assets, interaction/focus and input actions, state/event scheduling; preserve ordering and rendering layers. Do not assume a particular Bevy API version. |
| Custom/browser engine | Native project UI/rendering and input loop, asset loading, pixel ratio, layout, focus, and accessibility where available. Browser UI is valid for a browser game, not a substitute for another engine. |

## Scaling

Define reference resolution, anchor rules, min/max scale, safe-area behavior, aspect ratio strategy and text reflow. Distinguish decorative elements that may crop from critical state that must remain visible. Test narrow/wide and low/high resolution supported by the project. Do not stretch artwork non-uniformly unless designed for it. Verify long localized text, large numbers, font fallback and dynamic content. Touch hit targets must work at physical display scale, not just screenshot pixels.

## Input and feedback

Focus is navigational; selection is a value; active/equipped is committed state. Do not merge these semantics. Set first focus deterministically, restore focus on back, prevent modal input leaking to gameplay, and preserve stable IDs when lists change. Ensure supported controller navigation does not need hover or dragging. Update glyphs on actual device/remap changes; avoid switching on incidental pointer jitter if project input policy forbids it. Touch does not have hover. Keyboard and controller shortcuts need visible discoverability when relevant.

Acknowledge input immediately, separate pending from confirmed state, prevent accidental duplicate commitments, and support cancel/recovery where allowed. Treat network failure as real. UI should reflect authoritative state and use validated actions. Do not fabricate offline data for a shipped UI.

Define sound categories (move, confirm, cancel, blocked, reward, warning) from existing sound language. Trigger once per meaningful transition; avoid noisy repeated hover or focus events. Honor UI volume, mute and accessibility options. Audio is not the sole indicator of a critical state. Honor reduced motion or equivalent project settings; maintain selection and feedback without animation. Measure performance on target hardware when available; note any untested constraints. Avoid per-frame allocation, unnecessary layout invalidation, overdraw and excessive effects.
