// Bundled demo song catalogue used by the off-chain "Quick Game" practice mode.
// This lets a brand-new visitor play a full round with no wallet and no signing.
// When the backend song catalogue (LF-090) is available it can be layered on top
// of this demo set, but the practice mode must always work offline.

export interface DemoLyricLine {
  /** Timestamp in seconds when the line becomes active. */
  time: number;
  /** The lyric text shown on the LyricCard. */
  text: string;
}

export interface DemoSong {
  id: string;
  title: string;
  artist: string;
  /** Duration of the round in seconds. */
  duration: number;
  lyrics: DemoLyricLine[];
}

/**
 * A small, self-contained demo catalogue. Each song is short enough to finish
 * a 5-card practice round quickly and contains enough lines to exercise the
 * LyricCard / SongOptions / timer components used by the on-chain modes.
 */
export const DEMO_SONGS: DemoSong[] = [
  {
    id: 'demo-1',
    title: 'Neon Skyline',
    artist: 'Demo Artist',
    duration: 30,
    lyrics: [
      { time: 0, text: 'City lights are calling out my name' },
      { time: 6, text: 'Every window glowing like a flame' },
      { time: 12, text: 'We were never meant to stay the same' },
      { time: 18, text: 'Running through the neon skyline' },
      { time: 24, text: 'Chasing every color till we shine' },
    ],
  },
  {
    id: 'demo-2',
    title: 'Paper Boats',
    artist: 'Demo Artist',
    duration: 30,
    lyrics: [
      { time: 0, text: 'Folded dreams are sailing down the stream' },
      { time: 6, text: 'Little paper boats inside a dream' },
      { time: 12, text: 'If they sink I know just what it means' },
      { time: 18, text: 'We can build another one tonight' },
      { time: 24, text: 'Set it free and watch it catch the light' },
    ],
  },
  {
    id: 'demo-3',
    title: 'Slow Motion',
    artist: 'Demo Artist',
    duration: 30,
    lyrics: [
      { time: 0, text: 'Everything is moving slow today' },
      { time: 6, text: 'Seconds stretch and then they drift away' },
      { time: 12, text: 'I could stay right here and let it play' },
      { time: 18, text: 'Nothing in the world can make me run' },
      { time: 24, text: 'Slow motion till the day is done' },
    ],
  },
  {
    id: 'demo-4',
    title: 'Golden Hour',
    artist: 'Demo Artist',
    duration: 30,
    lyrics: [
      { time: 0, text: 'Sun is dipping low behind the hill' },
      { time: 6, text: 'Golden hour, everything is still' },
      { time: 12, text: 'Hold the moment, keep it if you will' },
      { time: 18, text: 'Shadows growing longer on the wall' },
      { time: 24, text: 'Golden hour, we can have it all' },
    ],
  },
  {
    id: 'demo-5',
    title: 'Echoes',
    artist: 'Demo Artist',
    duration: 30,
    lyrics: [
      { time: 0, text: 'Voices in the hallway fade to grey' },
      { time: 6, text: 'Echoes of the words we used to say' },
      { time: 12, text: 'I can hear them bouncing off the walls' },
      { time: 18, text: 'Every empty room remembers you' },
      { time: 24, text: 'Echoes finding ways to make it through' },
    ],
  },
];

/** Number of cards in a Quick Game practice round. */
export const QUICK_GAME_ROUND_SIZE = 5;

/**
 * Returns a shuffled copy of the demo catalogue limited to a practice round.
 * Shuffling keeps repeat plays fresh without requiring any backend call.
 */
export function getQuickGameSongs(count: number = QUICK_GAME_ROUND_SIZE): DemoSong[] {
  const shuffled = [...DEMO_SONGS];
  for (let i = shuffled.length - 1; i > 0; i -= 1) {
    const j = Math.floor(Math.random() * (i + 1));
    [shuffled[i], shuffled[j]] = [shuffled[j], shuffled[i]];
  }
  return shuffled.slice(0, Math.min(count, shuffled.length));
}
