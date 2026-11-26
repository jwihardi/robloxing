---
version: alpha
name: Workshop
description: A native dark workspace for project conversations, agents and assets.
colors:
  rail: "#101317"
  sidebar: "#13181d"
  canvas: "#171c22"
  surface: "#222a32"
  selected: "#2a363d"
  line: "#35414a"
  text: "#edf2f4"
  muted: "#a7b6c1"
  accent: "#86e0c0"
  symbol: "#a7bdff"
  keyword: "#d4aff2"
typography:
  sans: { fontFamily: "Segoe UI" }
  display: { fontFamily: "Segoe UI" }
  mono: { fontFamily: "Consolas" }
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

The far-left rail holds the destinations that apply across the project. A second column holds whatever you can browse in the current destination, such as chats, saved assets, reports or settings sections. At widths below 940 logical pixels the second column becomes a drawer. The reading area, composer and output preview get that space back. The Sidebar control opens or closes the column at either size. This arrangement draws on the separation of navigation and sessions in KiroCrew and Linear’s sidebar hierarchy. [KiroCrew session view](https://github.com/kirodotdev/KiroCrew/blob/main/.github/screenshots/25249/composer-meter-after-model-switch-dark.png) · [Linear UI refresh](https://linear.app/changelog/2026-03-12-ui-refresh)

## Conversations

Keep the composer at the bottom in a new chat and throughout a conversation. The two shortcuts on the welcome screen fill in editable drafts. The project navigator searches saved chat prompts across history pages. The reading column tops out at 760 logical pixels. Right-align the user's messages on a raised surface and place selectable assistant replies directly on the canvas.

Keep the model and stepped effort selector beside Send. Attachments and subagent settings belong on the left of the composer. Show dispatch receipts where the work happened. Activity in the chat header opens the session state, workers and project handoff. Open a worker to read its assignment, criteria, dependencies, scope and latest activity. Keep the review action in the transcript when a change is ready. Outputs lists the media from this chat and links to the screenshot album. The contextual session panel takes inspiration from Claude Code’s desktop layout. [Claude Code desktop](https://code.claude.com/docs/en/desktop)

A chat goal sits above its conversation as one compact row with the objective and its current state. Selecting the row opens the editor in the composer area. Editing an existing goal preserves the unsent message. The same editor is available from **+** and `/goal`.

Sketches open inline above the composer from **Add to message**. **Annotate** opens a returned image on the same pad and keeps its aspect ratio. Keep the caption editable while the pad is open. The paper and pen colors are fixed across themes, with a white pen for dark images. The toolbar and chat bubble use the current theme. A collapsed sketch stays in its chat draft, and a sent sketch can expand inside its original message.

During a run, Enter steers the lead. The arrow beside Send opens delivery options. Pending follow-ups sit above the composer in a short scrollable list, each with its own remove action. Selecting **Send direction** on an expanded worker addresses the same composer to that worker. The recipient stays visible until the direction is sent or the user returns to the lead.

## Project tools, settings and album

Chat is the creation surface for images and 3D models. Show the finished preview, its maker and the relevant actions together in the transcript. Outputs opens the images, models and screenshots for this chat. Tools contains a searchable project asset library and the local decompilation editor. Decompilation run settings apply to one analysis. Global task defaults live in Settings alongside providers, plugins, skills and appearance. Keep manual installation fields behind **Install from source**. Expand a provider to toggle the tasks it may handle. Show the number of enabled tasks beside its name, and keep account actions separate from the toggles. Closing Settings returns to the previous destination and selected output. The album uses a responsive thumbnail grid with the project and capture date under each image.

The Work queue opens from the chat sidebar. Give the task title and its next action the reading space, put the state and provider on a short line above, and open the existing conversation from its row. Order blockers and paused work before active and reviewable work. The queue and the headless status command share one project query. This follows the request for a concrete next action in a session overview and the multi-session status view in Agent of Empires. [Developer discussion](https://www.reddit.com/r/ClaudeAI/comments/1xmzp2n/how_do_you_keep_track_of_multiple_coding_agents/) · [Agent of Empires](https://www.agent-of-empires.com/)

## Appearance

Each preset has a miniature conversation thumbnail and the active theme has a checkmark. Selecting a row applies it immediately. The custom theme editor previews valid color changes inside a sample conversation so the surrounding editor stays legible. Saving applies the colors to the workspace. The additional presets adapt the source palettes to Workshop’s surfaces and text roles. [Nord](https://www.nordtheme.com/docs/colors-and-palettes/) · [Catppuccin](https://catppuccin.com/palette/) · [Solarized](https://ethanschoonover.com/solarized/)

## Components

The files in `themes/` own the preset colors. Their CSS custom properties map directly to `ThemeColors` in `ui/common.slint`, which supplies the runtime palette to app components. The values above document Workshop Dark. Canvas brightness selects the light or dark treatment for standard Slint widgets. Segoe UI carries headings and conversation text. Consolas identifies code, model names and technical metadata. Mint marks the active destination and the assistant speaking. Blue identifies symbols and models, lilac identifies keywords and agent controls, and amber highlights constants and pending review. Capacity meters pair their green-to-red scale with numeric values. Worker states keep explicit labels.

Center the visible label or icon-and-label group inside standalone actions. Align navigation, disclosure and picker rows to the leading edge, reserving space for trailing counts, shortcuts and checkmarks.

Replies use the native transcript in `ui/transcript.slint`. Headings break up long answers. Code sits in a recessed panel with a language header and a keyboard-accessible copy action. Streaming code appears immediately in monospace and gains syntax colors when the reply settles. Saved conversations render through the same component.

| Capability | Canonical owner | Source of truth | Allowed variants | Verification |
|---|---|---|---|---|
| Form | Composer, StudioPage and TaskEditor | Draft, selected run and saved task policy | Chat, task briefs, global and per-run settings | Windows picker suite exercises draft editing, attachments and task overrides |
| Select/Listbox | InferencePicker, EffortTrack and history ListView | Provider catalog, project store, Studio history | Chat and worker policies, chat navigation and Studio run selection | Picker suite exercises focus, model switching, effort and library search |
| Scrollbar | Slint ScrollView, ListView and TextEdit | Pane dimensions and content size | Transcript, activity, catalog, settings and run details | Long content can scroll independently of the composer |
| CRUD | Store and Desk | SQLite attempt state and saved patch | New chat, stop, retry, accept and discard | Recovery tests and private desktop cancellation suite |
