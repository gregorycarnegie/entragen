---
name: run-mutants
description: Run cargo-mutants on entragen's `common` crate and read the results. Use when asked to run mutation testing, mutants, to measure or re-measure test quality, to check whether new tests killed the surviving mutants, or to find where the tests are weak.
---

# Mutation testing entragen

Line coverage says a line ran; it does not say a test would fail if the line
were wrong. cargo-mutants changes the code and checks whether anything notices.

One command, from the repo root, ~80s for the whole crate:

```powershell
cargo mutants -p common
```

Results land in `mutants.out/` — `missed.txt`, `caught.txt`, `unviable.txt`,
`timeout.txt`. Re-check one file after writing tests with `--file` (matches on
the **basename**):

```powershell
cargo mutants -p common --file xlsx_parser.rs --file csv_parser.rs
```

One invocation, however many files: the tree copy and baseline build are paid
once.

## `-p common` is not optional

`app` is a Leptos CSR crate with no tests. It compiles for the host target, so
mutants will happily mutate it and report every mutant as missed — 100% noise.
There is nothing to measure there until the app has tests; the logic worth
mutating all lives in `common`.

## Baseline (2026-08-30, v1.2.2)

`64 mutants tested in 2m: 59 caught, 5 unviable` - every viable mutant caught.

Compare **percentages**, never raw counts: the count moves with the tree. A
clean run is the expected state here; a survivor in `common` means a real gap
opened, so treat it as a regression rather than noise.

Two rounds got it there, and both are worth knowing about:

- `spreadsheet.rs` (then `xlsx_parser.rs`) had no tests at all - 7 survivors. `common/fixtures/*.xlsx`
  plus the tests in that module killed them. Three of the seven turned out to be
  **equivalent** mutants in a short-row padding loop: a calamine `Range` is
  rectangular, so the loop could never run, and it was deleted rather than
  tested. That is the right outcome for an equivalent mutant.
- The remaining 6 were all **degenerate fixtures**, not missing tests - the
  tests ran the code and asserted exact values, and still could not tell two
  operators apart. See below; it is the failure mode to expect here.

## Fixture hygiene — the commonest cause of a surviving mutant

Most survivors are not missing tests. They are tests whose *numbers* make two
different operators agree, so an exact-value assertion still fails to
discriminate.

| fixture value | what silently collapses |
|---|---|
| `processed = 3, missing = 2` | `p - m` and `p / m` are both 1 (a real survivor here) |
| `qty = 1` | `x * qty`, `x / qty` and `x` are one number |
| a minimum of `0` | `v - min` and `v + min` agree |
| one side of a difference `= 0` | `a - b` and `a + b` agree |
| a single-row or single-group fixture | `first()`, `last()` and any index agree |
| every name matching one clause of an `||` | the other clauses are never exercised |
| every skipped row failing *both* halves of an `&&` | `&&` and `||` agree |

**The rule:** every number that flows through arithmetic should be non-zero,
non-unit, and pairwise non-dividing. `processed = 5, missing = 3`, not `3` and
`2`. Three houses, not one.

**The boolean equivalent:** for every `&&` or `||` in the code, one fixture must
fail *only* the left side and another *only* the right, or the two operators are
indistinguishable however exact the assertions look. Concretely, what fixed the
survivors here: an Entra row with a mail but a blank `id` (not both blank), an
emails row with an admission number but a blank email, and a house name
(`St-Mary's`) holding a character from each clause of
`is_alphanumeric() || c == '-' || c == '_'` plus one from none of them.

**The ordering that matters:** when a mutant survives a test that already
asserts an exact value, the first hypothesis is "my fixture is degenerate", and
only the second is "equivalent mutant". Getting that backwards wastes a
re-check cycle and can retire a real gap as unreachable.

## Reading the outcomes

- **missed** — the mutant survived. A gap, or a degenerate fixture, or genuinely
  equivalent code.
- **caught** — a test failed. Working as intended.
- **unviable** — the mutant did not compile. Ignore; the type system already
  rules it out.
- **timeout** — usually a mutant that turned a loop bound into an infinite loop.
  Ignore unless the count is large.

## Where mutation testing has low signal here

- **Redundant defensive code.** Two guards covering the same case (an
  `is_empty()` *and* an `all(|h| h.is_empty())`) make each other untestable.
  That is a signal to delete one, not to write a test.
- **`to_string().trim().to_string()` chains.** Mutants that swap the order of
  equivalent normalisation steps survive by definition.
- **Error-formatting arms.** Mutating a `map_err` message body survives unless a
  test asserts on the string, and asserting on error prose is churn.

## Tooling notes

- `cargo-mutants` 27.1 and `cargo-nextest` 0.9.140 are both installed. mutants
  drives plain `cargo test` by default, which is right for these fast unit tests —
  `--test-tool nextest` buys nothing at this size.
- No `--gitignore` flag needed: `target/` and `dist/` are ignored and there are no
  nested git repos to trip the tree copy.
- Workbook fixtures (.xlsx and .ods) are generated by `common/fixtures/mkxlsx.py` (stdlib only, no
  openpyxl). Edit the row tables in that script and re-run it; never hand-edit
  the `.xlsx`. `parse_any` takes `&[u8]`, so tests `include_bytes!` them - no
  `tempfile`, no filesystem.
- For a parameterised relationship (CSV short-row padding, tag
  de-duplication) a `proptest` over many shapes kills a whole class of mutants
  where hand-picked values cannot. It is a `common` dev-dependency; use it for
  invariants across generated inputs and `rstest` for named input cases.
