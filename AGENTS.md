# Testing Workshop

When testing with fixtures, use a temporary project and set `WORKSHOP_HOME` to a temporary data directory for that test process. This keeps fixture conversations out of the user's chat history.

Use `workshop --headless` or `workshop-cli` for command-level tests. For automated Windows UI checks, use the private-desktop harness in `tests/windows/TestDesktop.ps1`, including for ad hoc UI inspection and screenshots. Both Windows UI suites use it automatically. For a command-only test session you can also set `WORKSHOP_HEADLESS=1` in that process. Open Workshop on the user's desktop when they ask to see the app.
