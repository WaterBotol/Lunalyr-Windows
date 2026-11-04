import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { DEMO_LICENCE_FILES, demoHref, LINK_URLS, linkUrl } from "./links";

const publicDir = resolve(dirname(fileURLToPath(import.meta.url)), "../public");

describe("outside links", () => {
  it("names the project pages open_link opens", () => {
    expect(linkUrl("project")).toBe("https://github.com/WaterBotol/Lunalyr-Windows");
    expect(linkUrl("mac")).toBe("https://github.com/WaterBotol/Lunalyr");
    expect(linkUrl("upstream")).toBe("https://github.com/AkkiCode06/verso");
    expect(linkUrl("license:LICENSE.txt")).toBeNull();
    expect(linkUrl("toString")).toBeNull();
    for (const url of Object.values(LINK_URLS)) expect(url).toMatch(/^https:\/\//);
  });

  it("links the browser demo only to licence files it serves", () => {
    expect(demoHref("project")).toBe(LINK_URLS.project);
    expect(demoHref("license:LICENSE.txt")).toBe("./licenses/Lunalyr-Verso-LICENSE.txt");
    expect(demoHref("license:Manrope-OFL.txt")).toBe("./licenses/Manrope-OFL.txt");
    expect(demoHref("license:THIRD_PARTY_NOTICES.md")).toBeNull();
    for (const file of Object.values(DEMO_LICENCE_FILES)) expect(existsSync(resolve(publicDir, "licenses", file!))).toBe(true);
  });
});
