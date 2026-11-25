---
version: alpha
name: Workshop
description: A native dark workspace for project conversations, agents and assets.
colors:
  rail: "#101217"
  sidebar: "#14161b"
  canvas: "#191c22"
  surface: "#252a33"
  selected: "#303643"
  line: "#393e49"
  text: "#f0f1f5"
  muted: "#aeb5c3"
  accent: "#b9c9ff"
typography:
  sans: { fontFamily: "Segoe UI" }
  display: { fontFamily: "Segoe UI" }
rounded:
  composer: "23px"
  action: "9px"
spacing:
  rail: "76px"
  sidebar: "218px"
components:
  composer: {}
  action: {}
---

# Workshop

The far-left rail holds the destinations that apply across the project. A second column holds whatever you can browse in the current destination, such as chats, Studio runs or settings sections. At widths below 940 logical pixels the second column becomes a drawer. The reading area, composer and output preview get that space back. The Sidebar control opens or closes the column at either size. This arrangement draws on the separation of navigation and sessions in KiroCrew and Linear’s sidebar hierarchy. [KiroCrew session view](https://github.com/kirodotdev/KiroCrew/blob/main/.github/screenshots/25249/composer-meter-after-model-switch-dark.png) · [Linear UI refresh](https://linear.app/changelog/2026-03-12-ui-refresh)

## Conversations

Keep the composer at the bottom in a new chat and throughout a conversation. The two shortcuts on the welcome screen fill in editable drafts. The project navigator searches saved chat prompts across history pages. The reading column tops out at 760 logical pixels. Right-align the user's messages on a raised surface and place selectable assistant replies directly on the canvas.

Keep the model and stepped effort selector beside Send. Attachments and subagent settings belong on the left of the composer. Show dispatch receipts where the work happened. Activity in the chat header opens the session state, workers and project handoff. Open a worker to read its assignment, criteria, dependencies, scope and latest activity. Keep the review action in the transcript when a change is ready. Outputs opens screenshots for this chat and links to the album. The contextual session panel takes inspiration from Claude Code’s desktop layout. [Claude Code desktop](https://code.claude.com/docs/en/desktop)

## Studio, settings and album

Studio pairs a task-specific editor and preview with a searchable run history. The result actions stay next to the selected output. Run settings apply to one task. Global task defaults live in Settings alongside providers, plugins, skills and appearance. Keep manual installation fields behind **Install from source**. The album uses a responsive thumbnail grid with the project and capture date under each image.

## Appearance

Each preset has a miniature conversation thumbnail and the active theme has a checkmark. Selecting a row applies it immediately. The custom theme editor previews valid color changes inside a sample conversation so the surrounding editor stays legible. Saving applies the colors to the workspace. The additional presets adapt the source palettes to Workshop’s surfaces and text roles. [Nord](https://www.nordtheme.com/docs/colors-and-palettes/) · [Catppuccin](https://catppuccin.com/palette/) · [Solarized](https://ethanschoonover.com/solarized/)

## Components

The files in `themes/` own the preset colors. Their CSS custom properties map directly to `ThemeColors` in `ui/common.slint`, which supplies the runtime palette to app components. The values above document Workshop Dark. The canvas brightness also selects the light or dark treatment for standard Slint widgets. Segoe UI carries both headings and conversation text. In Workshop Dark the rail is the darkest surface, the workspace is brighter, and selected actions and focused controls use a pale periwinkle accent. Capacity meters pair their green-to-red scale with numeric values. Use a labeled action for every icon-only control and keep advanced information available by keyboard.

| Capability | Canonical owner | Source of truth | Allowed variants | Verification |
|---|---|---|---|---|
| Form | Composer, StudioPage and TaskEditor | Draft, selected run and saved task policy | Chat, task briefs, global and per-run settings | Windows picker suite exercises draft editing, attachments and task overrides |
| Select/Listbox | InferencePicker, EffortTrack and history ListView | Provider catalog, project store, Studio history | Chat and worker policies, chat navigation and Studio run selection | Picker suite exercises focus, model switching, effort and library search |
| Scrollbar | Slint ScrollView, ListView and TextEdit | Pane dimensions and content size | Transcript, activity, catalog, settings and run details | Long content can scroll independently of the composer |
| CRUD | Store and Desk | SQLite attempt state and saved patch | New chat, stop, retry, accept and discard | Recovery tests and private desktop cancellation suite |
