---
version: alpha
name: Workshop
description: A dark Windows workspace for directing Codex and following its runs.
colors:
  page: "#1c1c22"
  rail: "#15151a"
  panel: "#26262d"
  edge: "#38383f"
  text: "#f1ede7"
  muted: "#b0aeb8"
  accent: "#edb487"
typography:
  sans: { fontFamily: "Segoe UI" }
  display: { fontFamily: "Georgia" }
  mono: { fontFamily: "Consolas" }
rounded:
  controls: "4px"
spacing:
  pane: "32px"
  rail: "16px"
components:
  signal: {}
  run-link: {}
  heading: {}
  label: {}
  rule: {}
---

# Workshop

Workshop is a single-agent desktop tool for Windows creators. The dark rail holds project navigation and run history. Each run opens onto the agent, its assignment and its recent activity. A four-bar copper mark identifies the agent across the assignment and session views.

## Layout and controls

Reserve 244 logical pixels for the rail and let the reading column grow with the window. Show the assignment once above the feed. Keep directions and decisions within reach while history and activity scroll independently.

Use Georgia for the Workshop name, Segoe UI for instructions and controls, and Consolas for local event times. Use the accent for the selected run, agent mark and status text. Group content with space and quieter surfaces. The `Ink` global in `ui/workshop.slint` owns the runtime palette. Native Slint controls use the dark Fluent palette.

Activity comes from saved events for the selected attempt. Display recent agent messages, directions, checks and named actions in reverse chronological order. The CLI provides the full patch and provider logs.

| Capability | Canonical owner | Source of truth | Allowed variants | Verification |
|---|---|---|---|---|
| Form | Desk task editor | Task prompt or steering direction | New assignment, active turn, resumed turn | Keyboard input and retained text on setup failure |
| Select/Listbox | RunLink in the history ListView | Saved attempts ordered newest first | Selected, idle and busy | Selecting a run loads its assignment and activity |
| Scrollbar | Slint scroll and list widgets | Available pane height | Assignment, history and activity | Scroll independently from the run controls |
| CRUD | Store and Desk | SQLite attempt state and saved patch | Submit, resume, retry, accept and discard | Recovery tests and native exit demo |
