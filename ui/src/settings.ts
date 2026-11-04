// settings.html — the settings window, bound to the contract commands.
// In a plain browser it runs against the in-memory demo host, which shares
// its settings with an open demo.html over a BroadcastChannel.

import { createBridge, type Bridge } from "./bridge";
import type { CandidateSummary, Composition, FontChoice, LicenceFile, LinkTarget, MatchMode, NoLyricsBehaviour, ReadingDisplay, Settings, SourceId, SourceSettings } from "./contract";
import { COMMANDS, EVENTS } from "./contract";
import { defaultSettings, SETTING_RANGES, SOURCE_IDS, SOURCE_TIMING, SOURCE_TITLES } from "./defaults";
import { demoHref } from "./links";
import { button, choice, h, percent, section, seconds, select, slider, toggle, type Binder, type Refresher } from "./settings/controls";
import { errorText, lookupStatus, pausedByUser } from "./settings/status";
import { songOffsetKey } from "./sha256";
import { connectStore, Store, type Change } from "./store";
import "./styles/settings.css";
import { arrivalOnly, COMPOSITION_DETAILS, COMPOSITION_TITLES, IMPLEMENTED_COMPOSITIONS, renderableComposition, type ConcreteComposition } from "./timing/composition";
import { VISUALIZER_AVAILABLE } from "./render/visualizer";

type PageId = "wallpaper" | "style" | "lyrics" | "sources" | "readings" | "app" | "about";

const PAGES: { id: PageId; title: string; group: string }[] = [
  { id: "wallpaper", title: "Wallpaper", group: "Appearance" },
  { id: "style", title: "Lyric style", group: "Appearance" },
  { id: "lyrics", title: "This song", group: "Lyrics" },
  { id: "sources", title: "Timing & sources", group: "Lyrics" },
  { id: "readings", title: "Readings", group: "Lyrics" },
  { id: "app", title: "Displays & performance", group: "App" },
  { id: "about", title: "About", group: "App" },
];

const FONT_OPTIONS: [FontChoice, string][] = [
  ["manrope", "Manrope"],
  ["system", "Segoe UI Variable"],
  ["serif", "Serif — Georgia / Sitka"],
  ["slab", "Slab — Rockwell"],
  ["soft", "Soft — Segoe UI Display"],
];

class SettingsApp {
  private readonly refreshers: Refresher[] = [];
  private readonly lookupRefreshers: (() => void)[] = [];
  private readonly renderRefreshers: (() => void)[] = [];
  private readonly binder: Binder;
  private page: PageId = "wallpaper";
  private readonly pages = new Map<PageId, HTMLElement>();
  private readonly navButtons = new Map<PageId, HTMLButtonElement>();
  private readonly status: HTMLElement;

  constructor(private readonly bridge: Bridge, private readonly store: Store) {
    this.binder = {
      patch: (patch) => void this.patch(patch),
      watch: (refresh) => this.refreshers.push(refresh),
      settings: () => this.store.state.settings,
    };
    this.status = h("div", { class: "toast", role: "status", "aria-live": "polite" });
  }

  private async patch(patch: Partial<Settings>): Promise<void> {
    try {
      const result = await this.bridge.invoke<Settings>(COMMANDS.updateSettings, { patch });
      if (result && typeof result === "object") this.store.applyEvent(EVENTS.settings, result);
    } catch (error) {
      this.toast(`Couldn’t save that setting: ${errorText(error)}`);
    }
  }

  private async command(name: string, args?: Record<string, unknown>): Promise<unknown> {
    try {
      return await this.bridge.invoke(name, args);
    } catch (error) {
      this.toast(errorText(error));
      return null;
    }
  }

  /** Runs an action whose outcome is read in `target`: its status string (or `done`), or the error. */
  private async report(target: HTMLElement, done: string, action: () => Promise<unknown>): Promise<void> {
    target.textContent = "";
    try {
      const result = await action();
      target.textContent = typeof result === "string" && result.trim() ? result : done;
    } catch (error) {
      target.textContent = errorText(error);
    }
  }

  private toast(message: string): void {
    this.status.textContent = message;
  }

