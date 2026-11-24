---
version: alpha
name: Workshop
description: A dark Windows chat window for talking to a project agent.
colors:
  canvas: "#202020"
  sidebar: "#181818"
  surface: "#303030"
  selected: "#3b3b3b"
  line: "#474747"
  text: "#f5f5f5"
  muted: "#b5b5b5"
  accent: "#eeeeee"
typography:
  sans: { fontFamily: "Segoe UI" }
  display: { fontFamily: "Segoe UI" }
rounded:
  composer: "27px"
  message: "14px"
spacing:
  reading-gutter: "28px"
  sidebar: "12px"
components:
  composer: {}
  action: {}
---

# Workshop

A single sidebar holds New chat, conversation history and the project picker. The composer sits at the bottom in both new and existing chats. The opening question is centered in the space above it. Multiline drafts expand upward to a fixed scrollable height. A pill beside Send shows the current model and effort. It opens a rounded panel where the effort title and model name share a full-width button above the stepped slider. This button opens the scrollable model list. Each stop corresponds to a level supported by that model. In a conversation, the chat title identifies the transcript and its menu offers retry, accept and discard as the conversation state allows.

Right-align the creator's messages on a soft graphite surface. Set Codex replies directly on the canvas and let both be selected and copied. Keep messages in time order within a 720-pixel reading column. This arrangement takes inspiration from the [KiroCrew session screen](https://github.com/kirodotdev/KiroCrew/blob/main/.github/screenshots/25249/composer-meter-after-model-switch-dark.png).

Delegating adds an expandable worker tree above the transcript. Each branch pairs an assignment title with its current state. Opening a branch reveals activity, scope, retries, usage and the running worker’s stop action. The tree stays visible while the transcript scrolls and has its own height limit. Direct chats use the full reading space. The persistent rows and per-worker controls take inspiration from KiroCrew. [SubagentProgressBar](https://github.com/kirodotdev/KiroCrew/blob/f4ed7c1fe7e857a533381f9f852b8a17fdee6eca/website/src/pages/chat/SubagentProgressBar.tsx)

The sidebar narrows from 224 to 192 logical pixels on smaller windows. Segoe UI carries the conversation. The wordmark and opening question use the same family at semibold weight. Neutral charcoal surfaces frame the reading column. The bright slider fill and checkmark identify the selected effort and model. The `Theme` global in `ui/workshop.slint` owns the runtime tokens.

| Capability | Canonical owner | Source of truth | Allowed variants | Verification |
|---|---|---|---|---|
| Form | Composer | Local draft and selected provider session | Start a chat, steer, follow up | Enter sends, Shift+Enter inserts a newline |
| Select/Listbox | InferencePicker, EffortTrack and history ListView | Codex model catalog, project defaults and selected chat | Composer and worker settings share the model and effort controls | Each model exposes its supported effort levels and selections survive reopening |
| Scrollbar | Slint scroll and list widgets | Available pane height | History, messages and composer | Long conversations scroll above the composer |
| CRUD | Store and Desk | SQLite attempt state and saved patch | New chat, stop, retry, accept and discard | Recovery and transcript tests |
