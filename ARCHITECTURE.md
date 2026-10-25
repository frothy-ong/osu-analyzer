# ARCHITECTURE

Living document. Claude: update after every milestone. Keep it short.
Mark each item PLANNED until it exists in the code, then change to DONE.

## Modules
| Module | Purpose | Status |
|---|---|---|
| src/main.rs | app entry, egui window, tab shell (7 empty tabs) | DONE (M00, not yet user-checked) |
| src/config | config file, lazer path detection | PLANNED |
| src/scan | walk files/, identify by content, incremental index | PLANNED |
| src/replay | replay header + frame decoding | PLANNED |
| src/sim | hit-detection simulator | PLANNED |
| src/db | SQLite schema, load hits into RAM | PLANNED |
| src/stats | aggregations: bins, UR, keys, radar | PLANNED |
| src/ui | one file per page | PLANNED |

## Database (SQLite cache; every table rebuildable from lazer files)
All planned. Types are a first draft; change them if a better choice appears
and record why under Decisions.

- meta(key TEXT PK, value TEXT): includes analyzer_version.
- files(hash TEXT PK, kind TEXT, size INTEGER, seen_at INTEGER): scan state.
  A file hash already present is never re-processed.
- beatmaps(md5 TEXT PK, file_hash TEXT, artist, title, version, creator,
  mode INTEGER, cs, ar, od, hp REAL, object_count INTEGER).
- replays(score_id INTEGER PK, file_hash TEXT, beatmap_md5 TEXT, player TEXT,
  mods INTEGER, rate REAL, played_at INTEGER, n300, n100, n50, nmiss INTEGER,
  is_mine INTEGER, excluded_reason TEXT).
- hits(score_id, object_index, time_ms, offset_ms, cursor_dx, cursor_dy,
  key, judgement, local_density, spacing). One flat table. Loaded into RAM at
  startup. Every graph and heatmap is an aggregation of this table.
- sim_check(score_id, analyzer_version, sim_n300, sim_n100, sim_n50,
  sim_nmiss, matches_header INTEGER): the oracle test results from M02.

## Decisions
- 2026-10-25 | Pin eframe = "=0.29.1" (exact) | Matches SPEC stack; exact pin keeps builds repeatable.
- 2026-10-25 | Tab state is a simple enum in main.rs for now | Smallest thing that meets M00; pages move to src/ui later.
- 2026-10-25 | Cargo.lock is committed | App binary; keeps dependency versions repeatable.

## Gotchas
- Edition 2024 needs Rust 1.85 or newer. Check with `rustc --version`.
- Arch: if the build fails on missing windowing libraries, run `sudo pacman -S --needed libxkbcommon wayland libx11`.
- Stray empty-purpose file `milestones/M00-` is a duplicate of M00-window.md; delete it.
