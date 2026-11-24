---
version: alpha
name: Workshop
description: A Windows workbench for assigning a task and reviewing a file change.
colors:
  text: "#203449"
  muted: "#526778"
  rail: "#eaf0f4"
  page: "#fbfcfd"
  edge: "#c5d2dc"
  accent: "#155a83"
typography:
  sans: { fontFamily: "Segoe UI" }
  display: { fontFamily: "Georgia" }
  mono: { fontFamily: "Consolas" }
rounded:
  controls: "4px"
spacing:
  pane: "26px"
  rail: "18px"
components:
  heading: {}
  caption: {}
  rule: {}
---

# Workshop

The desktop window is a project notebook for a creator working across sessions. The current task and its patch share a reading column. A quiet blue-gray rail keeps the milestone and previous attempts within reach.

The window uses English, a left-to-right layout and Windows system fonts. Georgia identifies the workbench once in the header. Segoe UI carries task instructions and actions. Consolas preserves the indentation, context and change markers of a Git patch.

## Layout and controls

Use the single row of project navigation for the folder name and folder picker. Keep the rail at 220 logical pixels and give additional width and height to review. The blue accent marks the product name. Slint's light system palette owns focus, selection, disabled states and primary buttons. Status is always readable as text.

The active task controls the available actions. Place acceptance beside the check, with discard on its own action row. A discarded attempt keeps its working copy and artifacts. Long diffs scroll independently. Use the window's existing whitespace and one-pixel dividers for grouping. Transitions are immediate.

The `Ink` global and shared text components in `ui/workshop.slint` own the project palette and type at runtime. Use standard Slint widgets for editing, buttons and keyboard list navigation.

| Capability | Canonical owner | Source of truth | Allowed variants | Verification |
|---|---|---|---|---|
| Form | Desk task editor | Task prompt or steering direction | New task, active turn, resumed turn | Keyboard input and retained text on setup failure |
| Select/Listbox | StandardListView | Saved attempts ordered newest first | Project task history | Selection loads the matching patch |
| Scrollbar | Slint text and list widgets | Available pane height | History and diff | Scroll without moving the decision buttons |
| CRUD | Store and Desk | SQLite attempt state and saved patch | Submit, resume, retry, accept and discard | Recovery tests and native exit demo |
