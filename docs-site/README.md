# tt-pane documentation

Astro + Starlight, with an animated position playground and coordinated light/dark themes. The structure follows tt-git's custom hero + standard documentation approach. Product behavior is sourced from the root README, `src/config.rs`, and `crates/tt-pane-platform/src/geometry.rs`.

Requires Node.js 22.12+ (Node 24 recommended).

```sh
cd docs-site
npm ci
npm run dev
npm run check
npm run build
npm run preview
```

Open the printed local URL under `/tt-pane/`. Production output is `docs-site/dist/`. Search is indexed by Pagefind during the production build; use preview to test it.

The configuration targets `https://thirtytech.github.io/tt-pane/`. The `Docs` workflow in `.github/workflows/docs.yml` installs locked dependencies, checks types, builds the site, and deploys it to GitHub Pages when documentation or workflow changes are pushed to `main`. Pull requests run the checks and build without deployment. Manual runs are supported; only runs on `main` deploy.

One-time repository setup: **Settings → Pages → Build and deployment → Source → GitHub Actions**. The workflow must be merged to `main` before automatic publication starts. It uses the `github-pages` environment and grants Pages/OIDC write permissions only to the deployment job. This does not change repository visibility or configure Pages settings automatically.

If changing the base, update internal `/tt-pane/` content links as well as `astro.config.mjs`.

## Editing

- Pages: `src/content/docs/`
- Theme and responsive layout: `src/styles/theme.css`
- Homepage: `src/components/Hero.astro`
- Shared animated demo: `src/components/Showcase.astro`
- Initial position geometry: `src/data/positions.ts`

The demo never registers desktop shortcuts or moves real windows. Native buttons support keyboard and touch input. Its optional tour pauses on hover, focus, and hidden tabs; reduced-motion preference disables autoplay and transitions. Keep the ten placements and growth behavior aligned with Rust geometry when updating the app.
