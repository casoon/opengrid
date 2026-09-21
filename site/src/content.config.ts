import { docsSchema } from '@casoon/pages-theme/content';
import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';

// Sources: ../docs (docs/ in the project repository). docs/releasing.md is the
// maintainers' release checklist; it stays a repository document.
export const collections = {
  docs: defineCollection({
    loader: glob({ pattern: ['**/*.{md,mdx}', '!releasing.md'], base: '../docs' }),
    schema: docsSchema,
  }),
};
