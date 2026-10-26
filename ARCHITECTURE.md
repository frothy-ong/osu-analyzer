# ARCHITECTURE

Living document. Claude: update after every milestone. Keep it short.
Mark each item PLANNED until it exists in the code, then change to DONE.

## Modules
| Module | Purpose | Status |
|---|---|---|
| src/main.rs | app entry, egui window, tab shell (7 empty tabs), `scan` and `sim` commands | DONE (M00 user-checked? no; M01/M02 commands not yet user-checked) |
| src/config.rs | config file (~/.config/osu-analyzer/config.toml), lazer path detection | DONE (M01, not yet user-checked) |
| src/scan.rs | walk files/, identify by content, incremental index, summary print | DONE (M01, not yet user-checked) |
| src/replay.rs | replay header, LZMA frames, lazer mod blob | DONE (M02, user-checked) |
| src/sim/objects.rs | beatmap -> objects: mods, HR flip, stacking, windows, density, spacing | DONE (M02) |
| src/sim/engine.rs | hit detection: presses, hover, note lock, early miss, auto-miss | DONE (M02) |
| src/sim/mod.rs | `simulate_replay` (pure), header comparison, mod names | DONE (M02) |
| src/sim/run.rs | run over all kept replays, store rows, print report | DONE (M02) |
| src/db.rs | SQLite schema, analyzer_version, wipe, known-file lookup | DONE (M01/M02); load hits into RAM PLANNED (M03) |
| src/stats | aggregations: bins, UR, keys, radar | PLANNED |
| src/ui | one file per page | PLANNED |

## Commands
- `cargo run --release -- scan`: index the lazer folder into ~/.local/share/osu-analyzer/cache.db and print a summary.
- `cargo run --release -- sim`: run scan (incremental), simulate every kept replay not yet done at this analyzer_version, print the M02 report.
- `cargo run`: open the window.

## Database (SQLite cache; every table rebuildable from lazer files)
Cache file: ~/.local/share/osu-analyzer/cache.db (our own file, never inside the lazer folder).
A stored analyzer_version different from the build's wipes all tables on the next scan.

- meta(key TEXT PK, value TEXT): includes analyzer_version ("4"). DONE.
- files(hash TEXT PK, kind TEXT, size INTEGER, seen_at INTEGER): scan state. DONE.
  kind = replay | beatmap | other | bad_replay | bad_beatmap. A hash already
  present is never re-read.
- beatmaps(md5 TEXT PK, file_hash TEXT, artist, title, version, creator,
  mode INTEGER, cs, ar, od, hp REAL, object_count INTEGER). DONE.
- replays(score_id INTEGER PK AUTOINCREMENT, file_hash TEXT UNIQUE,
  beatmap_md5 TEXT, player TEXT, mods INTEGER, rate REAL, played_at INTEGER,
  n300, n100, n50, nmiss INTEGER, is_mine INTEGER, excluded_reason TEXT). DONE.
  `rate` is set from legacy mod bits at scan and overwritten by the simulator
  with the exact rate (lazer blob) once simulated.
- hits(score_id, object_index, time_ms, offset_ms, cursor_dx, cursor_dy,
  key, judgement, local_density, spacing, obj_x, obj_y). DONE (M02). One flat
  table, PK (score_id, object_index). Loaded into RAM at startup (M03).
  - object_index = index in the .osu file's hit object list (spinners count).
  - time_ms = object time in REAL ms (map time / rate). offset_ms = click minus
    object time in real ms, NULL if the object was never clicked (miss).
    An early click inside 400 ms that is outside the 50-window is a miss WITH an offset.
  - cursor_dx/dy = cursor minus object centre at the object time, osu!px, in true
    map orientation (Hard Rock un-flipped: dy negated). obj_x/obj_y = object centre
    (stack offset applied) in true map orientation.
  - key = K1/K2/M1/M2 or NULL. judgement = 300/100/50/0 (0 = miss).
  - local_density = objects per real second within +-1 s. spacing = osu!px from
    the previous object's end to this object's start (NULL for the first).
- sim_check(score_id PK, analyzer_version, sim_n300, sim_n100, sim_n50, sim_nmiss,
  matches_header, map_type, expected_objects, rate, status, match_incl,
  match_incl_floor, match_excl_floor, miss_diff). DONE (M02).
  status = 'ok' or the failure reason. match_* = exact header match under each
  window-edge rule (diagnostic). miss_diff = sim misses - header misses.
  Replays with no beatmap file get no row and are retried on the next run.

## Verified replay layout (from real lazer files, 2026-10-25)
Stored in files/<2 hex>/<4 hex>/<sha256>. Standard .osr layout: mode u8,
version i32 (seen 30000018), beatmap md5, player, replay md5, n300 n100 n50
geki katu miss (u16), score i32, combo u16, perfect u8, mods i32, life graph
string, timestamp i64 (.NET ticks), data length i32, LZMA data (starts 5d 00 00
20 00), then online score id i64 and a length-prefixed lazer score JSON
(version >= 30000001). Strings are 0x0b + ULEB128 length + UTF-8.
Beatmaps: plain text starting "osu file format v9..v14"; md5 = md5 of file bytes.
Frames text "delta|x|y|keys," with a final -12345 seed frame: verified on 2 real replays.

## Simulator rules (M02)
- Press = a key bit going 0 to 1. Left action = bits M1|K1, right = M2|K2 (like lazer).
  Label K1/K2 only if the K bit is set, else M1/M2. Lazer-recorded replays only
  carry M1/M2.
- Frame time = judgement time. Cursor at a press = that frame's position.
- Hit test: distance to (stacked, HR-flipped) object centre <= 64 * scale,
  scale = (1 - 0.7*(CS-5)/5)/2. HR: CS*1.3, AR*1.4, OD*1.4 (cap 10), y -> 384-y.
  EZ: all *0.5. Difficulty Adjust values from the lazer blob override.
