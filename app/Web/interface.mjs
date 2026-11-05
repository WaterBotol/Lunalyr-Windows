import { arrivals, compositions, fonts, highlights, modes, pages, quality, sourceNames, textColours, visualisers, welcome } from './catalog.mjs';
import { lyricCard } from './card.mjs';
import { cueAt } from './score.mjs';
import { effectNames } from './gpu.mjs';
import { demo, position, Stage } from './stage.mjs';

const surface = new URLSearchParams(location.search).get('surface') ?? 'settings';
const app = document.querySelector('#app');
const host = window.chrome?.webview;
const send = (type, value) => host?.postMessage({ type, value });
const state = { preferences: null, playback: {}, players: [], lyrics: {}, syncedAt: 0, status: {}, covered: false };
let page = 'wallpaper';
let step = 0;
let stage;
let inspector;
let notice;
let previewHidden = false;
let cardBusy = false;
let presetList = [['default', 'Default']];
let controls = [];
let live = [];

document.body.dataset.surface = surface;
host?.addEventListener('message', event => {
  const { type, value } = event.data;
  if (type === 'preferences') {
    const fresh = !state.preferences;
    const previous = state.preferences;
    state.preferences = value;
    document.documentElement.dataset.theme = value.theme;
    if (fresh) mount();
    else {
      if (['background', 'visualiser', 'composition', 'textColour', 'highlight', 'lyricsFolder', 'sources']
        .some(key => JSON.stringify(previous[key]) !== JSON.stringify(value[key]))) renderInspector();
      refresh();
    }
  } else if (type === 'playback') {
    state.playback = value.playback;
    state.syncedAt = performance.now();
    const changed = JSON.stringify(state.players) !== JSON.stringify(value.players);
    state.players = value.players;
    if (changed && page === 'general') renderInspector();
    refresh();
  } else if (type === 'lyrics') {
    const changed = JSON.stringify(state.lyrics.sources) !== JSON.stringify(value.sources) || state.lyrics.saved !== value.saved;
    state.lyrics = value;
    if (changed && page === 'timing') renderInspector();
    refresh();
  } else if (type === 'status') {
    state.status = value;
    refresh();
  } else if (type === 'audio') state.audio = value;
  else if (type === 'covered') state.covered = value;
  else if (type === 'page') go(value);
  else if (type === 'notice') showNotice(value);
});
send('ready');

