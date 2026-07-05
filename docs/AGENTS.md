# docs — Project documentation

## Purpose

Durable project documentation and architecture decision records. Reference material for humans
and agents; not a change log.

## Ownership

- `ARCHITECTURE.md` — layout, IPC, tabs, status envelope, profiles model
- `I18N.md` — how localization works, adding strings/locales
- `BUILD.md` — build, release, icon, troubleshooting
- `adr/` — architecture decision records (e.g. `0001-binding-via-settings-json-dummy-token.md`)
- `img/` — banner + per-tab screenshots used by README/docs

## Local Contracts

- Keep docs in sync with the code they describe; when an architecture/i18n/build change lands,
  update the matching doc as part of the DOX pass
- New durable decision → add a numbered `adr/NNNN-*.md`; do not rewrite past ADRs, supersede them
- Screenshots in `img/` are referenced by name; regenerate rather than rename

## Work Guidance

- These docs are authoritative for their topics; `CLAUDE.md` (repo root) remains the project charter

## Verification

None.

## Child DOX Index

None.
