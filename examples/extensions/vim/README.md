# Vim Keys — SpeedyF v2 reference extension

Vim-style navigation, and the extension the v1 plan named but could not
support. It needs two things v1 did not have:

- `contributes.keybindings`, to claim `j`, `k`, `/` and the rest
- the `commands:run` permission, to drive SpeedyF once a key fires

Load it from **Home → Extensions → Load folder…**.

| Key       | Does                 |
| --------- | -------------------- |
| `j` / `k` | Next / previous page |
| `+` / `_` | Zoom in / out        |
| `W`       | Fit width            |
| `o`       | Table of contents    |
| `/`       | Search               |

## The shape worth copying

Keys bind to _this extension's_ commands, never directly to SpeedyF's. Every
effect on the app goes through `commands.run`, so all of them pass the
permission gate. A manifest entry alone can never move the document.

## What `commands:run` does not grant

Only command groups that change **what you are looking at**: View, Navigate,
Panels, Search. Anything that changes the document, the file on disk, or your
settings — File, Edit, Page, Tools, Settings — is refused however it is asked
for.

Run **Vim: Try To Undo (refused on purpose)** from the command palette to see
it: the extension asks for `edit.undo`, the gate refuses, and the refusal shows
up in this extension's log as an ordinary rejected call rather than a crash.

## Keys SpeedyF has already claimed

A binding for a chord SpeedyF uses itself is refused at resolve time and
reported in the panel — built-ins always win, so an extension can never take
⌘S. Two extensions wanting the same key is resolved first-come, and the loser
is told which extension holds it.
