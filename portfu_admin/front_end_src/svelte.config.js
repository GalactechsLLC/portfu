import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";
import adapter from "@sveltejs/adapter-static";

export default {
  kit: {
    adapter: adapter({
      pages: "../front_end_dist/pf-admin",
      assets: "../front_end_dist/pf-admin",
      fallback: "index.html",
      precompress: false,
      strict: true,
    }),
    paths: {
      base: '/pf-admin',
      assets: ''
    },
  },
  preprocess: [vitePreprocess({})],
};
