# Changelog

All notable changes to the Node Graph plugin are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/).

## 0.2.1 - 2026-09-03

### Fixed

- A connections-list row in the Inspector stayed highlighted yellow after being clicked, even
  after the click's own selection highlight (blue) was applied: the Inspector's re-render on
  click was destroying the hovered row before its own `mouseleave` could fire and clear it.

## 0.2.0 - 2026-09-01

### Added

- Inspector-driven editing for a selected node's or connection's details.
- Optional badges on nodes/connections (attached files, port type, connection counts), with a
  setting to toggle them off.

## 0.1.0 - 2026-08-28

Initial release.

### Added

- Visual `.lan` graph editor: place, connect, and rearrange nodes; save back to disk.
