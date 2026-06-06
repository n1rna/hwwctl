// Astro content collection config.
//
// The docs collection deliberately loads from `../docs/` (the
// repo-root docs directory) rather than the Starlight default
// `src/content/docs/`. That keeps a single source of truth: every
// page on the website corresponds 1:1 to a markdown file you can
// read on GitHub. Editing on either side updates the same file.

import { glob } from "astro/loaders";
import { defineCollection } from "astro:content";
import { docsLoader } from "@astrojs/starlight/loaders";
import { docsSchema } from "@astrojs/starlight/schema";

// Disable docsLoader's default behavior; we point at our own path.
// The `loader` returned by `docsLoader()` itself just wraps the
// `glob` loader against `src/content/docs/`, so swapping in our own
// `glob` is equivalent — we get all the Starlight content-layer
// plumbing (the `docsSchema()` validation, ID generation, etc.) for
// free.
void docsLoader;

export const collections = {
	docs: defineCollection({
		loader: glob({
			pattern: "**/*.{md,mdx}",
			base: "../docs",
		}),
		schema: docsSchema(),
	}),
};