function element(tag, options = {}, ...children) {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(options)) {
    if (name === 'class') node.className = value;
    else if (name === 'click') node.addEventListener('click', value);
    else if (name === 'text') node.textContent = value;
    else node.setAttribute(name, value);
  }
  for (const item of children.flat(Infinity)) if (item != null && typeof item !== 'boolean') node.append(item);
  return node;
}
const button = (label, click, style = '') => element('button', { type: 'button', class: style, click }, label);
const text = (label, style = '') => element('span', { class: style }, label);
const preference = (key, value) => change({ [key]: value });
function change(values) {
  state.preferences = { ...state.preferences, ...values };
  send('preferences', state.preferences);
  document.documentElement.dataset.theme = state.preferences.theme;
  refresh();
}
function showNotice(value) {
  if (notice) { notice.textContent = value; notice.hidden = !value; }
}
function liveText(get, style = '') {
  const label = text('', style);
  const update = () => { if (label.isConnected) label.textContent = get() ?? ''; };
  update.node = label;
  live.push(update);
  return label;
}
function refresh() {
  for (const update of controls) update();
  for (const update of live) update();
}
function clearBindings() {
  controls = controls.filter(update => update.node.isConnected);
  live = live.filter(update => !update.node || update.node.isConnected);
}
function bind(node, update) {
  update.node = node;
  controls.push(update);
  update();
}
function field(label, input, note) {
  return element('label', { class: 'field' }, element('span', {}, label, note && text(note, 'caption')), input);
}
function toggle(label, key, note, accessor) {
  const input = element('input', { type: 'checkbox', role: 'switch', 'aria-label': label });
  const get = accessor?.get ?? (() => !!state.preferences[key]);
  const set = accessor?.set ?? (value => preference(key, value));
  bind(input, () => { input.checked = get(); });
  input.addEventListener('change', () => set(input.checked));
  return field(label, input, note);
}
function select(label, key, items, access) {
  const input = element('select', { 'aria-label': label });
  for (const [value, name] of items) input.append(element('option', { value }, name));
  const get = access?.get ?? (() => state.preferences[key]);
  const set = access?.set ?? (value => preference(key, value));
  bind(input, () => { if (input.value !== String(get())) input.value = String(get()); });
  input.addEventListener('change', () => set(input.value));
  return field(label, input);
}
function range(label, key, min, max, increment, format, access) {
  const input = element('input', { type: 'range', min, max, step: increment, 'aria-label': label });
  const output = element('output');
  const get = access?.get ?? (() => state.preferences[key]);
  const set = access?.set ?? (value => preference(key, value));
  bind(input, () => {
    if (document.activeElement !== input || !input.matches(':active')) input.value = get();
    output.textContent = format(Number(input.value));
  });
  input.addEventListener('input', () => { output.textContent = format(Number(input.value)); });
  input.addEventListener('change', () => set(Number(input.value)));
  return field(label, element('span', { class: 'range' }, input, output));
}
function colour(label, key) {
  const input = element('input', { type: 'color', 'aria-label': label });
  bind(input, () => { input.value = state.preferences[key]; });
  input.addEventListener('change', () => preference(key, input.value));
  return field(label, input);
}
const pct = value => `${Math.round(value * 100)}%`;
const multiple = value => `${value.toFixed(1)}×`;
const seconds = value => `${value > 0 ? '+' : ''}${value.toFixed(2)} s`;
const caption = message => element('p', { class: 'caption group-note' }, message);
function group(title, ...children) {
  return element('section', { class: 'group' },
    title && element('h2', {}, title), element('div', { class: 'card' }, children));
}
function action(label, command, value) {
  return button(label, () => send(command, value), 'secondary');
}
function modeGrid() {
  const panel = element('div', { class: 'mode-grid' });
  for (const [id, title, icon] of modes) {
    const choice = button([text(icon, 'glyph'), text(title)], () => {
      if (id === 'desktop') return;
      if (id === 'custom' && !state.preferences.wallpaper) send('wallpaper');
      else { preference('background', id); renderInspector(); }
    }, id === 'desktop' || id === 'custom' ? 'wide' : '');
    if (id === 'desktop') {
      choice.disabled = true;
      choice.title = 'A transparent overlay on the Windows system wallpaper is unavailable.';
    }
    bind(choice, () => choice.setAttribute('aria-pressed', String(state.preferences.background === id)));
    panel.append(choice);
  }
  return panel;
}

function trackRow() {
  const art = element('img', { alt: '', class: 'cover' });
  bind(art, () => {
    const src = state.playback.artwork ?? 'assets/lunalyr.png';
    if (art.getAttribute('src') !== src) art.src = src;
  });
  return element('div', { class: 'track' }, art, element('div', { class: 'track-label' },
    liveText(() => state.playback.track?.title ?? (surface === 'settings' ? demo.track.title : 'Nothing playing'), 'track-title'),
    liveText(() => state.playback.track?.artist ?? (surface === 'settings' ? demo.track.artist : 'Play music to get started'), 'caption')));
}
function transport() {
  const row = element('div', { class: 'transport', 'aria-label': 'Playback' });
  for (const [id, label, glyph, flag] of [['previous', 'Previous track', '◀◀', 'canPrevious'],
    ['play', 'Play or pause', '▶', 'canPlay'], ['next', 'Next track', '▶▶', 'canNext']]) {
    const control = button(glyph, () => send('control', id));
    control.setAttribute('aria-label', label);
    bind(control, () => {
      control.disabled = !state.playback[flag];
      if (id === 'play') control.textContent = state.playback.playing ? 'Ⅱ' : glyph;
    });
    row.append(control);
  }
  return row;
}
function offsetAction() {
  return element('div', { class: 'offset' }, text('Lyric timing', 'caption'),
    button('Earlier', () => send('offset', (state.lyrics.offset ?? 0) - .1), 'secondary'),
    liveText(() => seconds(state.lyrics.offset ?? 0)),
    button('Later', () => send('offset', (state.lyrics.offset ?? 0) + .1), 'secondary'),
    button('↺', () => send('offset', 0), 'secondary'));
}