  mount(root: HTMLElement): void {
    const nav = h("nav", { class: "sidebar", "aria-label": "Settings sections" });
    nav.append(h("div", { class: "brand" }, h("span", { class: "brand-mark", "aria-hidden": "true" }), h("span", { text: "Lunalyr" })));
    let group = "";
    let list: HTMLUListElement | null = null;
    for (const page of PAGES) {
      if (page.group !== group) {
        group = page.group;
        nav.append(h("h3", { text: group }));
        list = h("ul");
        nav.append(list);
      }
      const b = h("button", { type: "button", class: "nav-item", text: page.title, "data-page": page.id });
      b.addEventListener("click", () => this.show(page.id, true));
      this.navButtons.set(page.id, b);
      list!.append(h("li", {}, b));
    }
    nav.addEventListener("keydown", (event) => {
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      event.preventDefault();
      const ids = PAGES.map((p) => p.id);
      const next = ids[(ids.indexOf(this.page) + (event.key === "ArrowDown" ? 1 : ids.length - 1)) % ids.length];
      this.show(next, true);
    });

    const main = h("main", { class: "content", id: "content", tabindex: "-1" });
    const builders: Record<PageId, () => HTMLElement> = {
      wallpaper: () => this.wallpaperPage(),
      style: () => this.stylePage(),
      lyrics: () => this.lyricsPage(),
      sources: () => this.sourcesPage(),
      readings: () => this.readingsPage(),
      app: () => this.appPage(),
      about: () => this.aboutPage(),
    };
    for (const page of PAGES) {
      const el = h("div", { class: "page", id: `page-${page.id}`, role: "region", "aria-label": page.title, hidden: true });
      el.append(h("h1", { text: page.title }), builders[page.id]());
      this.pages.set(page.id, el);
      main.append(el);
    }
    root.append(h("a", { class: "skip", href: "#content", text: "Skip to settings" }), nav, main, this.status);

    const initial = (location.hash.replace("#", "") as PageId) || "wallpaper";
    this.show(this.pages.has(initial) ? initial : "wallpaper", false);
    this.store.subscribe((changed) => this.refresh(changed));
    this.refresh(new Set<Change>(["settings", "lookup", "track", "lyrics", "render"]));
  }

  private show(page: PageId, focus: boolean): void {
    this.page = page;
    for (const [id, el] of this.pages) el.hidden = id !== page;
    for (const [id, b] of this.navButtons) {
      if (id === page) b.setAttribute("aria-current", "page");
      else b.removeAttribute("aria-current");
    }
    history.replaceState(null, "", `#${page}`);
    if (focus) this.navButtons.get(page)?.focus();
  }

  private refresh(changed: ReadonlySet<Change>): void {
    if (changed.has("settings")) for (const r of this.refreshers) r(this.store.state.settings);
    if (changed.has("render")) for (const r of this.renderRefreshers) r();
    if (changed.has("lookup") || changed.has("track") || changed.has("lyrics") || changed.has("settings") || changed.has("version")) {
      for (const r of this.lookupRefreshers) r();
    }
  }

  // MARK: pages

  private wallpaperPage(): HTMLElement {
    const b = this.binder;
    const visual = h("div", { class: "visualizer-options" });
    visual.append(
      select<"wavefield" | "spectrum">(b, "Scene", [["wavefield", "Wavefield"], ["spectrum", "Spectrum"]], (s) => s.visualizer, (v) => ({ visualizer: v })),
      toggle(b, "React to system audio", "audioReactive", "Uses the audio your PC is playing. Nothing is recorded or sent anywhere."),
      slider(b, "Sensitivity", (s) => s.audioSensitivity, (v) => ({ audioSensitivity: v }), { min: SETTING_RANGES.audioSensitivity[0], max: SETTING_RANGES.audioSensitivity[1], step: 0.05, format: (v) => `${v.toFixed(2)}×` }),
    );
    b.watch((s) => {
      visual.hidden = s.background !== "visualizer" || !VISUALIZER_AVAILABLE;
    });
    return h("div", {},
      section("Background",
        choice(b, "Wallpaper type", [
          { value: "colour", label: "Album colours", detail: "Soft colour drifting from the cover of what’s playing." },
          { value: "visualizer", label: "Visualiser", detail: VISUALIZER_AVAILABLE ? "Moves with the music behind the lyrics." : "Coming soon.", disabled: !VISUALIZER_AVAILABLE },
        ], (s) => s.background, (v) => ({ background: v })),
        visual,
      ),
      section("Appearance",
        slider(b, "Motion speed", (s) => s.effectSpeed, (v) => ({ effectSpeed: v }), { min: SETTING_RANGES.effectSpeed[0], max: SETTING_RANGES.effectSpeed[1], step: 0.05, format: (v) => `${v.toFixed(2)}×` }),
        slider(b, "Colour richness", (s) => s.vividness, (v) => ({ vividness: v }), { min: SETTING_RANGES.vividness[0], max: SETTING_RANGES.vividness[1], step: 0.05, format: percent }),
        slider(b, "Darkening", (s) => s.scrimStrength, (v) => ({ scrimStrength: v }), { min: SETTING_RANGES.scrimStrength[0], max: SETTING_RANGES.scrimStrength[1], step: 0.05, format: percent, help: "Shades the top and bottom of the wallpaper so lyrics stay readable." }),
        h("div", { class: "actions" }, button("Reset appearance", () => b.patch({ effectSpeed: 1, vividness: 1, scrimStrength: 1 }))),
      ),
    );
  }

