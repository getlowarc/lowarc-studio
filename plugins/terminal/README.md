# Terminal

An integrated shell, in a real PTY: ConPTY on Windows, a native pty everywhere else. Vendors
[@xterm/xterm](https://xtermjs.org/) (see `LICENSE.md`).

## Features

- Multiple concurrent terminal instances, each its own PTY session, switchable via the console
  sidebar.
- Any shell on your `PATH`: PowerShell, `pwsh`, `cmd`, `bash`, `zsh`, or a custom command.
- Configurable font size and scrollback.
- A Command Palette entry ("Terminal: New Terminal") for opening a new instance without touching
  the console header.

## Settings

| Setting | Description |
| --- | --- |
| Shell command | Command used to launch a new terminal. Default is PowerShell on Windows, `$SHELL` elsewhere. |
| Font size | Applies to new terminals opened after the change. |
| Scrollback (lines) | How many lines of output each terminal keeps. Applies to new terminals. |
