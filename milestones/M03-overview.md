# M03: Overview page and filter bar
Status: built; tests and `load` verified by user run (46 tests pass, 2871 plays, startup 2.68 s); awaiting window click-through (acceptance 1, 2, 4)
Depends on: M02
Goal: First useful page, plus the global filter bar every page will share.

## In scope
- Load hits and replays into RAM at startup.
- Recompute PP with one pinned rosu-pp version.
- Charts: PP per play over time, plays per month, accuracy over time, mod
  usage. Totals: plays, hours played.
- Filter bar: mods, date range, star range, best-only vs all plays,
  include fails. Filters apply to every chart.
- Clicking a summary number opens a list of the plays behind it.

## Out of scope
- Heatmaps, playstyle, training.

## Acceptance tests
1. Totals match the user's own knowledge of their history (user confirms).
2. Changing any filter changes the charts.
3. Startup load time with the full dataset is reported.
4. Clicking a number shows plays whose count equals that number.