function mount() {
  if (surface === 'wallpaper') {
    stage = new Stage(app, state, false);
    return;
  }
  notice = element('div', { class: 'notice', role: 'status', hidden: '' });
  if (surface === 'onboarding') mountOnboarding();
  else if (surface === 'tray') mountTray();
  else mountSettings();
  app.append(notice);
  refresh();
}

function mountSettings() {
  const nav = element('nav', { 'aria-label': 'Settings pages' });
  for (const [category, links] of pages) {
    nav.append(element('h2', {}, category));
    for (const [id, title, icon, tint] of links) {
      const item = button([element('span', { class: 'badge', style: '--badge:' + tint }, icon), title], () => go(id));
      bind(item, () => item.setAttribute('aria-current', id === page ? 'page' : 'false'));
      nav.append(item);
    }
  }
  const display = element('div', { class: 'display' });
  const preview = element('div', { class: 'stage', 'aria-label': 'Wallpaper preview' });
  stage = new Stage(preview, state, true);
  const detail = element('div', { class: 'detail' }, display,
    inspector = element('aside', { class: 'inspector', 'aria-label': 'Page controls' }));
  display.append(preview, element('div', { class: 'stage-footer' }, trackRow(), transport()));
  const title = liveText(() => pages.flatMap(row => row[1]).find(row => row[0] === page)[1]);
  const togglePreview = button('Hide preview', () => {
    previewHidden = !previewHidden;
    detail.classList.toggle('without-preview', previewHidden);
    display.hidden = previewHidden;
    togglePreview.textContent = previewHidden ? 'Show preview' : 'Hide preview';
  }, 'secondary');
  app.append(element('header', {}, element('h1', {}, title), togglePreview),
    element('div', { class: 'workspace' }, nav, detail));
  renderInspector();
}

function go(id) {
  if (!pages.some(([, links]) => links.some(([key]) => key === id))) return;
  page = id;
  if (inspector) { inspector.scrollTop = 0; renderInspector(); }
  refresh();
}

