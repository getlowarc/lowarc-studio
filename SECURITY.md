# Security

## Reporting a vulnerability

Report it privately, not as a public issue. Use GitHub's
[private vulnerability reporting](https://github.com/getlowarc/lowarc-studio/security/advisories/new)
on this repository.

Include what you need to make the case: the version (Help, then About, or `lowarc --version`), the
platform, what an attacker gets, and the smallest project or module that shows it. A working repro
is worth more than a description.

Expect an acknowledgement within a few days. LowArc is a small project, so a fix may take longer
than that; you will hear where it stands rather than nothing.

## What is in scope

Studio runs untrusted things on purpose, and the boundaries between them are the interesting part:

- **Plugins** render in a sandboxed iframe with no filesystem access and no direct `invoke`. A
  plugin reaching disk, another plugin's state, or a host command it was never granted is a bug.
- **The plugin asset server** serves plugin files over loopback HTTP, scoped to one plugin folder
  each. A request escaping its own folder is a bug.
- **Modules** are separate processes, deliberately. They are not sandboxed and are not meant to be:
  installing a module is the same trust decision as running any program. A module reading your
  files is the feature. A module reaching something *through Studio* that it could not reach on its
  own is a bug.
- **Projects.** Opening a project must not execute anything from it. Running it, obviously, does.
- **The updater** verifies a signature before applying anything. Any path that applies an unsigned
  or wrongly-signed update is a bug, and a serious one.

## What is not

- A module or plugin doing what it plainly says it does, however unwelcome. Vet what you install.
- Anything requiring an attacker who already has your user account on your machine.
- Findings from a scanner with no demonstrated path to exploiting them.

## Supported versions

The beta supports the latest release only. There are no backported fixes; the fix ships in the next
version. See [docs/versioning.md](docs/versioning.md).
