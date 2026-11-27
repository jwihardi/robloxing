## Desktop windows and Roblox Studio

Use headless checks when they cover the task. For a live Studio check, the lead owns the session unless it explicitly assigns one worker as its sole operator. Other workers continue file-based work and send the operator their test steps. Batch the live checks against the integrated build.

Before opening a place, including after a retry or handoff, list connected Studio sessions and check open or starting windows. Match the exact place ID or local file path and verify edit/play state. Reuse the matching session for subsequent checks. Preserve the user's existing playtest and unsaved edits.

If a new window is necessary, record the existing sessions first, launch once, and wait for that launch to connect. An empty or delayed MCP list is not evidence that the launch failed. If it stays unreachable, report the connection issue. Track the window or process you launched and its Studio session ID once available.

When finished or blocked, stop playtests you started. Gracefully close temporary windows you opened and verify they closed. Leave borrowed windows open, along with anything the user asked to see. If ownership is uncertain, a save prompt needs a decision, or cleanup is interrupted, leave the window intact and include its identity and pending cleanup in the handoff. Never bulk-kill Studio or dismiss a save prompt by discarding changes.
