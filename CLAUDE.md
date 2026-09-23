<!-- Canonical project context lives in AGENTS.md (agent-neutral). Keep this file as a one-line import. -->
@AGENTS.md

## Where your changes appear

`docs/` → **https://docs.goldplated.photos** in ~1 min. A push to `main` touching
`docs/**` runs `.github/workflows/docs-deploy.yml`: `mkdocs build -f docs/mkdocs.yml`
(Material theme + minify) → `wrangler deploy --config docs/wrangler.jsonc` (Cloudflare
Worker `goldplated-docs`, NOSCO account, static assets from `docs/site`).
Verify on the live URL, do not deploy by hand.