- Windows (map ms, not rate-adjusted): 300 = 80-6*OD, 100 = 140-8*OD, 50 = 200-10*OD,
  each window floored, hit only if |offset| < window (exclusive edge). Verified: reproduces the header counts of 2 real replays exactly. Offsets are divided by rate for storage.
- Press on the earliest unjudged object under the cursor that is alive
  (time - preempt <= now). Note lock: if an earlier unjudged object has not started,
  the press is ignored for later ones. Within 400 ms but outside the 50-window = miss.
  More than 400 ms early = ignored. Objects become misses once past the 50-window.
- Stacking: lazer v6+ algorithm for all maps. Spinners and slider ticks/ends are skipped.

## Decisions
- 2026-10-25 | Pin eframe = "=0.29.1" (exact) | Matches SPEC stack; exact pin keeps builds repeatable.
- 2026-10-25 | Tab state is a simple enum in main.rs for now | Smallest thing that meets M00; pages move to src/ui later.
- 2026-10-25 | Cargo.lock is committed | App binary; keeps dependency versions repeatable.
- 2026-10-25 | SPEC section 9 item 4 resolved: header fields are the standard .osr fields listed above | Verified on 3 real files.
- 2026-10-25 | Pinned: rosu-map 0.2.1, rusqlite 0.40.2 (bundled SQLite), rayon 1.12.0, md-5 0.11.0, serde 1.0.229, toml 1.1.8 | Exact pins per SPEC.
- 2026-10-25 | Files identified by first 32 bytes (replay: mode 0-3, version 20M-40M, 0x0b 0x20; beatmap: "osu file format v") | Content, not name, per SPEC. File name is only used as the dedupe hash.
- 2026-10-25 | Config default player_name = "fr01hy" (name in all sampled replays) | Edit config.toml to change. Name match is case-insensitive.
- 2026-10-25 | replays.score_id is a local autoincrement id, not the online id | Lazer-only/offline scores have no online id.
- 2026-10-25 | Unmatched replays/beatmaps are computed by SQL join at summary time | Scan order cannot cause false "no beatmap".
- 2026-10-26 | SPEC section 9 item 1 resolved: UR and offsets are in REAL ms (map-time offset / rate) | Reflects the player's actual timing error. Windows stay in map time.
- 2026-10-26 | SPEC section 9 item 2 resolved: circle-only maps need exact header counts for >=95% of replays; maps with sliders/spinners need |sim misses - header misses| <= 2 | User accepted defaults. "Circle-only" = no sliders and no spinners.
- 2026-10-26 | Pinned: lzma-rs 0.3.0 (frames), serde_json 1.0.154 and flate2 1.1.10 (lazer mod blob) | Exact pins per SPEC.
- 2026-10-26 | ANALYZER_VERSION = "3" (2 was a half-wiped cache); db::init no longer overwrites a stored version; mismatch wipes all tables | Fixes M01 bug where open() hid a stale version.
- 2026-10-26 | hits gets extra columns obj_x, obj_y | M04 heatmaps need absolute object position; avoids re-reading beatmaps.
- 2026-10-26 | Window-edge rule = floored window, exclusive edge (PRIMARY_RULE) | Matched header counts exactly on 2 real replays (inclusive rules were off by 9 and 3 hits). sim_check still stores exact-match flags for all three rules.
- 2026-10-26 | Slider-map header counts cannot match exactly (header judges whole sliders; we judge heads only) | Only misses are compared, with tolerance.
- 2026-10-26 | Objects after the last replay frame are left unjudged (no hit row) | First real run: failed/quit replays showed ~1900 fake misses because unplayed objects were counted. ANALYZER_VERSION 4.
- 2026-10-26 | Rate: lazer blob DT/NC/HT/DC speed_change if readable, else legacy bits. Variable-rate mods (WU/WD/AS) fail with status "variable rate mod" | Cannot simulate a changing rate.

## Gotchas
- Edition 2024 needs Rust 1.85 or newer. Check with `rustc --version`.
- Arch: if the build fails on missing windowing libraries, run `sudo pacman -S --needed libxkbcommon wayland libx11`.
- Stray empty-purpose file `milestones/M00-` is a duplicate of M00-window.md; delete it.
- Adding M02 dependencies changes Cargo.lock; commit it.
- A cache from before M02 may hold an old `hits` table; `db::wipe` drops and recreates all tables.

## Verified vs not (M02, final)
- User run on the real folder (2750 replays, 372 replays/s): circle-only 2/2 exact; circles+spinners 3/3 within +-2 misses; slider maps 2609/2743 (95.1%) within +-2 misses; 2721 complete plays, 27 incomplete (failed/quit). Remaining slider mismatches come from slider ticks/ends (out of scope for v1), not head hits.
- 121 replays skipped (beatmap file not in folder), 2 failed (variable-rate mod).

### Earlier notes
- Verified (non-GUI crate, Rust 1.91, 36 unit tests; plus 2 real replays: counts match header exactly, blob and DT 1.1x read): frame parsing, LZMA with unknown and known size, gzip/plain lazer blob, stacking, slider end/duration, HR/EZ/DT handling, judgement by offset, early miss, note lock, K1/K2 labels, DB store, re-run skip, report printing.
- First full run (2750 replays, 8 s, 345 replays/s): circle-only 2/2 exact; slider maps 95.0% within +-2 misses before the fail fix; 121 replays have no beatmap file; 2 skipped (variable-rate mod).
- Not verified: `cargo build`/`cargo test` of the full GUI crate, `cargo run --release -- sim` on the real folder, any match against real header counts, the lazer blob format, window-edge rule.
