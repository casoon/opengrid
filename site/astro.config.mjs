// @ts-check
import casoonPages from '@casoon/pages-theme';
import { defineConfig } from 'astro/config';

// Project page: https://casoon.github.io/opengrid/ — `base` is the GitHub Pages path.
export default defineConfig({
  site: 'https://casoon.github.io/opengrid',
  base: '/opengrid/',
  integrations: [
    casoonPages({
      name: 'opengrid',
      description:
        'A portable Rust/WASM data and query engine, with an accessible data grid, table and pivot delivered as Web Components.',
      repo: 'casoon/opengrid',
      version: '0.1.0',
      license: 'MIT OR Apache-2.0',
      // Not published yet: no crates.io, npm or docs.rs links until a release exists.
      packages: [],
      docsGroups: {
        overview: 'Overview',
        'getting-started': 'Getting started',
        guides: 'Guides',
      },
      // CHANGELOG.md has no release yet, only an empty Unreleased section.
      changelog: false,
      // The examples need the WASM modules in a browser; the demo page runs them live.
      showcase: false,
      demo: 'Demo',
    }),
  ],
});
