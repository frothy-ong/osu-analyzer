# ARCHITECTURE

Living document. Claude: update after every milestone. Keep it short.
Mark each item PLANNED until it exists in the code, then change to DONE.

## Modules
| Module | Purpose | Status |
|---|---|---|
| src/main.rs | app entry, egui window, tab shell (7 empty tabs), `scan` command | DONE (M00 user-checked? no; M01 `scan` not yet user-checked) |
| src/config.rs | config file (~/.config/osu-analyzer/config.toml), lazer path detection | DONE (M01, not yet user-checked) |
| src/scan.rs | walk files/, identify by content, incremental index, summary print | DONE (M01, not yet user-checked) |
| src/replay.rs | replay header parsing (frame decoding is M02) | header DONE (M01); frames PLANNED |
| src/sim | hit-detection simulator | PLANNED |
| src/db.rs | SQLite schema, analyzer_version, known-file lookup | DONE (M01); load hits into RAM PLANNED |
| src/stats | aggregations: bins, UR, keys, radar | PLANNED |
| src/ui | one file per page | PLANNED |

## Commands
- `cargo run -- scan`: index the lazer folder into ~/.local/share/osu-analyzer/cache.db and print a summary.
- `cargo run`: open the window.

## Database (SQLite cache; every table rebuildable from lazer files)
Cache file: ~/.local/share/osu-analyzer/cache.db (our own file, never inside the lazer folder).

- meta(key TEXT PK, value TEXT): includes analyzer_version. DONE.
- files(hash TEXT PK, kind TEXT, size INTEGER, seen_at INTEGER): scan state. DONE.
  kind = replay | beatmap | other | bad_replay | bad_beatmap. A hash already
  present is never re-read.
- beatmaps(md5 TEXT PK, file_hash TEXT, artist, title, version, creator,
  mode INTEGER, cs, ar, od, hp REAL, object_count INTEGER). DONE.
- replays(score_id INTEGER PK AUTOINCREMENT, file_hash TEXT UNIQUE,
  beatmap_md5 TEXT, player TEXT, mods INTEGER, rate REAL, played_at INTEGER,
  n300, n100, n50, nmiss INTEGER, is_mine INTEGER, excluded_reason TEXT). DONE.
- hits(score_id, object_index, time_ms, offset_ms, cursor_dx, cursor_dy,
  key, judgement, local_density, spacing). PLANNED (M02). One flat table.
  Loaded into RAM at startup. Every graph and heatmap aggregates this table.
- sim_check(score_id, analyzer_version, sim_n300, sim_n100, sim_n50,
  sim_nmiss, matches_header INTEGER). PLANNED (M02).

## Verified replay layout (from real lazer files, 2026-10-25)
Stored in files/<2 hex>/<4 hex>/<sha256>. Standard .osr layout: mode u8,
version i32 (seen 30000018), beatmap md5, player, replay md5, n300 n100 n50
geki katu miss (u16), score i32, combo u16, perfect u8, mods i32, life graph
string, timestamp i64 (.NET ticks), data length i32, LZMA data (starts 5d 00 00
20 00), then online score id i64 and a length-prefixed lazer score JSON
(version >= 30000001). Strings are 0x0b + ULEB128 length + UTF-8.
Beatmaps: plain text starting "osu file format v9..v14"; md5 = md5 of file bytes.

## Decisions
- 2026-10-25 | Pin eframe = "=0.29.1" (exact) | Matches SPEC stack; exact pin keeps builds repeatable.
- 2026-10-25 | Tab state is a simple enum in main.rs for now | Smallest thing that meets M00; pages move to src/ui later.
- 2026-10-25 | Cargo.lock is committed | App binary; keeps dependency versions repeatable.
- 2026-10-25 | SPEC section 9 item 4 resolved: header fields are the standard .osr fields listed above | Verified on 3 real files.
- 2026-10-25 | Pinned: rosu-map 0.2.1, rusqlite 0.40.2 (bundled SQLite), rayon 1.12.0, md-5 0.11.0, serde 1.0.229, toml 1.1.8 | Exact pins per SPEC.
- 2026-10-25 | Files identified by first 32 bytes (replay: mode 0-3, version 20M-40M, 0x0b 0x20; beatmap: "osu file format v") | Content, not name, per SPEC. File name is only used as the dedupe hash.
- 2026-10-25 | Config default player_name = "fr01hy" (name in all sampled replays) | Edit config.toml to change. Name match is case-insensitive.
- 2026-10-25 | replays.score_id is a local autoincrement id, not the online id | Lazer-only/offline scores have no online id.
- 2026-10-25 | replays.rate comes from legacy mod bits only (DT/NC 1.5, HT 0.75) | Custom lazer rates are in the trailing JSON; read in M02.
- 2026-10-25 | Unmatched replays/beatmaps are computed by SQL join at summary time | Scan order cannot cause false "no beatmap".

## Gotchas
- Edition 2024 needs Rust 1.85 or newer. Check with `rustc --version`.
- Arch: if the build fails on missing windowing libraries, run `sudo pacman -S --needed libxkbcommon wayland libx11`.
- Stray empty-purpose file `milestones/M00-` is a duplicate of M00-window.md; delete it.

## Verified vs not (M01)
- Verified: non-GUI modules type-check (`cargo check --tests`, Rust 1.85); header layout matches 3 real hexdumps by hand.
- Not verified: `cargo test`, `cargo run -- scan` on the real folder, GUI build with the new modules.
