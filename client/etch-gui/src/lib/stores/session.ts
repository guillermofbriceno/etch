// Registry of the stores that belong to a server session.
//
// Which state was session-scoped used to be a hand-written list of reset calls
// in the ServerReset branch of eventRouter.ts. Nothing tied that list to the
// stores it was clearing, so a new session-scoped store was only cleared if
// whoever added it remembered to edit a file on the far side of the module
// graph, and nothing failed when they did not. Registering beside the store
// definition puts the declaration where the reader already is, and gives the
// test suite a concrete set of names to assert against.
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

type SessionReset = () => void;

const registries: Record<SessionScope, Map<string, SessionReset>> = {
    matrix: new Map(),
    voice: new Map(),
};

/**
 * Declare that `name` holds state belonging to the current `scope` session,
 * and that `reset` returns it to the value it had before that session began.
 * Call this next to the store's definition, at module scope.
 */
export function registerSessionStore(scope: SessionScope, name: string, reset: SessionReset): void {
    // A repeat registration overwrites rather than throws, because Vite
    // re-evaluates a store module on hot reload without re-evaluating this
    // one. Two different stores sharing a name would go unnoticed here, so
    // the name snapshots in session.test.ts are what catch that: a collision
    // shows up there as a missing name.
    registries[scope].set(name, reset);
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
