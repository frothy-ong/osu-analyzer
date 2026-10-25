# osu!lazer Offline Replay Analyzer — Technical Specification (`SPEC.md`)

## 1. Overview & Objective
A fast, native, offline desktop application that analyzes a user's entire local `osu!lazer` replay and beatmap library. Unlike web-based analyzers limited by the official API's small sample size and top-play bias, this tool indexes thousands of local plays directly from disk, re-simulates hit detection, and computes statistical, spatial, and mechanical metrics in memory.

### 1.1 Core Constraints & Rules
1. **Lazer Data is Strictly Read-Only:** Never write to, lock, or modify the `osu!lazer` directory or its files.
2. **100% Offline:** No browser wrapper, no web server, no external network calls. All recommendations and metrics derive strictly from the local library.
3. **Cache Versioning:** Every simulation and aggregation algorithm must have an explicit integer constant (`ANALYZER_VERSION`). Mismatches in the SQLite cache trigger an automatic migration/recompute for affected rows to prevent mixing stale and new calculations.
4. **Scope (v1):** `osu!standard` (mode 0) only. Hit simulation evaluates **circles and slider heads only** (slider ticks, slider ends, and spinners are ignored for timing/aim offsets in v1).

---

## 2. Tech Stack & Dependencies
* **Target Platform:** Linux first (Arch Linux native). Windows and macOS supported later as separate compilation targets.
* **Language:** Rust (latest stable, edition 2021/2024).
* **GUI Framework:** `egui` via `eframe`.
* **Storage / Cache:** `rusqlite` (with `bundled` feature) for persistent caching; flat `Vec<HitRecord>` loaded into RAM on startup for real-time filtering and binning.
* **Parallelism:** `rayon` for multi-threaded file scanning, replay parsing, and hit simulation.
* **Domain Crates (Pin exact versions in `Cargo.toml` to prevent API drift):**
  * `osu-db` or `osr` (for `.osr` replay header and LZMA frame decompression/parsing).
  * `rosu-map` (for `.osu` beatmap parsing, object stacking, and difficulty attributes).
  * `rosu-pp` (for star rating, difficulty attributes, and hypothetical PP calculations).
  * `md-5` (for verifying/computing beatmap MD5 hashes when matching).
  * `notify` (for watching the lazer `files/` directory for new plays in real time).

---

## 3. Data Ingestion & Storage Architecture

### 3.1 Lazer Directory Discovery
Auto-detect the `osu!lazer` data directory on startup in the following order, falling back to a manual folder picker in the UI:
1. `$XDG_DATA_HOME/osu` (or `~/.local/share/osu`)
2. `~/.var/app/sh.ppy.osu/data/osu` (Flatpak)
3. Manual path configured in local app settings.

### 3.2 Realm-Free Content Scanning
Do not link or parse lazer's `client.realm` database. Lazer stores all assets in a content-addressed directory structure (`files/<h[0]>/<h[0..2]>/<sha256_hash>`).
1. **Incremental Walk:** Walk the `files/` directory. Use the filename (SHA-256 hash) as the unique `file_hash` primary key. Skip any `file_hash` already recorded in the local SQLite `indexed_files` table.
2. **Magic-Byte / Header Identification:** Read the first 256 bytes of each unindexed file:
   * **Beatmap (`.osu`):** Starts with UTF-8 string `"osu file format v"` (optionally preceded by a UTF-8 BOM `\xEF\xBB\xBF`). Compute its MD5 hash and parse metadata/objects via `rosu-map`.
   * **Replay (`.osr`):** Validate the first byte (`0x00` for osu!standard), followed by a valid integer version (e.g., `>= 20190000`) and valid ULEB128 string prefix (`0x0b` followed by 32 hex characters representing the beatmap MD5 hash).
   * **Other:** Mark as `ignored` in `indexed_files` so subsequent startups skip it immediately.
3. **Replay Filtering:** Discard or flag replays that:
   * Do not match the configured local player username (filters out watched/imported replays).
   * Use non-standard/automation mods (`Relax`, `Autopilot`, `Auto`, `Cinema`, `Target Practice`).
4. **Linking:** Match valid `.osr` files to indexed `.osu` files via the 32-character beatmap MD5 hash stored in the `.osr` header.

### 3.3 SQLite Schema & In-Memory Representation
SQLite serves as the persistent cache (`analyzer_cache.db`). At startup, the flat `hits` and `replays` tables are read into contiguous memory arrays (`Vec<HitRow>`, `Vec<ReplayMeta>`) for sub-millisecond UI filtering.

