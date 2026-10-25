# M08: Tablet tips
Status: not started
Depends on: M02
Goal: Tablet page with measured area stats and plain-English tips.

## In scope
- Real used area (99.5th percentile bounding box), edge usage, rest jitter,
  peak cursor speed from replay frames.
- Everything in osu!px unless the user enters their tablet area in mm.
- With the area entered: convert to mm and give tips (e.g. area could be
  shortened or thinned).

## Acceptance tests
1. Used-area numbers match a manual check on one replay.
2. With no tablet size entered, no mm values are shown.
3. Tips only appear when their condition is measured, with the number shown.
