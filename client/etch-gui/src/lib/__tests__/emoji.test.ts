import { describe, it, expect } from 'vitest';
import { EMOJI_GROUPS, isJumboEmoji, isRenderable, searchEmoji } from '$lib/emoji';

const find = (emoji: string) => EMOJI_GROUPS.flatMap((g) => g.emojis).find((e) => e.emoji === emoji)!;

describe('searchEmoji', () => {
    it.each([
        ['thumbs up', '👍'],
        ['THUMBS', '👍'],
        ['heart eyes', '😍'],
        ['cat', '🐈'],
        ['flag united', '🇦🇪'],
    ])('ranks name matches first: %s', (query, first) => {
        expect(searchEmoji(query)[0].emoji).toBe(first);
    });

    it('matches CLDR keywords that are not in the name', () => {
        expect(searchEmoji('lol').map((e) => e.emoji)).toContain('😂');
    });

    it('returns nothing for a blank or unmatched query', () => {
        expect(searchEmoji('  ')).toEqual([]);
        expect(searchEmoji('zzqx')).toEqual([]);
    });
});

describe('isRenderable', () => {
    it('hides emoji newer than the font and flags it cannot draw', () => {
        const support = { maxVersion: 12, flags: false };
        expect(isRenderable(find('😀'), support)).toBe(true);
        expect(isRenderable(find('🫠'), support)).toBe(false);
        expect(isRenderable(find('🇺🇸'), support)).toBe(false);
        expect(isRenderable(find('🏁'), support)).toBe(true);
    });
});

describe('isJumboEmoji', () => {
    it.each([
        ['👍', true],
        [' 😂 😂 ', true],
        ['👍🏽', true],
        ['👨‍👩‍👧', true],
        ['🇺🇸', true],
        ['❤️', true],
        ['1️⃣', true],
        ['😀😀😀😀', false],
        ['ok 👍', false],
        ['1', false],
        ['©', false],
        ['', false],
    ])('%s -> %s', (text, expected) => {
        expect(isJumboEmoji(text)).toBe(expected);
    });
});
