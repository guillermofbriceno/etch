import { describe, it, expect } from 'vitest';
import { sessionStoreNames, exemptStoreEntries } from '../session';

// Importing the router, and no store module, is the point of this file.
//
// The router is what the app loads, and it pulls in every store module by
// globbing the directory, so the declarations below are the ones a running app
// would actually have. Importing a store module here directly would register
// it for the test and hide the case this file exists to catch: a module the
// running app never reaches.
import '../eventRouter';

// The source of every module in the stores directory, read as text. The
// running app's declarations can only say what registered; they cannot say
// what exists. This is the other half -- it sees a store whether or not
// anybody remembered to classify it.
//
// Stores defined outside this directory, in a component say, are out of scope
// here and always were: this is the state the session resets are about.
const sources = import.meta.glob('../*.ts', {
    query: '?raw',
    import: 'default',
    eager: true,
}) as Record<string, string>;

// The same read one level deeper, to check that the flat pattern above is
// still the whole directory. Neither glob can recurse in the router -- that
// would import the test files into the app -- so a store put in a
// subdirectory would be invisible to the reset and to this file alike.
const nested = import.meta.glob('../**/*.ts', {
    query: '?raw',
    import: 'default',
    eager: true,
}) as Record<string, string>;

// A module-scope `const x = writable(...)` / `derived(...)` / `readable(...)`,
// exported or not: private stores hold session state just as well as public
// ones, and two of the ones cleared on ServerReset are private.
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

/** Every store declared in either direction, with the bucket it landed in. */
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

// -----------------------------------------------------------------------
// The scan itself
// -----------------------------------------------------------------------

// Every check below compares what the source defines against what registered.
// A scan that found nothing would pass most of them without meaning anything,
// so this asserts the scan is still looking at the thing it claims to.
describe('the store scan', () => {
    it('reads the whole stores directory', () => {
        const files = Object.keys(sources).map(p => p.replace(/^.*\//, '')).sort();
        expect(files).toContain('session.ts');
        expect(files).toContain('eventRouter.ts');
        expect(files.length).toBeGreaterThan(15);
    });

    it('leaves no subdirectory for a store to hide in', () => {
        // Keys are relative to this file: './x.ts' is a test file next to it,
        // '../x.ts' a store module. Anything else is a directory neither the
        // router's glob nor the scan above looks in.
        const hidden = Object.keys(nested)
            .filter(path => !(path in sources) && !path.startsWith('./'));

        expect(hidden).toEqual([]);
    });

    it('finds the stores those files define', () => {
        const names = FOUND.map(s => s.name);
        // One public, one private, one derived: if the pattern stops matching
        // any of those shapes it stops covering a whole class of store.
        expect(names).toContain('channels');
        expect(names).toContain('hiddenDmInfos');
        expect(names).toContain('activeChannel');
        expect(names.length).toBeGreaterThan(20);
    });
});

// -----------------------------------------------------------------------
// Classification
// -----------------------------------------------------------------------

describe('every store is classified', () => {
    // This is the check the session registry could not make about itself.
    // sessionStoreNames() can only report what registered, so a store nobody
    // registered left every list unchanged and every test green -- which is
    // the direction the bug comes from. Reading the source instead means the
    // store has to be classified to exist.
    //
    // If this fails, work out which bucket the new store belongs in:
    //   matrix session  state from one homeserver connection, cleared on
    //                   ServerReset -- registerSessionStore('matrix', ...)
    //   voice session   state from one Mumble connection, cleared on its
    //                   disconnect -- registerSessionStore('voice', ...)
    //   device-scoped   a preference or UI state that has to survive both --
    //                   declareStores('device', ...)
    //   backend-owned   the engine persists and replays it, so the frontend
    //                   must not clear it -- declareStores('backend', ...)
    //   derived         computed from other stores, nothing of its own to
    //                   reset -- declareStores('derived', ...)
    //
    // Reaching for 'device' to make the red go away puts the bug back.
    it('leaves no store undeclared', () => {
        const declared = new Set(declarations().map(d => d.name));
        const undeclared = FOUND.filter(s => !declared.has(s.name)).map(describeStore);

        expect(undeclared).toEqual([]);
    });

    // The mirror image: a declaration whose store is gone, or renamed, or
    // misspelled. Without this a typo would look like a classification and
    // leave the real store uncovered -- and the reset it named would then be
    // clearing a store nobody can point at.
    it('declares no store this directory does not define', () => {
        const names = new Set(FOUND.map(s => s.name));
        const stale = declarations()
            .filter(d => !names.has(d.name))
            .map(d => `${d.bucket}: ${d.name}`);

        expect(stale).toEqual([]);
    });

    // A name is the only handle the registry has on a store, so two stores
    // answering to one name means one of them is quietly unregistered: the
    // second registration overwrites the first, and the reset that goes with
    // it is lost. The registry cannot see this -- it holds one entry either
    // way -- but the source can.
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

    // A store cannot both belong to a session and be exempt from one. Either
    // claim on its own is a decision; both at once is two people disagreeing,
    // and which one wins depends on module evaluation order.
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
