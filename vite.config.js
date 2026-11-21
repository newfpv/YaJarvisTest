import { fileURLToPath } from "node:url";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

const fromRepo = (path) => fileURLToPath(new URL(path, import.meta.url));

export default defineConfig(({ mode }) => {
  if (mode !== "installer") throw new Error("Use npm run build:installer or npm run build:panel");

  return {
    root: fromRepo("installer/ui"),
    base: "./",
    plugins: [svelte()],
    build: {
      outDir: fromRepo("build/ui/installer"),
      emptyOutDir: true,
      rolldownOptions: {
        // The installer serves this module with the token for the current process.
        external: ["/token.js"],
      },
    },
  };
});
