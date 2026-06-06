// @ts-check
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import starlightThemeFlexoki from "starlight-theme-flexoki";
import tailwindcss from "@tailwindcss/vite";

// https://astro.build/config
export default defineConfig({
	site: "https://hwwctl.n1rna.net",
	integrations: [
		starlight({
			title: "hwwctl",
			description:
				"Control plane for hardware-wallet emulators. Drive Trezor / BitBox02 / Coldcard / Specter / Ledger / Jade from end-to-end tests.",
			social: [
				{
					icon: "github",
					label: "GitHub",
					href: "https://github.com/n1rna/hwwctl",
				},
			],
			plugins: [starlightThemeFlexoki()],
			editLink: {
				baseUrl: "https://github.com/n1rna/hwwctl/edit/main/",
			},
			sidebar: [
				{
					label: "Getting started",
					items: [
						{ label: "Introduction", slug: "" },
						{ label: "Quick start", slug: "quick-start" },
					],
				},
				{
					label: "Reference",
					items: [
						{ label: "CLI", slug: "cli" },
						{ label: "Architecture", slug: "architecture" },
						{ label: "Wallets", slug: "wallets" },
					],
				},
				{
					label: "Development",
					items: [
						{ label: "Building & testing", slug: "development" },
						{ label: "Roadmap", slug: "todo" },
					],
				},
			],
		}),
	],
	vite: {
		plugins: [tailwindcss()],
	},
});