* **`meta`**: `key TEXT PRIMARY KEY, value TEXT` (stores `analyzer_version`, target `username`, `lazer_path`, `tablet_area_mm`).
* **`indexed_files`**: `file_hash TEXT PRIMARY KEY, file_type INTEGER, indexed_at INTEGER`.
* **`beatmaps`**: `md5 TEXT PRIMARY KEY, file_hash TEXT, artist TEXT, title TEXT, version TEXT, mapper TEXT, ar REAL, cs REAL, od REAL, hp REAL, bpm REAL, stars_nomod REAL, length_ms INTEGER, max_combo INTEGER`.
* **`replays`**: `replay_id INTEGER PRIMARY KEY, file_hash TEXT UNIQUE, beatmap_md5 TEXT, timestamp INTEGER, mods INTEGER, score INTEGER, max_combo INTEGER, c300 INTEGER, c100 INTEGER, c50 INTEGER, c_miss INTEGER, pp REAL, is_fail BOOLEAN, sim_valid BOOLEAN, analyzer_version INTEGER`.
* **`hits` (Flat Table)**:
  * `replay_id INTEGER`
  * `obj_idx SMALLINT`
  * `time_ms INTEGER` (object time in map)
  * `x REAL, y REAL` (true playfield coordinates `0..=512`, `0..=384`, **HR un-flipped** so `y_true = 384.0 - y_hr`)
  * `cursor_x_at_hit REAL, cursor_y_at_hit REAL` (cursor position when key was pressed)
  * `cursor_x_at_zero REAL, cursor_y_at_zero REAL` (interpolated cursor position at the exact `0.0ms` perfect hit timing window)
  * `offset_ms REAL` (**Rate-adjusted** timing error: `(press_time - obj_time) / clock_rate`; negative = early, positive = late)
  * `judgment TINYINT` (`300`, `100`, `50`, `0` for miss)
  * `key_mask TINYINT` (`1` = K1, `2` = K2, `4` = M1, `8` = M2)
  * `prev_dist_px REAL` (distance from previous object)
  * `prev_dt_ms REAL` (rate-adjusted time delta from previous object)
  * `prev_angle_rad REAL` (movement angle from previous trio of objects)
  * `local_nps REAL` (notes per second in a rolling 2-second window around this object, rate-adjusted)

---

## 4. Hit Simulation & Coordinate Normalization

### 4.1 Re-Simulation Engine
Because `.osr` files only store `(dt, x, y, keys)` frames and summary totals, hit detection must be re-simulated against the `.osu` hit objects:
1. Apply mod transforms (`HR`, `EZ`, `DT`, `HT`) to beatmap `CS`, `AR`, `OD`, object positions (including stacking), and clock rate (`1.5x` for DT/NC, `0.75x` for HT/DC).
2. Compute hit windows (`300`, `100`, `50`) from effective `OD` (handling both lazer/classic mod rules as applicable).
3. Step through replay frames chronologically. Detect rising edges on `K1/K2/M1/M2`.
4. Evaluate against the earliest unhit circle or slider head within its active hit window and effective circle radius (`R = 54.4 - 4.48 * CS`). Respect osu!standard notelock rules.
5. **Interpolate Zero-Window Cursor:** For every object, linearly interpolate the replay frames surrounding `obj_time` (`+0.0ms`) to record `cursor_x_at_zero` and `cursor_y_at_zero`.

### 4.2 Oracle Verification
Every simulated replay compares its tallied `(c300, c100, c50, c_miss)` against the `.osr` header's reported counts.
* Because v1 ignores slider ends/ticks (which affect combo and 100/50 downgrades on sliders in classic scoring, or misses on slider breaks), the oracle check verifies that `c300 + c100 + c50 + c_miss == total_hit_objects` and that circle-only maps match the `.osr` header within `< 1%` divergence across the test suite. Flag `sim_valid` on the replay row.

### 4.3 Coordinate & Time Conventions
* **HR Normalization:** All spatial coordinates stored in `hits` are stored in standard playfield space (`512x384`). If `HR` is active on the replay, invert the Y-axis before saving so spatial heatmaps represent actual physical hand/tablet positions rather than mirrored map coordinates.
* **Rate Adjustment:** All timing offsets (`offset_ms`), unstable rate (`UR`), and BPM/delta metrics are divided by the active clock rate (`1.5` for DT, `0.75` for HT) so they reflect real human milliseconds (`ms`), not map-clock milliseconds.

---

## 5. Global UI Architecture

