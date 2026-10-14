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
      version: '0.13.0',
      license: 'MIT OR Apache-2.0',
      // On npm; the Rust crates are not on crates.io yet, so no crates.io or docs.rs links.
      packages: [
        { label: 'npm', href: 'https://www.npmjs.com/package/@casoon/opengrid' },
        { label: 'Live demos', href: 'https://og-vanilla.casoon.dev/' },
      ],
      mdxComponents: './src/mdx-components.ts',
      docsGroups: {
        overview: 'Overview',
        'getting-started': 'Getting started',
        guides: 'Guides',
      },
      // CHANGELOG.md writes `## [x.y.z] — date` (em dash) and has package-prefixed releases;
      // the theme reads `## [x.y.z] - YYYY-MM-DD`. The docs link the file on GitHub instead.
      changelog: false,
      // The examples need the WASM modules in a browser; the demo page runs them live.
      showcase: false,
      demo: 'Demo',
    }),
  ],
});
