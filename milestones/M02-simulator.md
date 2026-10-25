# M02: Hit simulator (make-or-break)
Status: not started
Depends on: M01
Goal: Turn each replay into per-hit rows (which object, offset in ms, cursor
position, key, judgement), verified against the replay header counts.

## Decide first (ask the user, then record in ARCHITECTURE.md Decisions)
1. Are UR and offsets rate-adjusted for DT/HT?
2. Tolerance for count mismatches.

## In scope
- Write the test harness FIRST.
- Decode cursor frames and key states from each replay.
- Simulate circles and slider heads: hit windows from OD, note lock, Hard
  Rock vertical flip, Easy and HR size and window changes, DT/HT rate scaling.
- Per-hit rows: score_id, object_index, time_ms, offset_ms, cursor_dx,
  cursor_dy, key, judgement, local_density, spacing.
- Store hits and sim_check rows. Bump analyzer_version.

## Out of scope
- Slider ticks and ends, spinners.
- Relax, Autopilot, Auto replays and replays that are not the user's.

## Acceptance tests
1. On circle-only maps, simulated 300/100/50/miss counts match the header
   exactly for at least 95% of replays (confirm the threshold with the user).
2. Mismatches are reported grouped by mods and map type, with the 3 worst
   shown hit-by-hit.
3. On maps with sliders, hits + misses equals the object count and the
   simulated miss count is within the agreed tolerance.
4. Re-running skips replays already simulated at this analyzer_version.
5. Full run speed is reported (replays per second).

## If this fails
Stop. Do not start M03. Fix the simulator first.
