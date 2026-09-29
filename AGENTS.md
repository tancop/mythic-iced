# AGENTS.md

Single-binary Rust GUI client for Epic Games Store (iced 0.14, edition 2024, nightly toolchain — uses let-chains).

## Commands

- `cargo check` — fast verification (prefer before `run`; no test suite exists).
- `cargo run` — opens GUI window; requires display + live Epic login, not headless-testable.
- `cargo fmt --check` / `cargo clippy` — no CI; run manually before finishing.

## Architecture (`src/`)

- `main.rs` — app shell only: `State`, `Message`, `boot` / `update` / `view`. `update` just dispatches to topic handlers below.
- `library.rs` — library data flow: fetch queue (`CONCURRENT_LIMIT`), catalog lookup, scroll handling, `visible_range` / `evict_stale`.
- `login.rs` — auth flow + refresh-token cache (`tokens.toml`).
- `epic.rs` — Epic HTTP API (isahc): auth, library pagination, catalog lookup. Hosts + OAuth client secrets hardcoded; request helpers via `RequestBuilderExt`.
- `decode.rs` — serde helpers for Epic's odd shapes: `SingleValueWrapper` (single-key map objects), `deserialize_path_list` (`[{path}]` categories).
- `images.rs` — rkyv-serialized thumbnail cache (`HashMap<String, Vec<u8>>` JPEG bytes) + image pipeline: `resize_image`, per-row-chunk async decode (`decode_visible` → `ChunkDecoded`), download completion handling.
- `search.rs` — library toolbar state: `SortKey` (non-empty query overrides it with non-selectable `Search`, sorting by `relevance_score`), DLC `FilterRule` + `is_included` (query text never hides items). `State::order` holds indexes into `catalog_items` in display order (filtered + sorted); the vec itself is never reordered. Order is rebuilt once when all catalog fetches drain (`inflight_fetches`) and on control change — never per arrival, so rows don't jump. The grid shows `Loading...` until fetches complete. Purchase dates live in `State::purchase_dates` (filled at load from `acquisitionDate`).
- `ui/` — `login.rs`, `library.rs`, `theme.rs`; `mod.rs` exposes `get_theme()`.

## Gotchas

- Grid layout lives in `ui/library.rs` (`cols_for_width` 5–7 cols via `TARGET_CARD_WIDTH`, `SPACING`, `BUFFER_ROWS`, `CARD_ASPECT`, `card_width/height`, `row_pitch`); `library.rs` / `images.rs` use it for `visible_range` / `decode_visible` / `evict_stale` — don't duplicate. Cells derive size from `State::viewport_width` (tracked via scrollable `on_scroll` bounds, which also fires on resize). Scrollable height is pinned to the final count (`State::total_items`, set at load, decremented on fetch failure) so the scrollbar doesn't drift as rows stream in; unloaded tail rows are bottom padding. When a DLC/text filter hides items, height is sized from the visible `order` instead.
- Library is Windows-only: `epic::get_library_items` drops records whose `platform` lacks `"Windows"` (plus `IGNORE_NAMESPACES`). DLC/sort toolbar filters are implemented; text search is stubbed (input stored, not filtered).
- Image pipeline order: download → `resize_image` (255×340 JPEG) → `ImageLibrary` → per-row-chunk async decode to RGBA (`ChunkDecoded`, one message per grid row so rows swap in atomically). Catalog fetch concurrency capped at `CONCURRENT_LIMIT = 20`; failed fetches/decodes skip the item and advance the queue rather than wedging it.
- Auth: `boot()` tries refresh token from OS cache dir (`dirs::cache_dir()/mythic/tokens.toml`); else Login → opens browser to Epic → paste `authorizationCode`. Cannot be automated in tests.
- `epic::get_library_items` writes `library_items.json` into repo root on every call (debug side effect). `catalog_item.ron`, `EpicGamesLauncher.log`, `profile.json.gz` are also local debug artifacts — do not commit. Runtime caches (`tokens.toml`, `images.db`) live under OS cache dir `mythic/`, not the repo.
- `opencode.jsonc` quiets rust-analyzer log spam (`wgpu`/`iced_wgpu`/`cosmic_text` off); leave as-is.
