# M00: Window and tabs
Status: built, awaiting user check (cargo build passes, window opens, tabs clickable)
Depends on: none
Goal: A native window that opens on Arch Linux with empty tabs.

## In scope
- eframe/egui app with tabs: Overview, Playstyle, Training, Analysis, Tablet,
  Data, Settings.
- Each tab shows only its own name.
- Cargo.toml with pinned crate versions.

## Out of scope
- Any data, database, parsing or charts.

## Acceptance tests
1. `cargo build` succeeds with no errors.
2. `cargo run` opens a window and the user can click between all tabs.
3. ARCHITECTURE.md module table is updated for what now exists.
