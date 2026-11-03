# sTile documentation

Astro + Starlight, with an animated position playground and coordinated light/dark themes. The structure follows tt-git's custom hero + standard documentation approach. Product behavior is sourced from the root README, `src/config.rs`, and `crates/stile-platform/src/geometry.rs`.

Requires Node.js 22.12+ (Node 24 recommended).

```sh
cd docs-site
npm ci
npm run dev
npm run check
npm run build
npm run preview
```

Open the printed local URL under `/sTile/`. Production output is `docs-site/dist/`. Search is indexed by Pagefind during the production build; use preview to test it.

The configuration targets `https://jsheely.github.io/sTile/`, but this change does not enable hosting or publish the site. Set GitHub Pages to GitHub Actions and add a deployment workflow when ready. If changing the base, update internal `/sTile/` content links as well as `astro.config.mjs`.

## Editing

- Pages: `src/content/docs/`
- Theme and responsive layout: `src/styles/theme.css`
- Homepage: `src/components/Hero.astro`
- Shared animated demo: `src/components/Showcase.astro`
- Initial position geometry: `src/data/positions.ts`

The demo never registers desktop shortcuts or moves real windows. Native buttons support keyboard and touch input. Its optional tour pauses on hover, focus, and hidden tabs; reduced-motion preference disables autoplay and transitions. Keep the ten placements and growth behavior aligned with Rust geometry when updating the app.
