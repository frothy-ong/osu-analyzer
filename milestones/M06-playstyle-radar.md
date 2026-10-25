# M06: Playstyle and radar
Status: not started
Depends on: M02, M03
Goal: Playstyle page with a radar that has a reference point.

## Decide first
Agree the final 8 axes with the user and write each formula into SPEC.md
glossary before coding. Suggested start: Aim, Speed, Accuracy, Stamina,
Reading, Tech, Consistency, Precision. Axes must not overlap heavily.

## In scope
- Pattern detection (alternate, stream, burst, jump, tech) from beatmap data.
- Played style vs mapped style using the key actually pressed.
- Most-played styles and top styles by PP.
- Accuracy grid by spacing x time gap.
- Bias by movement direction and by BPM.
- Radar with two overlaid shapes: Potential and Demonstrated, and an info
  panel showing each axis formula.
- Recent-form vs all-time toggle.

## Out of scope
- Training recommendations.

## Acceptance tests
1. Every axis has a written formula visible in the app.
2. The Potential/Demonstrated gap is computed and displayed per axis.
3. The recent/all-time toggle changes the numbers.
4. User confirms the picture matches their own sense of their play.