  private stylePage(): HTMLElement {
    const b = this.binder;
    const options = (["automatic", "editorial", "spoken", "constellationEditorial", "constellation", "scattered", "echo", "cascade", "cutout"] as Composition[])
      .map((c) => [c, COMPOSITION_TITLES[c]] as [Composition, string]);
    const detail = h("p", { class: "help" });
    b.watch((s) => {
      let text = COMPOSITION_DETAILS[s.composition];
      if (s.composition !== "automatic" && !IMPLEMENTED_COMPOSITIONS.has(s.composition)) {
        const shown = renderableComposition(s.composition as ConcreteComposition);
        text += ` Shown as ${COMPOSITION_TITLES[shown]} in this version.`;
      }
      detail.textContent = text;
    });
    const rolling = (s: Settings) => s.composition !== "automatic" && arrivalOnly(s.composition as ConcreteComposition);
    const lead = slider(b, "Preview lead", (s) => s.nextWordPreviewLead, (v) => ({ nextWordPreviewLead: v }), {
      min: SETTING_RANGES.nextWordPreviewLead[0], max: SETTING_RANGES.nextWordPreviewLead[1], step: 0.05, format: (v) => seconds(v, false),
      help: "How long before a word is sung its faint preview appears.",
    });
    b.watch((s) => {
      lead.hidden = !s.nextWordPreview;
    });
    return h("div", {},
      section("Composition",
        select<Composition>(b, "Layout", options, (s) => s.composition, (v) => ({ composition: v })),
        detail,
        slider(b, "Lyric size", (s) => s.lyricScale, (v) => ({ lyricScale: v }), { min: SETTING_RANGES.lyricScale[0], max: SETTING_RANGES.lyricScale[1], step: 0.05, format: percent, help: "Lyrics are fitted to the screen first, then scaled by this amount." }),
      ),
      section("Typeface",
        select<FontChoice>(b, "Typeface", FONT_OPTIONS, (s) => s.font, (v) => ({ font: v }), "Editorial layouts pair it with a contrasting face."),
      ),
      section("Word entrances",
        toggle(b, "Reveal sung words", "wordReveal", "Words appear as they are sung. When off, the whole line shows and each word lights up in turn.",
          (s) => (rolling(s) ? "Always on for this layout: its words arrive one at a time." : null)),
        toggle(b, "Faint next-word preview", "nextWordPreview"),
        lead,
      ),
      section("Motion",
        toggle(b, "Show three dots during instrumental breaks", "showInterludes"),
        toggle(b, "Reduce motion", "reduceMotion", "Words arrive without travelling, tilting or blurring. Windows’ “Animation effects” setting is honoured as well."),
      ),
    );
  }