function renderInspector() {
  if (!inspector) return;
  const active = document.activeElement;
  const label = active?.closest('label')?.textContent;
  inspector.replaceChildren(...({
    wallpaper: wallpaperPage, typography: typographyPage, covers: coversPage, timing: timingPage,
    readings: readingPage, general: generalPage, extras: extrasPage, performance: performancePage, about: aboutPage
  }[page])());
  clearBindings();
  if (label && !active.isConnected) {
    const row = [...inspector.querySelectorAll('label')].find(item => item.textContent === label);
    row?.querySelector('select, input')?.focus();
  }
}
function wallpaperPage() {
  const p = state.preferences;
  const result = [group('Wallpaper', modeGrid(), caption({
    match: 'Each song picks a look to suit its energy and feel.',
    visualiser: 'Use system sound to shape the wallpaper.',
    effects: 'Add an ambient effect behind the lyrics.',
    patterns: 'Draw lightweight lines in the album’s colours.',
    colour: 'Use colour from the album art.',
    desktop: 'The transparent Windows wallpaper overlay is unavailable.',
    custom: 'Choose an image or a silent looping video.'
  }[p.background]))];
  if (p.background === 'visualiser') {
    const controls = [select('Visualiser', 'visualiser', visualisers, {
      get: () => state.preferences.visualiser,
      set: value => { preference('visualiser', value); renderInspector(); }
    })];
    if (p.visualiser.startsWith('wavefield')) controls.push(select('Preset', 'preset', presetList),
      select('Projection', 'projection', [['preset', 'Preset'], ['plane', 'Plane'], ['sphere', 'Sphere']]));
    result.push(group('Visualiser', controls), group('Music response',
      toggle('Use system audio', 'audio', 'Audio is analysed locally and discarded.'),
      range('Sensitivity', 'sensitivity', .3, 3, .05, multiple), range('Punch', 'punch', 0, 2, .05, pct),
      liveText(() => state.status.audio, 'caption group-note'), action('Reconnect', 'reconnect')));
  }
  if (p.background === 'match' || p.background === 'effects' || p.background === 'patterns') {
    const rows = [];
    if (p.background !== 'patterns') rows.push(select('GPU effect', 'effect', [['auto', 'Match the music'], ...Object.entries(effectNames)]));
    if (p.background !== 'effects') rows.push(select('Pattern', 'pattern', [['auto', 'Match the music'], ['waves', 'Waves'], ['grid', 'Grid'], ['contour', 'Contour']]));
    result.push(group(p.background === 'match' ? 'Match the music' : 'Appearance', rows));
  }
  if (p.background === 'custom') result.push(group('Imported wallpaper', action('Choose image or video…', 'wallpaper'),
    select('Placement', 'placement', [['fill', 'Fill'], ['fit', 'Fit'], ['top', 'Top'], ['bottom', 'Bottom']])));
  result.push(group('Legibility', toggle('Automatic text backing', 'scrim'),
    caption('Light covers and images get a soft shadow behind the lyrics.')));
  result.push(group('Adjustments', toggle('Album gradient', 'albumGradient'),
    range('Motion speed', 'speed', .1, 2, .1, multiple), range('Colour richness', 'richness', .2, 2, .05, pct),
    range('Darkening', 'darkening', .25, 1.75, .05, pct),
    button('Reset Adjustments', () => change({ speed: 1, richness: 1, darkening: 1 }), 'secondary')));
  return result;
}
function typographyPage() {
  const p = state.preferences;
  const style = p.composition;
  const perStyle = (key, fallback) => ({
    get: () => state.preferences[key]?.[style] ?? fallback,
    set: value => preference(key, { ...state.preferences[key], [style]: value })
  });
  const colourRows = [
    select('Text colour', 'textColour', textColours, { get: () => state.preferences.textColour,
      set: value => { preference('textColour', value); renderInspector(); } }),
    range('Colour strength', 'colourStrength', 0, 1, .02, pct)
  ];
  if (p.textColour === 'customGradient') colourRows.push(colour('From', 'customText'), colour('To', 'customEnd'));
  colourRows.push(select('Highlight colour', 'highlight', highlights, { get: () => state.preferences.highlight,
    set: value => { preference('highlight', value); renderInspector(); } }));
  if (p.highlight === 'custom') colourRows.push(colour('Custom highlight', 'customHighlight'));
  return [group('Composition',
    select('Lyric style', 'composition', compositions, { get: () => state.preferences.composition,
      set: value => { preference('composition', value); renderInspector(); } }),
    select('Font', 'font', fonts), toggle('Editorial typography', 'editorial'), range('Lyric size', 'lyricSize', .55, 1.65, .05, pct)),
    group('Word by word', select('Word entrance', 'arrivals', arrivals, perStyle('arrivals', 'auto')),
      toggle('Reveal sung words', 'reveals', 'Some entrances and rolling styles always reveal sung words.', perStyle('reveals', true)),
      toggle('Preview next word', 'nextWords', null, perStyle('nextWords', true)),
      range('Preview lead', 'previewLead', .1, .8, .05, seconds)),
    group('Text & highlight colour', colourRows),
    group('Constellation', range('Spread', 'spread', .2, 1.5, .05, pct), range('Link opacity', 'links', 0, .8, .05, pct))];
}
function coversPage() {
  return [group('Animated covers', caption('Windows media sessions provide static cover art. Lunalyr uses that artwork for the wallpaper palette.')),
    group('Artwork', toggle('Album gradient', 'albumGradient'), range('Colour richness', 'richness', .2, 2, .05, pct))];
}
function timingPage() {
  const options = [['', 'Automatic'], ...(state.lyrics.sources ?? []).filter(row => row.source !== 'import')
    .map(row => [row.source, `${sourceNames[row.source]} · ${quality[row.quality]}`])];
  const list = state.preferences.sources.map((source, i, all) => element('div', { class: 'source' },
    toggle(sourceNames[source], 'disabledSources', null, {
      get: () => !state.preferences.disabledSources.includes(source),
      set: enabled => preference('disabledSources', enabled ? state.preferences.disabledSources.filter(id => id !== source)
        : [...state.preferences.disabledSources, source])
    }),
    button('↑', () => {
      if (i === 0) return;
      const reordered = all.slice();
      [reordered[i - 1], reordered[i]] = [reordered[i], reordered[i - 1]];
      preference('sources', reordered);
      renderInspector();
    }, 'secondary')));
  return [group('This recording', offsetAction(), liveText(() => state.lyrics.lookup, 'caption group-note'),
    select('Saved lyric choice', '', options, {
      get: () => state.lyrics.saved && state.lyrics.selected?.source !== 'import' ? state.lyrics.selected?.source : '',
      set: value => send('source', value || null)
    }),
    element('div', { class: 'actions' }, action('Try again', 'refresh'), action('Import lyrics…', 'lyricsFile'),
      action('Use automatic', 'source', null))),
  group('Lyric sources', select('Selection', 'lookupPolicy', [['accuracy', 'Accuracy first'], ['order', 'Source order']]),
    toggle('Prefer word timing', 'preferWords'), list,
    action('Choose local lyrics folder…', 'lyricsFolder'),
    caption(state.preferences.lyricsFolder ? 'Using your selected export folder.' : 'Using Lunalyr’s local lyrics folder.'))];
}
function readingPage() {
  return [group('Pronunciation readings',
    select('Romanisation', 'reading', [['off', 'Off'], ['below', 'Below lyrics'], ['replace', 'Replace lyrics']]),
    caption('Uses readings from the lyric source first. When absent, Lunalyr can generate Korean and Japanese readings locally.'))];
}
function players() {
  const items = state.players.map(player => [player, player.split('!').at(-1)]);
  const current = state.preferences.player;
  if (current && !state.players.includes(current)) items.push([current, 'Unavailable · ' + current.split('!').at(-1)]);
  return [['', 'Automatic'], ...items];
}
function generalPage() {
  return [group('Playback', select('Player', 'player', players()), liveText(() => state.playback.track?.player, 'caption group-note')),
    group('Behaviour', toggle('Launch at login', 'startup'), toggle('Enable wallpaper', 'wallpaperEnabled'),
      toggle('Show lyrics', 'lyricsVisible'), toggle('Instrumental interludes', 'interludes'),
      select('When lyrics are unavailable', 'missingLyrics', [['notice', 'Show a notice'], ['track', 'Show track title'], ['nothing', 'Show nothing']])),
    group('Appearance', select('Theme', 'theme', [['system', 'System'], ['light', 'Light'], ['dark', 'Dark']]),
      action('Show onboarding…', 'onboarding'))];
}
function extrasPage() {
  return [group('Global shortcuts',
    ...[['Show or hide lyrics', 'Ctrl + Alt + L'], ['Pause or resume wallpaper', 'Ctrl + Alt + P'],
      ['Lyrics earlier', 'Ctrl + Alt + ,'], ['Lyrics later', 'Ctrl + Alt + .']].map(([label, key]) =>
      element('div', { class: 'field' }, label, element('kbd', {}, key))),
    liveText(() => state.status.notice, 'caption group-note')),
    group('Copy lyrics', button('Copy current line', () => {
      const line = currentLine();
      if (line) send('copyText', line.text);
      else showNotice('Play a song with lyrics to copy a line.');
    }, 'secondary'), button('Copy animated lyric card', copyCard, 'secondary'))];
}
function performancePage() {
  return [group('Rendering',
    select('Frame rate', 'fps', [['15', '15 fps'], ['30', '30 fps'], ['60', '60 fps']], {
      get: () => state.preferences.fps, set: value => preference('fps', Number(value)) }),
    toggle('Pause when covered', 'pauseCovered'), toggle('Low power visuals', 'lowPower'),
    toggle('Use low power visuals on battery', 'respectBattery'), toggle('Reduce motion and flashes', 'calm'),
    liveText(() => `${state.status.desktop ?? 0} wallpaper surface(s)`, 'caption group-note'))];
}
function aboutPage() {
  return [group('Lunalyr', element('div', { class: 'about' }, element('img', { src: 'assets/lunalyr.png', alt: '' }),
    element('div', {}, element('strong', {}, 'Lunalyr for Windows'), caption('0.2.0 · Windows replacement preview'))),
    action('Open releases…', 'releases'))];
}

