// Registry of the stores that belong to a server session, and of the ones
// that deliberately do not.
//
// Which state was session-scoped used to be a hand-written list of reset calls
// in the ServerReset branch of eventRouter.ts. Nothing tied that list to the
// stores it was clearing, so a new session-scoped store was only cleared if
// whoever added it remembered to edit a file on the far side of the module
// graph, and nothing failed when they did not. Registering beside the store
// definition puts the declaration where the reader already is, and gives the
// test suite a concrete set of names to assert against.
//
// A registry alone still cannot see a store nobody registered, so every store
// in this directory has to declare itself one way or the other: session-scoped
// ones through registerSessionStore(), the rest through declareStores(). What
// makes that an enforceable rule rather than a convention is
// __tests__/storeClassification.test.ts, which reads the source of every
// module in this directory, finds every writable/derived/readable it defines,
// and fails on any that never reached this file. An omission -- the direction
// the original bug came from -- is a test failure there naming the store and
// the file it lives in.
//
// There are two sessions, not one, and they end independently:
//
//   'matrix'  The homeserver session. Ends on SystemEvent::ServerReset, which
//             the engine sends at the top of every connect attempt, retries in
//             the backoff loop included.
//   'voice'   The Mumble session. Ends on a Mumble ConnectionState of
//             Disconnected. A Matrix reconnect to the same voice server
//             deliberately leaves it alone (see resolve_and_launch_voice in
//             engine.rs), so voice state must not hang off ServerReset: there
//             would be no reconnect to repopulate it, and the user would be
//             left in voice looking at an empty voice panel.
//
// This module must stay a leaf: it imports nothing from the other stores, so
// that every store module can import it without creating a cycle.

export type SessionScope = 'matrix' | 'voice';

/**
 * Why a store is not session-scoped. Each value is a reason the store must
 * survive a session ending, so picking one is a claim, not a formality:
 *
 *   'device'   A preference or a piece of UI state that belongs to this
 *              install, not to any server it talks to.
 *   'backend'  The engine persists it and replays it on reconnect, so the
 *              frontend clearing it would only desync the two.
 *   'derived'  Computed from other stores, so it follows them and has no
 *              value of its own to reset.
 */
export type StoreExemption = 'device' | 'backend' | 'derived';

type SessionReset = () => void;

const registries: Record<SessionScope, Map<string, SessionReset>> = {
    matrix: new Map(),
    voice: new Map(),
};

// Every store that is not session-scoped, and the reason it is not.
const exempt = new Map<string, StoreExemption>();

/**
 * Declare that `name` holds state belonging to the current `scope` session,
 * and that `reset` returns it to the value it had before that session began.
 * Call this next to the store's definition, at module scope.
 *
 * `name` must be the store's own identifier: that is what lets the
 * classification test line these declarations up against the stores the
 * source actually defines.
 */
export function registerSessionStore(scope: SessionScope, name: string, reset: SessionReset): void {
    // A repeat registration overwrites rather than throws, because Vite
    // re-evaluates a store module on hot reload without re-evaluating this
    // one. Two stores sharing a name would silently collapse into one entry
    // here, so storeClassification.test.ts checks for that statically: it
    // reads the identifiers out of the source and fails if two files define
    // the same one, which is a claim this map cannot make about itself.
    registries[scope].set(name, reset);
}

/**
 * Declare stores that outlive a session, and why. Call this next to their
 * definitions, at module scope, with the stores' own identifiers.
 *
 * These get no reset, which is the point: naming them here is how a store
 * that should not be cleared is told apart from one nobody classified.
 */
export function declareStores(reason: StoreExemption, ...names: string[]): void {
    for (const name of names) exempt.set(name, reason);
}

/** Names declared exempt, with their reasons. Used by the enforcement tests. */
export function exemptStoreEntries(): [string, StoreExemption][] {
    return [...exempt.entries()].sort(([a], [b]) => a.localeCompare(b));
}

/** Clear every store belonging to the homeserver session. Fired on ServerReset. */
export function resetMatrixSession(): void {
    for (const reset of registries.matrix.values()) reset();
}

/** Clear every store belonging to the voice session. Fired on a Mumble disconnect. */
export function resetVoiceSession(): void {
    for (const reset of registries.voice.values()) reset();
}

/** Names registered in one scope, sorted. Used by the enforcement tests. */
export function sessionStoreNames(scope: SessionScope): string[] {
    return [...registries[scope].keys()].sort();
}