  private lyricsPage(): HTMLElement {
    const b = this.binder;
    const trackBox = h("div", { class: "track-card" });
    const provenance = h("div", { class: "provenance" });
    const pinned = h("div", { class: "pinned" });
    const offsetValue = h("output", { class: "value" });
    const offsetInput = h("input", { type: "range", min: -10, max: 10, step: 0.05, id: "song-offset", "aria-describedby": "song-offset-help" });
    const offsetRow = h("div", { class: "row" },
      h("div", { class: "row-main" }, h("label", { class: "label", for: "song-offset", text: "Song offset" }), h("div", { class: "slider" }, offsetInput, offsetValue)),
      h("div", { class: "actions" },
        button("Earlier", () => this.setSongOffset(this.songOffset() + 0.1)),
        button("Later", () => this.setSongOffset(this.songOffset() - 0.1)),
        button("Reset", () => this.setSongOffset(0)),
      ),
      h("p", { class: "help", id: "song-offset-help", text: "Saved for this recording. Positive values bring lyrics in earlier." }),
    );
    let offsetTimer: ReturnType<typeof setTimeout> | null = null;
    offsetInput.addEventListener("input", () => {
      offsetValue.textContent = seconds(Number(offsetInput.value));
      if (offsetTimer) clearTimeout(offsetTimer);
      offsetTimer = setTimeout(() => this.setSongOffset(Number(offsetInput.value)), 150);
    });
    const candidates = h("ul", { class: "candidates", "aria-label": "Lyrics found for this song" });
    const fileInput = h("input", { type: "file", accept: ".lrc,.ttml,.xml,.txt,text/plain,application/ttml+xml", id: "import-file", class: "visually-hidden" });
    const importLabel = h("label", { for: "import-file", class: "button", tabindex: "0", role: "button", text: "Import LRC or TTML…" });
    importLabel.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        fileInput.click();
      }
    });
    const message = h("p", { class: "help message", role: "status", "aria-live": "polite" });
    fileInput.addEventListener("change", () => {
      const file = fileInput.files?.[0];
      fileInput.value = "";
      if (!file) return;
      if (file.size > 1_000_000) {
        message.textContent = "That file is larger than 1 MB.";
        return;
      }
      void this.report(message, "Imported.", async () => this.bridge.invoke<string>(COMMANDS.importLyrics, { text: await file.text() }));
    });
    // Progress shows from `lookup.searching` (below), so a reload only clears the last result.
    const reload = button("Reload lyrics", () => {
      if (reload.getAttribute("aria-disabled") === "true") return;
      message.textContent = "";
      void this.command(COMMANDS.reloadLyrics);
    });
    const forget = button("Return to automatic selection", () => {
      void this.report(message, "Saved choice forgotten.", () => this.bridge.invoke(COMMANDS.forgetSelection));
    });
    const controls = h("div", { class: "actions" }, reload, importLabel, fileInput);

    this.lookupRefreshers.push(() => {
      const state = this.store.state;
      const track = state.track.track;
      const lookup = lookupStatus(state);
      trackBox.replaceChildren();
      if (!track) {
        trackBox.append(h("p", { class: "muted", text: "Play a song to adjust its timing or choose its lyrics." }));
      } else {
        trackBox.append(
          h("div", { class: "track-title", text: track.title }),
          h("div", { class: "track-artist", text: [track.artist, track.album].filter(Boolean).join(" — ") }),
          h("div", { class: "track-app muted", text: track.sourceApp ? `Playing in ${track.sourceApp.replace(/\.exe$/i, "")}` : "" }),
        );
      }
      offsetRow.hidden = !track;
      reload.disabled = !track;
      // Held while a lookup runs; aria-disabled (not `disabled`) keeps keyboard focus on it.
      if (lookup.searching) reload.setAttribute("aria-disabled", "true");
      else reload.removeAttribute("aria-disabled");
      importLabel.toggleAttribute("aria-disabled", !track);
      fileInput.disabled = !track;
      if (document.activeElement !== offsetInput) {
        offsetInput.value = String(this.songOffset());
        offsetValue.textContent = seconds(this.songOffset());
      }
      // A refresh keeps the current lyrics (and their source) on screen while it searches.
      const lyrics = state.lyrics;
      const shown = lyrics.status === "synced" || lyrics.status === "plain" ? lyrics.provenance : null;
      provenance.replaceChildren();
      if (shown) {
        provenance.append(
          h("div", {}, h("span", { class: "muted", text: "Source " }), h("strong", { text: shown.source })),
          h("div", {}, h("span", { class: "muted", text: "Timing " }), h("strong", { text: shown.precision })),
        );
      }
      if (lookup.note) provenance.append(h("div", { class: "muted", text: lookup.note }));
      if (shown?.precision === "Mixed timing") {
        provenance.append(h("p", { class: "help", text: "Some lines animate word by word. The others appear all at once because they only have a line timestamp." }));
      }
      pinned.replaceChildren();
      if (state.lookup.pinned && track) pinned.append(h("span", { class: "badge", text: "Using your saved choice" }), forget);
      candidates.replaceChildren(...state.lookup.candidates.map((c) => this.candidateItem(c)));
      if (state.lookup.candidates.length === 0 && track) candidates.append(h("li", { class: "muted", text: lookup.emptyMatches }));
    });

    b.watch(() => undefined);
    return h("div", {},
      section("Now playing", trackBox, provenance, pinned, offsetRow),
      section("Lyrics", controls, message, h("h3", { class: "subhead", text: "Matches" }), candidates),
    );
  }

  private candidateItem(c: CandidateSummary): HTMLLIElement {
    const meta = [c.precision, `${c.lineCount} lines`, c.duration ? formatDuration(c.duration) : null, c.hasReadings ? "Pronunciation readings" : null].filter(Boolean).join(" · ");
    const use = button(c.chosen ? "In use" : "Use for this song", () => void this.command(COMMANDS.selectCandidate, { source: c.source }), { disabled: c.chosen });
    use.setAttribute("aria-label", `${c.chosen ? "In use" : "Use"}: ${c.sourceTitle}, ${c.title} by ${c.artist}`);
    return h("li", { class: `candidate${c.chosen ? " chosen" : ""}` },
      h("div", { class: "candidate-text" },
        h("div", { class: "candidate-source", text: c.sourceTitle }),
        h("div", { class: "candidate-title", text: `${c.title} — ${c.artist}` }),
        h("div", { class: "muted small", text: meta }),
      ),
      use,
    );
  }

  private songOffset(): number {
    const track = this.store.state.track.track;
    if (!track) return 0;
    return this.store.state.settings.songOffsets[songOffsetKey(track.recordingKey)] ?? 0;
  }

  private setSongOffset(value: number): void {
    const bounded = Math.round(Math.max(-30, Math.min(30, value)) * 100) / 100;
    void this.command(COMMANDS.setSongOffset, { seconds: bounded });
  }

  private sourcesPage(): HTMLElement {
    const b = this.binder;
    const list = h("ol", { class: "sources", "aria-label": "Lyric sources in priority order" });
    const sources = () => this.store.state.settings.sources;
    const setSources = (next: Partial<SourceSettings>) => b.patch({ sources: { ...sources(), ...next } });
    const render = () => {
      const s = sources();
      const states = new Map(this.store.state.lookup.states);
      const focused = document.activeElement?.getAttribute("data-focus-key");
      list.replaceChildren(...s.order.map((source, index) => {
        const enabled = s.enabled.includes(source);
        const checkId = `source-${source}`;
        const check = h("input", { type: "checkbox", id: checkId, "data-focus-key": `check-${source}` });
        check.checked = enabled;
        check.addEventListener("change", () => setSources({ enabled: check.checked ? [...s.enabled, source] : s.enabled.filter((x) => x !== source) }));
        const up = button("↑", () => move(source, -1), { "aria-label": `Raise ${SOURCE_TITLES[source]}`, disabled: index === 0, "data-focus-key": `up-${source}` });
        const down = button("↓", () => move(source, 1), { "aria-label": `Lower ${SOURCE_TITLES[source]}`, disabled: index === s.order.length - 1, "data-focus-key": `down-${source}` });
        const state = states.get(source);
        const item = h("li", { class: `source${enabled ? "" : " off"}` },
          h("div", { class: "source-main" },
            check,
            h("label", { for: checkId, class: "source-text" },
              h("span", { class: "source-title", text: SOURCE_TITLES[source] }),
              h("span", { class: "muted small", text: SOURCE_TIMING[source] + (state ? ` · ${state}` : "") })),
          ),
          h("div", { class: "source-order" }, up, down),
        );
        return item;
      }));
      if (focused) (list.querySelector(`[data-focus-key="${focused}"]`) as HTMLElement | null)?.focus();
    };
    const move = (source: SourceId, by: number) => {
      const order = [...sources().order];
      const i = order.indexOf(source);
      const j = i + by;
      if (i < 0 || j < 0 || j >= order.length) return;
      [order[i], order[j]] = [order[j], order[i]];
      setSources({ order });
    };
    b.watch(render);
    this.lookupRefreshers.push(render);

    const folder = h("input", { type: "text", id: "local-folder", placeholder: "C:\\Users\\you\\Music\\Lyrics", spellcheck: "false" });
    folder.addEventListener("change", () => setSources({ localFolder: folder.value.trim() }));
    b.watch((s) => {
      if (document.activeElement !== folder) folder.value = s.sources.localFolder;
    });

    const lyricOffset = slider(b, "Global offset", (s) => s.lyricOffset, (v) => ({ lyricOffset: v }), { min: -3, max: 3, step: 0.05, format: (v) => seconds(v), help: "Applies to every song. Positive values bring lyrics in earlier." });

    return h("div", {},
      section("Timing",
        lyricOffset,
        select<NoLyricsBehaviour>(b, "When lyrics are unavailable", [["notice", "Say so briefly"], ["trackName", "Show title and artist"], ["nothing", "Show nothing"]], (s) => s.noLyrics, (v) => ({ noLyrics: v })),
      ),
      section("Selection",
        choice<MatchMode>(b, "Choose lyrics by", [
          { value: "smart", label: "Accuracy first", detail: "The recording match and real timing decide; source order breaks ties." },
          { value: "priority", label: "Source order", detail: "The first enabled source with a match wins." },
        ], (s) => s.sources.mode, (v) => ({ sources: { ...sources(), mode: v } })),
        this.sourceToggle("Prefer real word timing across sources", "preferWordTiming"),
        this.sourceToggle("Prefer LRCLIB with real word timing", "preferWordTimedLrclib"),
        this.sourceToggle("Treat long QQ Music pauses as instrumental", "repairQqGaps"),
        h("div", { class: "actions" }, button("Restore recommended order", () => setSources({ order: [...SOURCE_IDS] }))),
      ),
      section("Sources · priority order",
        list,
        h("p", { class: "help", text: "Online services receive the song’s title, artist and length. Nothing else leaves your PC." }),
      ),
      section("Local lyric files",
        h("div", { class: "row" },
          h("div", { class: "row-main" }, h("label", { class: "label", for: "local-folder", text: "Lyrics folder" }), folder),
          h("p", { class: "help", text: "A folder of exported .lrc, .ttml or .yrc files. Enable “Local lyric files” above to use it." }),
        ),
      ),
    );
  }

  private sourceToggle(label: string, key: "preferWordTiming" | "preferWordTimedLrclib" | "repairQqGaps"): HTMLElement {
    const inputId = `source-opt-${key}`;
    const input = h("input", { type: "checkbox", id: inputId, role: "switch", class: "switch" });
    input.addEventListener("change", () => this.binder.patch({ sources: { ...this.store.state.settings.sources, [key]: input.checked } }));
    this.binder.watch((s) => {
      input.checked = s.sources[key];
    });
    return h("div", { class: "row" }, h("div", { class: "row-main" }, h("label", { class: "label", for: inputId, text: label }), input));
  }

  private readingsPage(): HTMLElement {
    const b = this.binder;
    return h("div", {},
      section("Pronunciation readings",
        choice<ReadingDisplay>(b, "Show readings", [
          { value: "off", label: "Off" },
          { value: "below", label: "Below each word", detail: "Romanisation sits under the original lyric." },
          { value: "replace", label: "Instead of the lyric", detail: "Shows only the reading." },
        ], (s) => s.readings, (v) => ({ readings: v })),
        toggle(b, "Keep tone marks", "toneMarks", "For pinyin and other readings that mark tones."),
        h("p", { class: "help", text: "Readings follow the original timing. Saved readings from your lyric files are used when present; otherwise they are generated on this PC and can miss names or unusual pronunciations." }),
      ),
    );
  }

  private appPage(): HTMLElement {
    const b = this.binder;
    // Follows the render slice: the tray menu and the demo page pause too.
    const pause = h("input", { type: "checkbox", id: "user-pause", role: "switch", class: "switch" });
    const showPause = () => {
      pause.checked = pausedByUser(this.store.state.render);
    };
    this.renderRefreshers.push(showPause);
    pause.addEventListener("change", async () => {
      try {
        await this.bridge.invoke(COMMANDS.setUserPaused, { paused: pause.checked });
      } catch (error) {
        this.toast(errorText(error));
        showPause();
      }
    });
    return h("div", {},
      section("Displays",
        choice<"all" | "primary">(b, "Show lyrics on", [
          { value: "all", label: "Every display" },
          { value: "primary", label: "Main display only" },
        ], (s) => s.displays, (v) => ({ displays: v })),
      ),
      section("Performance",
        select<"15" | "30" | "60">(b, "Frame rate", [["15", "15 fps — economical"], ["30", "30 fps — balanced"], ["60", "60 fps — smooth"]], (s) => String(s.frameRate) as "15" | "30" | "60", (v) => ({ frameRate: Number(v) as 15 | 30 | 60 })),
        toggle(b, "Pause when a window covers the wallpaper", "pauseWhenCovered", "Stops drawing while a full-screen or maximised app hides the desktop."),
        h("div", { class: "row" }, h("div", { class: "row-main" }, h("label", { class: "label", for: "user-pause", text: "Pause the wallpaper now" }), pause)),
      ),
      section("Startup",
        toggle(b, "Launch at login", "launchAtLogin"),
      ),
    );
  }

  private aboutPage(): HTMLElement {
    const version = h("p", { class: "muted" });
    this.lookupRefreshers.push(() => {
      version.textContent = this.store.state.version ? `Version ${this.store.state.version}` : "";
    });
    const link = (text: string, target: LinkTarget) => this.outsideLink(text, target);
    const licence = (text: string, detail: string, file: LicenceFile) =>
      h("li", {}, link(text, `license:${file}`), h("span", { class: "muted", text: ` — ${detail}` }));
    return h("div", {},
      section("Lunalyr for Windows",
        h("p", { text: "Desktop lyrics, in motion." }),
        version,
        h("p", {}, link("Project & downloads", "project")),
      ),
      section("Built on open source",
        h("p", { class: "small" },
          "A port of ", link("Lunalyr for macOS", "mac"), ", itself based on ", link("Verso", "upstream"),
          " by Akki / AkkiCode06. The upstream GPLv3 with Commons Clause licence and attribution are preserved. Lyrics are set in Manrope; the Wavefield visualiser bundles Three.js and postprocessing."),
        h("h3", { class: "subhead", text: "Licences" }),
        h("ul", { class: "licences small", "aria-label": "Licences" },
          licence("Verso licence", "GPLv3 with the Commons Clause", "LICENSE.txt"),
          licence("Third-party notices", "attribution for everything bundled with the app", "THIRD_PARTY_NOTICES.md"),
          licence("Manrope font licence", "SIL Open Font License 1.1", "Manrope-OFL.txt"),
          licence("Wavefield licence", "MIT", "Wavefield-LICENSE.txt"),
          licence("Wavefield web dependency notices", "Three.js (MIT) and postprocessing (zlib)", "Wavefield-Web-Dependencies-LICENSE.txt"),
        ),
      ),
      section("Privacy",
        h("p", { class: "small", text: "Enabled lyric services receive song metadata. Audio analysis and readings stay on this PC." }),
      ),
    );
  }

  /**
   * A link out of the app. A webview cannot open the browser or a file
   * itself, so inside Tauri it asks the Rust side (`open_link`, a fixed set
   * of targets). The browser demo uses plain links, or plain text for files
   * it does not serve.
   */
  private outsideLink(text: string, target: LinkTarget): HTMLElement {
    if (this.bridge.kind === "tauri") return button(text, () => void this.command(COMMANDS.openLink, { target }), { class: "link" });
    const href = demoHref(target);
    return href ? h("a", { href, target: "_blank", rel: "noreferrer", text }) : h("span", { text });
  }
}

function formatDuration(seconds: number): string {
  const s = Math.round(seconds);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

async function main(): Promise<void> {
  document.documentElement.lang = "en";
  document.title = "Lunalyr Settings";
  const bridge = await createBridge(async () => {
    const { DemoHost } = await import("./demo/host");
    return new DemoHost({ scenario: "word", settings: defaultSettings() });
  });
  const store = new Store();
  const app = new SettingsApp(bridge, store);
  app.mount(document.body);
  await connectStore(bridge, store, { events: [EVENTS.snapshot, EVENTS.track, EVENTS.lyrics, EVENTS.settings, EVENTS.lookup, EVENTS.render] });
}

main().catch((error) => console.error("Settings failed to start", error));
