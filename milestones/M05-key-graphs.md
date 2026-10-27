# M05: Key graphs
Status: built; build and 4 unit tests verified by user run; awaiting window check of acceptance 1-3
Depends on: M03
Goal: Analysis > Keys with per-key timing and a fair stamina curve.

## In scope
- Overlaid hit-timing histograms for K1, K2, M1, M2.
- Table: hits, UR, mean offset ms, std ms per key.
- Stamina curve: timing error vs elapsed time, normalized by local note
  density (compare only sections of similar density).
- Global filters apply.

## Out of scope
- Playstyle detection.

## Acceptance tests
1. Per-key hit counts sum to the total hit count.
2. Key stats are plausible against known values for one replay (user checks).
3. The stamina curve changes when density normalization is switched off,
   showing that normalization is doing something.