function currentLine() {
  return cueAt(state.lyrics.selected?.lines ?? [], position(state, performance.now()) - (state.lyrics.offset ?? 0)).line;
}
async function copyCard() {
  if (cardBusy) return;
  cardBusy = true;
  try {
    showNotice('Preparing animated lyric card…');
    const time = position(state, performance.now()) - (state.lyrics.offset ?? 0);
    const card = await lyricCard(state, time, stage?.palette(performance.now()));
    send('copyCard', card);
  } catch (error) { showNotice(error.message); }
  finally { cardBusy = false; }
}
function mountTray() {
  const quick = (label, glyph, click) => button([text(glyph, 'round'), label], click, 'quick');
  const source = liveText(() => state.lyrics.selected && `${sourceNames[state.lyrics.selected.source]} · ${quality[state.lyrics.selected.quality]}`, 'caption');
  const line = element('strong', { class: 'tray-line' });
  const next = element('p', { class: 'caption tray-next' });
  setInterval(() => {
    const cue = cueAt(state.lyrics.selected?.lines ?? [], position(state, performance.now()) - (state.lyrics.offset ?? 0));
    line.textContent = cue.line?.text ?? state.lyrics.lookup ?? '';
    next.textContent = cue.next?.text ?? '';
  }, 180);
  app.append(element('div', { class: 'tray' }, trackRow(), transport(), line, next, source, offsetAction(),
    element('hr'), element('div', { class: 'group-bar' }, element('h2', {}, 'Wallpaper'), action('Options…', 'settings', 'wallpaper')),
    modeGrid(), element('div', { class: 'quick-grid' },
      quick('Pause wallpaper', 'Ⅱ', () => preference('wallpaperEnabled', !state.preferences.wallpaperEnabled)),
      quick('Toggle lyrics', '◉', () => preference('lyricsVisible', !state.preferences.lyricsVisible)), quick('Copy card', '▧', copyCard)),
    element('hr'), action('Settings…', 'settings'), action('Open releases…', 'releases'), action('Quit Lunalyr', 'quit')));
}
function mountOnboarding() {
  const wizard = element('div', { class: 'wizard' });
  app.prepend(wizard);
  function drawStep() {
    stage?.dispose();
    const [title, detail] = welcome[step];
    const preview = element('div', { class: 'stage' });
    stage = new Stage(preview, state, true);
    let choices;
    if (step === 1) choices = element('div', { class: 'wizard-controls' }, trackRow(), transport());
    if (step === 2) choices = modeGrid();
    if (step === 3) choices = element('div', { class: 'wizard-controls' },
      select('Lyric style', 'composition', compositions), select('Romanisation', 'reading', [['off', 'Off'], ['below', 'Below'], ['replace', 'Replace']]));
    wizard.replaceChildren(element('div', { class: 'wizard-main' }, preview,
      step === 0 && element('img', { src: 'assets/lunalyr.png', alt: '' }), element('h1', {}, title), caption(detail), choices),
    element('footer', {}, element('div', { class: 'steps', 'aria-label': `Step ${step + 1} of 6` },
      welcome.map((_, i) => element('span', { class: i === step ? 'active' : '' }))),
    step > 0 && button('Back', () => { step--; drawStep(); }, 'secondary'),
    button(step === 5 ? 'Done' : 'Continue', () => {
      if (step === 5) send('done');
      else { step++; drawStep(); }
    }, 'primary')));
    clearBindings();
    refresh();
  }
  drawStep();
}

fetch('engines/WavefieldPresets.json').then(response => response.json()).then(presets => {
  presetList = presets.map(({ id, name }) => [id, name]);
  if (state.preferences?.background === 'visualiser') renderInspector();
}).catch(() => {});
