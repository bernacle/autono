/* global console, setTimeout, URL */
// Exercise the emitted ES modules, not Vite's development transform. The native
// CSP permits scripts from self but deliberately allows connections only to IPC.
import { readFile } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import { JSDOM } from "jsdom";
const root = fileURLToPath(new URL("../dist/", import.meta.url));
const html = await readFile(resolve(root, "index.html"), "utf8");
const dom = new JSDOM(html, {
  url: "http://desktop.invalid/",
  runScripts: "outside-only",
  pretendToBeVisual: true,
});
dom.window.matchMedia = () => ({
  matches: false,
  addEventListener() {},
  removeEventListener() {},
});
dom.window.fetch = () => {
  throw new Error("Bootstrap must not depend on fetch under the IPC-only CSP");
};
const context = dom.getInternalVMContext(),
  modules = new Map();
async function load(path) {
  if (modules.has(path)) return modules.get(path);
  const m = new vm.SourceTextModule(await readFile(path, "utf8"), {
    context,
    identifier: path,
    initializeImportMeta(meta) {
      meta.url = `http://desktop.invalid/assets/${path.split("/").at(-1)}`;
    },
  });
  modules.set(path, m);
  await m.link((specifier, from) =>
    load(resolve(dirname(from.identifier), specifier)),
  );
  return m;
}
try {
  const entry = /type="module"[^>]+src="([^"]+)"/.exec(html)?.[1];
  if (!entry) throw new Error("Missing production module entry");
  const main = await load(resolve(root, `.${entry}`));
  await main.evaluate();
  await new Promise((done) => setTimeout(done, 100));
  const text = dom.window.document.body.textContent;
  if (
    !text.includes("Overview") ||
    !text.includes("Daemon connection unavailable")
  )
    throw new Error("Production view did not render its disconnected state");
  console.info(
    "Production modules render with fetch disabled; native IPC unavailable as expected.",
  );
} finally {
  dom.window.close();
}
