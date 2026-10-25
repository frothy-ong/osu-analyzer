# M07: Training recommendations
Status: not started
Depends on: M06
Goal: Suggest maps from the local library.

## In scope
- Maps similar to what the user plays now (recency-weighted) that train the
  weakest radar axes.
- Maps similar to current play that give the most PP.
- A would-be PP column for hypothetical accuracy.
- Flag underplayed maps where predicted performance exceeds the best score.
- Every row is clickable.

## Out of scope
- Anything needing the network.

## Acceptance tests
1. Every suggested map exists in the local library.
2. Weakest-axis list matches the radar from M06.
3. Would-be PP changes when the hypothetical accuracy is changed.
4. User confirms the suggestions look like maps they would play.