### 5.1 Persistent Global Filter Bar
Displayed at the top of every tab. Modifying any filter immediately slices the in-memory `hits` and `replays` arrays and updates the active view:
* **Recency / Weighting Toggle:** `All-Time` vs `Recent Form` (exponential decay half-life = 30 days, or strict date window).
* **Date Range:** Start / End date pickers + quick presets (`Last 7d`, `30d`, `90d`, `1y`, `All`).
* **Star Rating Range:** Dual-handle slider (`0.0★` to `12.0★`, effective SR with mods).
* **Mods Filter:** Include/Exclude toggles (`NM`, `HD`, `HR`, `DT`, `EZ`, `FL`).
* **Play Selection:** `All Plays` vs `Best Play Per Map Only`.
* **Pass/Fail:** `Passed Only` vs `Include Fails` (replays where health reached 0 or quit early).

### 5.2 Interactive Drill-Down Rule
**Every summary statistic, heatmap bin, chart bar, or table cell is clickable.** Clicking any metric opens a modal/side-panel listing the exact underlying replays and beatmaps that contributed to that number, sortable by date, PP, or error magnitude, with a one-click jump to the **Single-Play Detail** view.

---

## 6. Feature Specifications by Tab

### Tab 1: Overview
* **PP & Skill Trajectory:** Scatter + rolling percentile lines (95th, 50th) of play PP and effective SR over time.
* **Session Grouping:** Automatically cluster plays into sessions separated by inactivity gaps `>= 30 minutes`.
  * **Warm-up & Fatigue Curve:** Plot normalized accuracy and UR against play number / elapsed minutes within a session to reveal warm-up duration and fatigue/tilt onset.
  * **Time-of-Day Performance:** 24-hour polar/bar chart showing play volume vs mean UR/accuracy by hour of day.
* **Retry Analysis:** Group consecutive attempts on the same beatmap within a session. Show distribution of attempts needed to set a session-best score and accuracy trajectory across retries (`Attempt 1` through `Attempt 10+`).

### Tab 2: Spatial & Timing Analysis (Heatmaps & Key Graphs)
#### Spatial Playfield Heatmaps
* **Coarse Binning Architecture:** Pre-aggregate the `512x384` playfield into a `128x96` grid (4×4 osu!px per bin). Each bin stores `{count, sum_val, sum_sq_val, object_count}`.
* **Real-Time Display Controls:**
  * **Gaussian Smoothing Slider ($\sigma$):** Applied as a separable 2D convolution over the `128x96` grid on the UI thread (executes in `< 1ms`).
  * **Minimum Sample Threshold Slider ($N_{\min}$):** Bins with `count < N_{\min}` (prior to or weighted by smoothing) fade alpha toward `0.0` and display a hatched/dimmed background. Hovering any region always shows exact `n = X hits` in the tooltip.
  * **Judgment Filter Checkboxes:** Toggle inclusion of `300`, `100`, `50`, and `Miss` hits.
* **Heatmap Modes:**
  1. **Accuracy %:** Mean hit value (`300=1.0, 100=0.333, 50=0.167, Miss=0.0`) per bin.
  2. **Normalized Click Density (Clicks per Object):** `total_clicks_in_bin / total_objects_in_bin`. Eliminates mapper placement bias (center-bias) to show where the user over-clicks or misses relative to object presence.
  3. **Mean Offset Error:** Signed mean `offset_ms` per bin (diverging colormap: Blue = Early, White = 0ms, Red = Late).
  4. **Unstable Rate (UR):** `10.0 * sqrt((sum_sq / count) - (sum / count)^2)` per bin.
  5. **Directional Bias Map (2D Vector / Bivariate Color Map):** Uses `(cursor_x_at_zero - x, cursor_y_at_zero - y)`—the cursor error vector at the exact `0.0ms` perfect timing window.
     * **Red Channel ($X$-axis bias):** Higher red = cursor biased rightward (`+X`); lower red = leftward (`-X`).
     * **Blue Channel ($Y$-axis bias):** Higher blue = cursor biased upward (`-Y` in screen space); lower blue = downward (`+Y`).
     * Optional vector arrow overlay showing mean error direction and magnitude per `16x12` macro-cell.

#### Key Timing & Stamina Graphs
* **Per-Key Timing Distribution:** Overlaid kernel density / histograms of `offset_ms` separated by `K1`, `K2`, `M1`, `M2`.
* **Key Statistics Table:** Per key: Total Hits, Share %, Mean Offset (`ms`), Std Dev (`ms`), UR, Double-tap rate (both keys pressed within `< 15ms`), and Finger-lock rate.
* **Density-Normalized Stamina Curve:** Plots rolling timing error (`|offset_ms|` and `UR`) against elapsed map time (`0s` to `300s+`), **stratified or normalized by `local_nps` (notes per second)** (e.g., separate curves for `< 4 NPS`, `4–7 NPS`, `7+ NPS`) so map pacing (