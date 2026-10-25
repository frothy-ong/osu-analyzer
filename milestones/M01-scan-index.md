# M01: Scan and index
Status: not started
Depends on: M00
Goal: Find every replay and beatmap in the lazer folder and record them in
SQLite, skipping anything already processed.

## In scope
- Detect the lazer folder (normal and Flatpak paths) with a config override.
- Walk files/ in parallel (rayon). Identify files by CONTENT, not name.
- BEFORE writing the replay parser, ask the user to run `hexdump -C` on one
  replay file and a short command listing a few files, then confirm the
  real layout. Do not rely on memory of the format.
- Parse replay headers: player, beatmap MD5, mods, 300/100/50/miss counts,
  timestamp. Parse beatmaps with rosu-map. Compute each beatmap's MD5.
- Tables: files, beatmaps, replays (see ARCHITECTURE.md).
- Link each replay to its beatmap by MD5.
- Mark replays that are not the user's (player name from config) or use cheat
  mods as excluded, with a reason.
- Print a summary: replays found, beatmaps found, replays with no matching
  beatmap, excluded replays, scan time.

## Out of scope
- Frames, hit simulation, any UI beyond the summary print.
- Reading client.realm.

## Acceptance tests
1. The summary prints plausible counts on the user's real folder.
2. A second run re-parses zero files and finishes in under 5 seconds.
3. Unmatched replays are few, and Claude explains every category left.
4. Nothing in the code opens any path under the lazer folder for writing.
5. A config override changes the scanned folder.
6. `cargo test` passes with unit tests for the header parser using small
   byte arrays built in the test (no real replays committed).

## Open questions
- Which header fields lazer-stored replays actually contain (SPEC section 9,
  item 4).
