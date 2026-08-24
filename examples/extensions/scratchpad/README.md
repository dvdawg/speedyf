# Scratchpad — the narrow path

Collects passages you select and citations you hover into a list you can copy
out. Load it from **Home → Extensions → Load folder…**.

Read its manifest first. It asks for `clipboard:write` and `storage` — and
nothing else. **No `document:read`, no `citations:read`.** It never reads the
document.

Everything it holds arrived because you selected text or hovered a citation and
then chose its action, and it is handed exactly that and nothing more. This is
what selection and citation actions are for: a translate, summarise, or
note-taking extension should look like this rather than asking for the whole
document because that was the only way to get at a paragraph.

## Try it

1. Select a sentence, open the command palette, run **Keep in Scratchpad**.
2. Hover a DOI or arXiv citation and press **Keep in Scratchpad** in the popover.
3. Open the **Scratchpad** panel in the sidebar.
4. Run **Scratchpad: Copy Everything**.

Kept items survive a restart, because `storage` is namespaced to this extension
and persisted. Uninstalling it removes that data.
