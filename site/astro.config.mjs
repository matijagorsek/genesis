// @ts-check
import { defineConfig } from 'astro/config';

// Static output, served from GitHub Pages at the repository path. The screenshots live in docs/screens,
// where CI writes them after every release; the build copies them in (see scripts/prebuild.mjs).
export default defineConfig({
  site: 'https://matijagorsek.github.io',
  base: '/genesis',
  output: 'static',
  trailingSlash: 'ignore',
  build: { format: 'file', assets: 'assets' },
});
