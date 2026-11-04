// demo.html — a browser harness that runs the real wallpaper renderer
// against the in-memory demo host.
//
// Query parameters (all optional), for reproducible screenshots:
//   state       synced | word | line | plain | notFound | idle | loading
//   composition automatic | editorial | spoken | scattered | constellation |
//               constellationEditorial | echo | cascade | cutout
//   t           start time in seconds          paused   1 = playback paused
//   controls    0 = hide the control strip     palette  dusk | ocean | forest | neon | mono | fallback
//   fps         15 | 30 | 60                   scale    lyric size (0.5–2)
//   readings    off | below | replace          font     manrope | system | serif | slab | soft
//   reveal      0/1 word reveal                preview  0/1 next-word preview
//   interludes  0/1                            reduced  1 = reduce motion
//   noLyrics    notice | trackName | nothing   hold     1 = keep notices up
//   snapshot    URL of a JSON Snapshot to render instead of the samples
//   share       0 = don't sync settings with other open demo/settings pages
// Example: demo.html?state=synced&composition=spoken&t=12.5&paused=1&controls=0

import type { Composition, FontChoice, NoLyricsBehaviour, ReadingDisplay, Settings, Snapshot } from "./contract";
import { DemoHost, type DemoScenario } from "./demo/host";
import { PALETTES } from "./demo/samples";
import { el } from "./render/dom";
import { preloadFonts } from "./render/preload";
import { WallpaperRenderer } from "./render/renderer";
import { connectStore, Store } from "./store";
import "./styles/demo.css";
import { COMPOSITION_TITLES } from "./timing/composition";

const params = new URLSearchParams(location.search);

function flag(name: string): boolean | undefined {
  const v = params.get(name);
  if (v === null) return undefined;
  return v === "1" || v === "true" || v === "yes";
}

function number(name: string): number | undefined {
  const v = params.get(name);
  if (v === null || v.trim() === "") return undefined;
  const n = Number(v);
  return Number.isFinite(n) ? n : undefined;
}

const SCENARIO_ALIASES: Record<string, DemoScenario> = {
  synced: "word", word: "word", wordtimed: "word", line: "line", linetimed: "line", linesynced: "line",
  plain: "plain", notfound: "notFound", idle: "idle", loading: "loading",
};

const SCENARIO_LABELS: Record<Exclude<DemoScenario, "snapshot">, string> = {
  word: "Word-timed song",
  line: "Line-timed song",
  plain: "Plain lyrics",
  notFound: "No lyrics found",
  idle: "Nothing playing",
  loading: "Loading",
};

function settingsFromParams(): Partial<Settings> {
  const s: Partial<Settings> = {};
  const composition = params.get("composition");
  if (composition && composition in COMPOSITION_TITLES) s.composition = composition as Composition;
  const fps = number("fps");
  if (fps === 15 || fps === 30 || fps === 60) s.frameRate = fps;
  const scale = number("scale");
  if (scale !== undefined) s.lyricScale = Math.min(2, Math.max(0.5, scale));
  const readings = params.get("readings");
  if (readings === "off" || readings === "below" || readings === "replace") s.readings = readings;
  const font = params.get("font");
  if (font && ["manrope", "system", "serif", "slab", "soft"].includes(font)) s.font = font as FontChoice;
  const reveal = flag("reveal");
  if (reveal !== undefined) s.wordReveal = reveal;
  const preview = flag("preview");
  if (preview !== undefined) s.nextWordPreview = preview;
  const interludes = flag("interludes");
  if (interludes !== undefined) s.showInterludes = interludes;
  const reduced = flag("reduced");
  if (reduced !== undefined) s.reduceMotion = reduced;
  const noLyrics = params.get("noLyrics");
  if (noLyrics === "notice" || noLyrics === "trackName" || noLyrics === "nothing") s.noLyrics = noLyrics;
  const vividness = number("vividness");
  if (vividness !== undefined) s.vividness = vividness;
  const scrim = number("scrim");
  if (scrim !== undefined) s.scrimStrength = scrim;
  return s;
}

