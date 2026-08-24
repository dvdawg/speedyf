# Hello — SpeedyF v1 reference extension

The smallest thing that exercises the whole v1 extension surface. Load it from
**Home → Extensions → Load folder…** and point at this directory.

It contributes four commands, all reachable from the command palette:

| Command                                 | Demonstrates                                          |
| --------------------------------------- | ----------------------------------------------------- |
| Hello: Say Hello                        | A command that runs and succeeds                      |
| Hello: Fail On Purpose                  | A command that reports failure, surfaced in the panel |
| Hello: Hang On Purpose                  | A command that never answers — the host times out     |
| Hello: Ask For Something It Cannot Have | The permission gate refusing a call, without crashing |

Run the last two and check the Extensions panel: SpeedyF stays responsive, and
the extension's log records what went wrong. That containment is the point of
running extension code in a worker.

## What v1 does not have

No host APIs, no UI beyond palette commands, no network, no storage. An
extension can be invoked and can report back; that is the whole contract. See
`extension_api.md` at the repo root for what v2 and v3 add.

## Notes for extension authors

- `entry` must be self-contained. Worker source is loaded through a blob URL,
  so relative `importScripts` will not resolve — ship a bundle.
- Your command ids are namespaced with your extension id before they reach the
  palette: `dev.speedyf.hello/greet`.
- Anything you `postMessage` that is not a valid protocol message is dropped
  and logged, not silently ignored.
