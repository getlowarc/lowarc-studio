# Debugger

Pause, step, and set breakpoints on a running project.

Works against any module — it operates entirely on the wire protocol every module already speaks
(frame requests/replies, log lines), never anything module-internal like a source line. This
means it works uniformly across every first-party and third-party module without any
module-specific integration.

## Features

- Pause/resume and single-step a running project.
- Set and clear breakpoints.
- Lives in its own console tab (Debug), alongside Run and any other console-contributing plugin.