async function main(): Promise<void> {
  const scenario = SCENARIO_ALIASES[(params.get("state") ?? "word").toLowerCase()] ?? "word";
  const paused = flag("paused") ?? false;
  const time = number("t");
  const urlSettings = settingsFromParams();
  const host = new DemoHost({
    scenario,
    time,
    paused,
    palette: params.get("palette") ?? "dusk",
    // Explicit query settings win over settings shared from other pages.
    share: flag("share") ?? Object.keys(urlSettings).length === 0,
    settings: urlSettings,
    holdNotices: flag("hold") ?? false,
    // A page opened at an explicit state or time keeps it.
    follow: !params.has("state") && !params.has("t") && !params.has("snapshot"),
  });

  const snapshotUrl = params.get("snapshot");
  if (snapshotUrl) {
    try {
      const response = await fetch(snapshotUrl);
      if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
      const snapshot = (await response.json()) as Snapshot;
      host.loadSnapshot(snapshot, { time, paused: flag("paused") });
      if (Object.keys(urlSettings).length) host.patchSettings(urlSettings);
    } catch (error) {
      console.error(`Could not load snapshot ${snapshotUrl}`, error);
      showError(`Could not load snapshot ${snapshotUrl}: ${String(error)}`);
    }
  }

  const store = new Store();
  // Screenshots start the ambient field at a fixed phase.
  const renderer = new WallpaperRenderer(document.body, store, { ambientPhase: 0 });
  await preloadFonts();
  renderer.start();
  await connectStore(host, store, { onAudio: (frame) => renderer.pushAudio(frame) });
  if (flag("controls") !== false) buildControls(host, store);
  Object.assign(window as unknown as Record<string, unknown>, { lunalyrDemo: { host, store, renderer } });
}

function showError(message: string): void {
  const box = el("div", "demo-error", document.body);
  box.textContent = message;
}

function formatTime(t: number): string {
  const s = Math.max(0, t);
  return `${Math.floor(s / 60)}:${(s % 60).toFixed(1).padStart(4, "0")}`;
}

function select(label: string, options: [string, string][], value: string, onChange: (v: string) => void, parent: HTMLElement): HTMLSelectElement {
  const wrap = el("label", "demo-field", parent);
  el("span", "", wrap).textContent = label;
  const s = el("select", "", wrap);
  for (const [v, text] of options) {
    const o = el("option", "", s);
    o.value = v;
    o.textContent = text;
  }
  s.value = value;
  s.addEventListener("change", () => onChange(s.value));
  return s;
}

function toggle(label: string, value: boolean, onChange: (v: boolean) => void, parent: HTMLElement): HTMLInputElement {
  const wrap = el("label", "demo-check", parent);
  const input = el("input", "", wrap);
  input.type = "checkbox";
  input.checked = value;
  el("span", "", wrap).textContent = label;
  input.addEventListener("change", () => onChange(input.checked));
  return input;
}

