# Synthetic fixtures

Hand-written inputs live here; they must not depend on network access. Planned
coverage (P1):

- `hoogle/`: one `.txt` per edge case: header/version mismatch, modules with no
  entries (re-export-only), all entity kinds, constructors and record fields,
  GADT constructors, patterns, fixities, instances, class `where {}` blocks,
  deprecation doc blocks, adjacent entries without blank lines, Unicode, and
  malformed input.
- `cabal/`: multi-library, `reexported-modules`, common stanzas, conditionals,
  old `Cabal-Version`, truncated/malformed.
- `pairs/`: complete old/new pairs used by the offline tests, with expected
  JSON under `fixtures/expected/`.

Nothing is committed here until P1; this placeholder keeps the layout visible in
the scaffold.
