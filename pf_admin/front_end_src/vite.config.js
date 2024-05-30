import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import { viteStaticCopy } from 'vite-plugin-static-copy';

export default defineConfig({
	plugins: [
		sveltekit(),
		viteStaticCopy({
			targets: [
				{ src: 'node_modules/svelte-ace/dist/index.js', dest: './', rename: () => {return "svelte-ace.js"}},
				{ src: 'node_modules/brace/theme/gruvbox.js', dest: './', rename: () => {return "theme-gruvbox.js"} }
			]
		})
	]
});