function buildControls(host: DemoHost, store: Store): void {
  const bar = el("div", "demo-bar", document.body);
  bar.setAttribute("role", "toolbar");
  bar.setAttribute("aria-label", "Demo controls");
  const settings = () => store.state.settings;
  const patch = (p: Partial<Settings>) => host.patchSettings(p);

  const row1 = el("div", "demo-row", bar);
  const play = el("button", "demo-play", row1);
  play.type = "button";
  const scrub = el("input", "demo-scrub", row1);
  scrub.type = "range";
  scrub.min = "0";
  scrub.step = "0.05";
  scrub.setAttribute("aria-label", "Playback position");
  const clock = el("span", "demo-time", row1);
  const hide = el("button", "demo-hide", row1);
  hide.type = "button";
  hide.textContent = "Hide (H)";
  hide.title = "Hide the controls; press H to bring them back";

  const row2 = el("div", "demo-row", bar);
  const scenarios = Object.entries(SCENARIO_LABELS) as [string, string][];
  const scenarioSelect = select("State", scenarios, host.currentScenario === "snapshot" ? "word" : host.currentScenario, (v) => {
    host.setScenario(v as DemoScenario, { time: 0 });
    refresh();
  }, row2);
  select("Composition", Object.entries(COMPOSITION_TITLES), settings().composition, (v) => patch({ composition: v as Composition }), row2);
  select("Palette", Object.entries(PALETTES).map(([k, p]) => [k, p.label]), params.get("palette") ?? "dusk", (v) => host.setPalette(v), row2);
  select("Frame rate", [["15", "15 fps"], ["30", "30 fps"], ["60", "60 fps"]], String(settings().frameRate), (v) => patch({ frameRate: Number(v) as 15 | 30 | 60 }), row2);
  select("Readings", [["off", "Off"], ["below", "Below"], ["replace", "Replace"]], settings().readings, (v) => patch({ readings: v as ReadingDisplay }), row2);
  select("Font", [["manrope", "Manrope"], ["system", "Segoe UI"], ["serif", "Serif"], ["slab", "Slab"], ["soft", "Soft"]], settings().font, (v) => patch({ font: v as FontChoice }), row2);
  select("No lyrics", [["notice", "Say so briefly"], ["trackName", "Title and artist"], ["nothing", "Nothing"]], settings().noLyrics, (v) => patch({ noLyrics: v as NoLyricsBehaviour }), row2);

  const row3 = el("div", "demo-row", bar);
  const sizeWrap = el("label", "demo-field", row3);
  el("span", "", sizeWrap).textContent = "Lyric size";
  const size = el("input", "", sizeWrap);
  size.type = "range";
  size.min = "0.5";
  size.max = "2";
  size.step = "0.05";
  size.value = String(settings().lyricScale);
  size.addEventListener("input", () => patch({ lyricScale: Number(size.value) }));
  toggle("Word reveal", settings().wordReveal, (v) => patch({ wordReveal: v }), row3);
  toggle("Next-word preview", settings().nextWordPreview, (v) => patch({ nextWordPreview: v }), row3);
  toggle("Interludes", settings().showInterludes, (v) => patch({ showInterludes: v }), row3);
  toggle("Reduce motion", settings().reduceMotion, (v) => patch({ reduceMotion: v }), row3);
  toggle("Pause wallpaper", store.state.render.paused, (v) => host.setUserPaused(v), row3);
  const settingsLink = el("a", "demo-link", row3);
  settingsLink.href = "./settings.html";
  settingsLink.target = "lunalyr-settings";
  settingsLink.textContent = "Open settings ↗";

  let timer: ReturnType<typeof setInterval> | null = null;
  const tick = () => {
    const t = host.position();
    if (document.activeElement !== scrub) scrub.value = String(t);
    clock.textContent = `${formatTime(t)} / ${formatTime(host.duration)}`;
  };
  const refresh = () => {
    scrub.max = String(host.duration);
    play.textContent = host.playing ? "Pause" : "Play";
    play.setAttribute("aria-pressed", String(!host.playing));
    scenarioSelect.value = host.currentScenario === "snapshot" ? scenarioSelect.value : host.currentScenario;
    tick();
    if (timer) clearInterval(timer);
    timer = host.playing && !bar.hidden ? setInterval(tick, 200) : null;
  };
  play.addEventListener("click", () => {
    if (host.playing) host.pause();
    else host.play();
    refresh();
  });
  scrub.addEventListener("input", () => {
    host.seek(Number(scrub.value));
    tick();
  });
  const setHidden = (hidden: boolean) => {
    bar.hidden = hidden;
    refresh();
  };
  hide.addEventListener("click", () => setHidden(true));
  window.addEventListener("keydown", (event) => {
    if (event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement) {
      if (event.key !== "Escape") return;
    }
    if (event.key === "h" || event.key === "H") setHidden(!bar.hidden);
    else if (event.key === " ") {
      event.preventDefault();
      play.click();
    } else if (event.key === "ArrowLeft") host.seek(host.position() - 2);
    else if (event.key === "ArrowRight") host.seek(host.position() + 2);
    else return;
    tick();
  });
  store.subscribe((changed) => {
    if (changed.has("clock")) refresh();
  });
  refresh();
}

main().catch((error) => {
  console.error(error);
  showError(String(error));
});
