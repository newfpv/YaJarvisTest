import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import adapter from "@sveltejs/adapter-static";
import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

const output = fileURLToPath(new URL("../../build/ui/panel", import.meta.url));
const staticPanel = adapter({ pages: output, assets: output, fallback: "index.html" });

async function adaptPanel(builder) {
  await staticPanel.adapt(builder);
  const index = join(output, "index.html");
  const html = await readFile(index, "utf8");
  const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)];
  if (scripts.length !== 1) throw new Error("Expected one SvelteKit bootstrap script in the panel HTML");

  // The embedded server permits same-origin scripts. Keep the bootstrap in an external asset.
  const [[tag, bootstrap]] = scripts;
  const hash = createHash("sha256").update(bootstrap).digest("hex").slice(0, 16);
  const asset = `_app/immutable/entry/panel.${hash}.js`;
  await writeFile(join(output, asset), bootstrap);
  await writeFile(index, html.replace(tag, `<script src="/${asset}"></script>`));
}

export default defineConfig({
  plugins: [sveltekit({
    adapter: { ...staticPanel, adapt: adaptPanel },
    outDir: fileURLToPath(new URL("../../build/ui/panel-kit", import.meta.url)),
  })],
});
