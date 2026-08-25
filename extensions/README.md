# Built-in extensions

Everything in this folder ships inside SpeedyF and is loaded from the app
bundle. It is not sample code — for that, see `examples/extensions/`.

A built-in behaves like any other extension: same manifest, same worker, same
permission model, same API. Only three things differ.

- **It is never copied anywhere.** The app reads it out of its own resources, so
  a release updates it and nothing can leave a stale copy behind in
  `~/Library/Application Support`.
- **It cannot be removed, only disabled.** Removing files inside a signed bundle
  would either fail or last until the next update, so the panel offers
  Enabled/Disabled instead. The choice is kept in `builtins.json` beside the
  registry.
- **Its id is reserved.** Installing an extension whose manifest claims a
  built-in's id is refused, and a hand-installed copy from before it shipped is
  removed at startup. One id, one extension — the id is what commands, storage
  keys and asset URLs are all keyed by.

## Adding one

1. Put the folder here, named exactly as its manifest `id`.
2. Add it to `bundle.resources` in `src-tauri/tauri.conf.json`:
   `"../extensions/<id>/*": "extensions/<id>/"`. Discovery is automatic; this
   line only decides what gets copied into the bundle.

Under `cargo tauri dev` the folder is read straight from the checkout, so an
edit here needs a Reload from the extensions panel and nothing more.
