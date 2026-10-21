import { resolve } from "node:path";
import { readFileSync, statSync } from "node:fs";
import { describe, expect, it } from "vitest";
const css = readFileSync(resolve("src/styles.css"), "utf8");
function tokens(block: string) {
  return Object.fromEntries(
    [...block.matchAll(/--([\w-]+):\s*(#[A-Fa-f0-9]{6})\s*;/g)].map((m) => [
      m[1],
      m[2].toUpperCase(),
    ]),
  );
}
function luminance(hex: string) {
  const channels = [1, 3, 5]
    .map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
}
function contrast(a: string, b: string) {
  const values = [luminance(a), luminance(b)].sort((a, b) => b - a);
  return (values[0] + 0.05) / (values[1] + 0.05);
}
describe("Instrument foundations", () => {
  const dark = tokens(
    css.match(/:root,\s*:root\[data-theme=["']dark["']\]\s*\{([\s\S]*?)\}/)![1],
  );
  const light = tokens(
    css.match(/:root\[data-theme=["']light["']\]\s*\{([\s\S]*?)\}/)![1],
  );
  it.each([
    ["dark", dark],
    ["light", light],
  ] as const)(
    "keeps informative text and semantic labels at AA contrast in %s",
    (_, t) => {
      for (const surface of [
        "ground",
        "well",
        "surface-1",
        "surface-2",
        "surface-3",
      ])
        for (const text of [
          "text-1",
          "text-2",
          ...(surface === "surface-3" ? [] : ["text-3"]),
        ])
          expect(
            contrast(t[text], t[surface]),
            `${text} on ${surface}`,
          ).toBeGreaterThanOrEqual(4.5);
      for (const role of ["live", "attention", "success", "danger"]) {
        expect(contrast(t[role], t["surface-1"]), role).toBeGreaterThanOrEqual(
          4.5,
        );
        expect(
          contrast(t[role], t[`${role}-tint`]),
          `${role} tint`,
        ).toBeGreaterThanOrEqual(4.5);
      }
    },
  );
  it("pins the approved palette and reserves running color for live execution", () => {
    expect([
      dark.live,
      dark.attention,
      dark.success,
      dark.danger,
      dark.neutral,
    ]).toEqual(["#9FADF6", "#FFBB7B", "#8FDABD", "#EC8697", "#83868B"]);
    expect([light.live, light.attention, light.success, light.danger]).toEqual([
      "#5460B3",
      "#9E5711",
      "#22765C",
      "#B03D58",
    ]);
    expect(css).toMatch(/--seg-coding:\s*rgb\(255 255 255\s*\/\s*0?\.14\)/);
    expect(css).not.toMatch(/backdrop-filter|feTurbulence/);
    expect(css).toMatch(/prefers-contrast:\s*more/);
  });
  it("bundles deterministic PNG grain and licensed fonts within the 1 MB asset budget", () => {
    const files = [
      "assets/grain-256.png",
      "assets/fonts/inter-latin-standard-normal.woff2",
      "assets/fonts/jetbrains-mono-latin-wght-normal.woff2",
    ];
    expect(
      files.reduce((n, path) => n + statSync(resolve("src", path)).size, 0),
    ).toBeLessThan(1_000_000);
    const png = readFileSync(resolve("src/assets/grain-256.png"));
    expect(png.subarray(1, 4).toString()).toBe("PNG");
    expect(png.readUInt32BE(16)).toBe(256);
    expect(png.readUInt32BE(20)).toBe(256);
    for (const font of ["inter", "jetbrains-mono"])
      expect(
        readFileSync(resolve(`src/assets/fonts/${font}-OFL.txt`), "utf8"),
      ).toContain("SIL OPEN FONT LICENSE");
  });
});
