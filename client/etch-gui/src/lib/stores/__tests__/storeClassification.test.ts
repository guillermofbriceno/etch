import { describe, it, expect } from 'vitest';
import { sessionStoreNames, exemptStoreEntries } from '../session';

// Imports the router alone: a direct store import would register it and hide a module
// the app never reaches.
import '../eventRouter';

// Source text of every store module, since registrations cannot reveal a store nobody classified.
const sources = import.meta.glob('../*.ts', {
    query: '?raw',
    import: 'default',
    eager: true,
}) as Record<string, string>;

// Checks that the flat glob above still covers the whole directory; a store in a
// subdirectory would escape both it and the reset.
const nested = import.meta.glob('../**/*.ts', {
    query: '?raw',
    import: 'default',
    eager: true,
}) as Record<string, string>;

// Matches private stores too, since they can hold session state.
const STORE_DEFINITION =
    /^[ \t]*(?:export[ \t]+)?(?:const|let|var)[ \t]+([A-Za-z_$][\w$]*)[ \t]*(?::[^=\n]*)?=[ \t]*(?:writable|readable|derived)\b/gm;

type FoundStore = { name: string; file: string };

function findStores(): FoundStore[] {
    const found: FoundStore[] = [];
    for (const [path, source] of Object.entries(sources)) {
        const file = path.replace(/^.*\//, '');
        for (const match of source.matchAll(STORE_DEFINITION)) {
            found.push({ name: match[1], file });
        }
    }
    return found.sort((a, b) => a.name.localeCompare(b.name));
}

const FOUND = findStores();

type Declaration = { name: string; bucket: string };

function declarations(): Declaration[] {
    return [
        ...sessionStoreNames('matrix').map(name => ({ name, bucket: 'matrix session' })),
        ...sessionStoreNames('voice').map(name => ({ name, bucket: 'voice session' })),
        ...exemptStoreEntries().map(([name, reason]) => ({ name, bucket: reason })),
    ];
}

function describeStore(s: FoundStore): string {
    return `${s.file}: ${s.name}`;
}

describe('the store scan', () => {
    it('reads the whole stores directory', () => {
        const files = Object.keys(sources).map(p => p.replace(/^.*\//, '')).sort();
        expect(files).toContain('session.ts');
        expect(files).toContain('eventRouter.ts');
        expect(files.length).toBeGreaterThan(15);
    });

    it('leaves no subdirectory for a store to hide in', () => {
        // Keys starting './' are test files next to this one, '../x.ts' are store modules.
        const hidden = Object.keys(nested)
            .filter(path => !(path in sources) && !path.startsWith('./'));

        expect(hidden).toEqual([]);
    });

    it('finds the stores those files define', () => {
        const names = FOUND.map(s => s.name);
        expect(names).toContain('channels');
        expect(names).toContain('hiddenDmInfos');
        expect(names).toContain('activeChannel');
        expect(names.length).toBeGreaterThan(20);
    });
});

describe('every store is classified', () => {
    // A store nobody registered leaves the registry unchanged, so the source has to be
    // read instead.
    it('leaves no store undeclared', () => {
        const declared = new Set(declarations().map(d => d.name));
        const undeclared = FOUND.filter(s => !declared.has(s.name)).map(describeStore);

        expect(undeclared).toEqual([]);
    });

    // A typo in a declaration would otherwise look like a classification.
    it('declares no store this directory does not define', () => {
        const names = new Set(FOUND.map(s => s.name));
        const stale = declarations()
            .filter(d => !names.has(d.name))
            .map(d => `${d.bucket}: ${d.name}`);

        expect(stale).toEqual([]);
    });

    // A second store under the same name would silently overwrite the first registration.
    it('gives every store a name of its own', () => {
        const byName = new Map<string, string[]>();
        for (const store of FOUND) {
            byName.set(store.name, [...(byName.get(store.name) ?? []), store.file]);
        }
        const collisions = [...byName.entries()]
            .filter(([, files]) => files.length > 1)
            .map(([name, files]) => `${name} (${files.join(', ')})`);

        expect(collisions).toEqual([]);
    });

    // Membership in a session and exemption from one contradict each other.
    it('classifies each store exactly once', () => {
        const buckets = new Map<string, string[]>();
        for (const { name, bucket } of declarations()) {
            buckets.set(name, [...(buckets.get(name) ?? []), bucket]);
        }
        const contradictions = [...buckets.entries()]
            .filter(([, where]) => where.length > 1)
            .map(([name, where]) => `${name} (${where.join(', ')})`);

        expect(contradictions).toEqual([]);
    });
});
