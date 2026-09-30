import { describe, it, expect } from 'vitest';
import { sessionStoreNames, exemptStoreEntries } from '../session';

// Kept apart from storeClassification.test.ts: importing every module here would register
// them all and hide one the router never reaches.
const modules = import.meta.glob('../*.ts', { eager: true }) as Record<string, Record<string, unknown>>;

function isStore(value: unknown): boolean {
    return typeof value === 'object' && value !== null
        && typeof (value as { subscribe?: unknown }).subscribe === 'function';
}

describe('every exported store is classified', () => {
    // Catches stores the source scan cannot parse, such as custom factories or aliased imports.
    it('leaves no subscribable export undeclared', () => {
        const declared = new Set([
            ...sessionStoreNames('matrix'),
            ...sessionStoreNames('voice'),
            ...exemptStoreEntries().map(([name]) => name),
        ]);

        const undeclared = Object.entries(modules).flatMap(([path, exports]) =>
            Object.entries(exports)
                .filter(([name, value]) => isStore(value) && !declared.has(name))
                .map(([name]) => `${path.replace(/^.*\//, '')}: ${name}`),
        );

        expect(undeclared).toEqual([]);
    });
});
