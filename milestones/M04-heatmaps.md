# M04: Heatmaps
Status: not started
Depends on: M03
Goal: Analysis > Heatmaps with bias-corrected, instantly adjustable maps.

## In scope
- Bin the playfield at 128x96. Store sum, sum-of-squares and count per bin.
- Blur at display time so the smoothing slider is instant.
- Un-flip Hard Rock so everything is in true playfield coordinates.
- Heatmaps: accuracy%, click density divided by object density (clicks per
  object), mean offset error, unstable rate, directional bias (red = rightward,
  blue = upward, relative to the circle at offset 0 ms).
- Controls: 300/100/50/miss checkboxes, smoothing slider, minimum-sample
  slider that fades low-sample bins. Hover shows sample count.

## Out of scope
- Key graphs, tablet tips.

## Acceptance tests
1. Moving the smoothing slider updates the view with no visible delay.
2. Bins below the minimum-sample threshold are faded.
3. Click density is the divided version, not raw counts.
4. Directional bias looks neutral for a replay set known to be centered
   (user checks).
5. Hover shows the sample count for the bin.
