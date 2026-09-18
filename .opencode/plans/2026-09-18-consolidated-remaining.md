# Plan: deconflict — consolidated remaining work

**Date:** 2026-09-18 · **Source:** merged from 2026-08-30-deconflict.md + 2026-08-30-deconflict-tui.md

> **Note:** P0 items (1-4) were implemented in commit 78ad4cb (Meta tab, meld diff, config help button, terminal editor suspend). Changelog update pending.

---

## Remaining items grouped by domain & priority

### P1 — TUI Enhanced (polish / nice-to-have)

| # | Item | Source | Notes |
|---|------|--------|-------|
| 1 | **In-terminal image preview** | TUI plan (v2) | `textual-image` + protocol pre-probe before app start; `p` toggle. |
| 2 | **Command palette / fuzzy group search** | TUI plan (v2) | Quick jump to group by name/pattern. |
| 3 | **Enter → detail / metadata overlay** | TUI plan (v2) | In-TUI `(e)dit` / `(x)edit-meta` actions via overlay. |
| 4 | **Promote session-status + log to engine** | TUI plan (v2) | If/when a GUI (PySide) wants shared state. |
| 5 | **Rich metadata table in Meta tab** | TUI plan (v2) | Reuse `_meta_diff_columns` / `_truncate_name` logic from CLI. |

---

### P2 — Engine Performance & Architecture

| # | Item | Source | Notes |
|---|------|--------|-------|
| 6 | **Batch ExifTool** | main plan G7 | One `exiftool` subprocess for all audio/image/video files of a scan instead of ~0.5s/file. Biggest single win. |
| 7 | **Thread analyze loop** | main plan G7 | `ThreadPoolExecutor.map` over independent groups — easy, safe. |
| 8 | **Optional `fd` backend for scan** | main plan G7 | Config `[scan].use_fd=auto\|fd\|never`; fast file listing when `fd`/`fdfind` on PATH. |
| 9 | **True streaming pipeline (S1→S2→S3)** | main plan G7 | Generator-based: scan groups → analyze stream (workers) → resolve pull. Start resolving first ready group without waiting for all. |

---

### P3 — Dev / Infra

| # | Item | Source | Notes |
|---|------|--------|-------|
| 10 | **Pre-commit hooks** | main plan backlog | ruff check + format, maybe pytest. |
| 11 | **Dependabot / Renovate** | main plan backlog | Automated dependency updates. |
| 12 | **Editor AI integration** | main plan backlog | Copilot / Continue optional setup. |

---

### P4 — Future GUI

| # | Item | Source | Notes |
|---|------|--------|-------|
| 13 | **PySide GUI** | main plan backlog | Reuse UI-free `engine.py`; separate frontend. |

---

## Implementation order recommendation

1. **P2 item 6** — Batch ExifTool (immediate 10–50× speedup on media-heavy scans)
2. **P2 item 7** — Thread analyze (easy parallelism)
3. **P1 items 1–5** — TUI polish (can be done incrementally)
4. **P2 items 8–9** — `fd` + streaming pipeline (architectural, larger scope)
5. **P3 items 10–12** — Dev infra (low risk, high value)
6. **P4 item 13** — PySide GUI (new project, depends on engine stability)

---

## Acceptance criteria for "done"

- All P1–P4 items implemented + tested (`mise run check` green)
- P2 item 6 reduces analyze time for media-heavy scans from ~0.5s/file to ~1s/total
- No regressions in existing 140+ tests
- CHANGELOG.md updated per Keep a Changelog