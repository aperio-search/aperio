# docs/ — Aster documentation site

This directory hosts the [Aster](https://github.com/aperio-search/aperio) documentation site built with [VitePress](https://vitepress.dev).

## Commands

```sh
bun docs:dev      # dev server at localhost:5173
bun docs:build    # static build to .vitepress/dist
bun docs:preview  # preview the built site
```

Run from the `docs/` directory using `bun` (not npm/pnpm/yarn).

## Structure

| Path | Purpose |
|---|---|
| `*.md` (root) | Doc pages — each file is a route |
| `.vitepress/config.mts` | VitePress site config (nav, sidebar, social links, edit link) |

Routes map directly from filenames: `/quickstart` → `quickstart.md`, etc.

## Editing conventions

- Config file is `.mts` (TypeScript module).
- Edit link points to `https://github.com/aperio-search/aperio/edit/main/docs/:path`.
- Sidebar order is defined manually in `.vitepress/config.mts`.
- No tests, no linter, no CI config — focus on markdown content accuracy and broken link avoidance.
