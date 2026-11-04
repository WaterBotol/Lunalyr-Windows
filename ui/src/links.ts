// Where the fixed `open_link` targets lead (mirrors `open_link` in
// src-tauri/src/lib.rs). Inside the app the Rust side opens them, because a
// webview cannot open a browser or a file itself; the browser demo links to
// the same pages directly.

import type { LicenceFile, LinkTarget } from "./contract";

export const LINK_URLS = {
  project: "https://github.com/WaterBotol/Lunalyr-Windows",
  mac: "https://github.com/WaterBotol/Lunalyr",
  upstream: "https://github.com/AkkiCode06/verso",
} as const;

/**
 * Licence files the browser demo serves from ./licenses/ (ui/public/licenses),
 * by installed name. LICENSE.txt is served under its upstream name;
 * THIRD_PARTY_NOTICES.md is installed with the app only.
 */
export const DEMO_LICENCE_FILES: Partial<Record<LicenceFile, string>> = {
  "LICENSE.txt": "Lunalyr-Verso-LICENSE.txt",
  "Manrope-OFL.txt": "Manrope-OFL.txt",
  "Wavefield-LICENSE.txt": "Wavefield-LICENSE.txt",
  "Wavefield-Web-Dependencies-LICENSE.txt": "Wavefield-Web-Dependencies-LICENSE.txt",
};

/** The web page a target opens, or null for licence files and unknown targets. */
export function linkUrl(target: string): string | null {
  return Object.hasOwn(LINK_URLS, target) ? LINK_URLS[target as keyof typeof LINK_URLS] : null;
}

/** What the browser demo links to instead of calling `open_link`, or null when it has no copy. */
export function demoHref(target: LinkTarget): string | null {
  if (target.startsWith("license:")) {
    const file = DEMO_LICENCE_FILES[target.slice("license:".length) as LicenceFile];
    return file ? `./licenses/${file}` : null;
  }
  return linkUrl(target);
}
