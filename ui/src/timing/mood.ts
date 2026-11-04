// `TrackMood.derive` (Sources/UI/TrackMood.swift). Windows media sessions do
// not report a genre, so in practice the genre term takes its "unknown"
// values and the mood comes from lyric cadence and cover saturation.

export interface TrackMood {
  /** 0 = still, 1 = frantic. */
  energy: number;
  /** Multiplier on rotation and wave amplitude in the lyric layout. */
  intensity: number;
  /** 0 = machine-made, 1 = played and organic. */
  organic: number;
  motionRate: number;
}

export const NEUTRAL_MOOD: TrackMood = { energy: 0.5, intensity: 1, organic: 0.5, motionRate: 1 };

const ELECTRONIC = ["electronic", "dance", "house", "techno", "edm", "drum", "dubstep", "trance"];
const ROCK = ["rock", "metal", "punk", "grunge", "alternative"];
const HIPHOP = ["hip", "rap", "trap", "grime", "drill"];
const POP = ["pop", "k-pop"];
const SOUL = ["r&b", "soul", "funk", "disco"];
const CLASSICAL = ["jazz", "blues", "classical", "opera", "orchestral"];
const ACOUSTIC = ["ambient", "chill", "acoustic", "folk", "country", "singer"];

// The Mac checks rock before hip-hop for energy but after it for organic.
const ENERGY: [string[], number][] = [[ELECTRONIC, 0.9], [ROCK, 0.85], [HIPHOP, 0.8], [POP, 0.65], [SOUL, 0.55], [CLASSICAL, 0.2], [ACOUSTIC, 0.25]];
const ORGANIC: [string[], number][] = [[ELECTRONIC, 0.12], [HIPHOP, 0.3], [ROCK, 0.45], [POP, 0.52], [SOUL, 0.68], [CLASSICAL, 0.88], [ACOUSTIC, 0.92]];

function lookup(table: [string[], number][], text: string): number {
  for (const [needles, value] of table) if (needles.some((n) => text.includes(n))) return value;
  return 0.5;
}

export function deriveMood(options: { genre?: string | null; wordsPerSecond: number; artworkSaturation: number }): TrackMood {
  const text = (options.genre ?? "").toLowerCase();
  const genreEnergy = lookup(ENERGY, text);
  const genreOrganic = lookup(ORGANIC, text);
  const wps = options.wordsPerSecond;
  const cadence = wps > 0 ? Math.min(Math.max((wps - 0.8) / 2.6, 0), 1) : genreEnergy;
  const energy = Math.min(Math.max(genreEnergy * 0.55 + cadence * 0.45, 0), 1);
  const saturation = Number.isFinite(options.artworkSaturation) ? options.artworkSaturation : 0.5;
  return {
    energy,
    motionRate: 0.45 + energy * 1.25,
    intensity: 0.25 + energy * 1.3 + saturation * 0.2,
    // A vivid cover nudges the read a little more organic.
    organic: Math.min(Math.max(genreOrganic + (saturation - 0.5) * 0.16, 0), 1),
  };
}
