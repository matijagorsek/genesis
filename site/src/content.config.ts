import { defineCollection, z } from 'astro:content';
import { glob } from 'astro/loaders';

// The manual: twelve sections in the order things happen, as Markdown with the same HTML inside.
const manual = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/manual' }),
  schema: z.object({ title: z.string(), number: z.number(), slug: z.string() }),
});

// The decision log, copied in from docs/ before every build (scripts/prebuild.mjs).
const decisions = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/decisions' }),
  schema: z.object({}).passthrough(),
});

export const collections = { manual, decisions };
