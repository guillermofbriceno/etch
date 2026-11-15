import { EMOJI_DATA } from './emojiData';

export interface EmojiGroupData {
    name: string;
    icon: string;
    emojis: [emoji: string, name: string, keywords: string, version: number][];
}

export interface Emoji {
    emoji: string;
    name: string;
    version: number;
}

export interface EmojiGroup {
    name: string;
    icon: string;
    emojis: Emoji[];
}

/** What the platform's emoji font can draw, which lags the Unicode release the data comes from. */
export interface RenderSupport {
    maxVersion: number;
    flags: boolean;
}

interface Indexed {
    entry: Emoji;
    normalizedName: string;
    nameWords: string[];
    allWords: string[];
}

const FLAG_LETTERS = /[\u{1F1E6}-\u{1F1FF}\u{E0061}-\u{E007A}]/u;

const normalize = (s: string): string => s.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();

export const EMOJI_GROUPS: EmojiGroup[] = EMOJI_DATA.map((g) => ({
    name: g.name,
    icon: g.icon,
    emojis: g.emojis.map(([emoji, name, , version]) => ({ emoji, name, version })),
}));

const INDEX: Indexed[] = EMOJI_DATA.flatMap((g, gi) => g.emojis.map(([, name, keywords], ei) => {
    const normalizedName = normalize(name);
    const nameWords = normalizedName.split(' ');
    return {
        entry: EMOJI_GROUPS[gi].emojis[ei],
        normalizedName,
        nameWords,
        allWords: [...nameWords, ...normalize(keywords).split(' ')],
    };
}));

export function isRenderable(e: Emoji, support: RenderSupport): boolean {
    return e.version <= support.maxVersion && (support.flags || !FLAG_LETTERS.test(e.emoji));
}

/** One emoji per version, newest first, preferring a new code point over a sequence of older ones. */
export function versionProbes(): [version: number, emoji: string][] {
    const probes = new Map<number, string>();
    for (const g of EMOJI_GROUPS) {
        for (const e of g.emojis) {
            const current = probes.get(e.version);
            if (!current || (current.includes('‍') && !e.emoji.includes('‍'))) probes.set(e.version, e.emoji);
        }
    }
    return [...probes].sort((a, b) => b[0] - a[0]);
}

/** Every query word must start some word of the emoji's name or keywords; name matches rank first. */
export function searchEmoji(query: string): Emoji[] {
    const q = normalize(query);
    if (!q) return [];
    const terms = q.split(' ');
    const ranked: Emoji[][] = [[], [], [], []];
    for (const item of INDEX) {
        if (!terms.every((t) => item.allWords.some((w) => w.startsWith(t)))) continue;
        const rank = item.normalizedName === q ? 0
            : item.normalizedName.startsWith(q) ? 1
            : terms.every((t) => item.nameWords.some((w) => w.startsWith(t))) ? 2
            : 3;
        ranked[rank].push(item.entry);
    }
    return ranked.flat();
}

const EMOJI_GRAPHEME = /\p{Regional_Indicator}|⃣|\p{Emoji_Presentation}|\p{Extended_Pictographic}[️\p{Emoji_Modifier}]/u;
const JUMBO_MAX = 3;
const graphemes = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

/** A message of only a few emoji is shown large, as most messengers do. */
export function isJumboEmoji(text: string): boolean {
    const parts = [...graphemes.segment(text.replace(/\s+/g, ''))];
    return parts.length > 0 && parts.length <= JUMBO_MAX && parts.every((p) => EMOJI_GRAPHEME.test(p.segment));
}
