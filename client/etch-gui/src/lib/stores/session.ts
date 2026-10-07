// Matrix ends on ServerReset; voice ends on a Mumble Disconnected, and a Matrix
// reconnect leaves it alone. A login outlasts every Matrix reconnect: it ends on a
// sign out, or when the app connects as a different account.
// This module must stay a leaf so every store module can import it without a cycle.

export type SessionScope = 'matrix' | 'voice' | 'login';

/** Why a store survives a session ending: a per-install preference, engine-persisted state, or computed from other stores. */
export type StoreExemption = 'device' | 'backend' | 'derived';

type SessionReset = () => void;

const registries: Record<SessionScope, Map<string, SessionReset>> = {
    matrix: new Map(),
    voice: new Map(),
    login: new Map(),
};

const exempt = new Map<string, StoreExemption>();

let loginAccount: string | null = null;

/** `name` must be the store's own identifier, which the classification test matches against the source. */
export function registerSessionStore(scope: SessionScope, name: string, reset: SessionReset): void {
    // Overwrites rather than throws, because hot reload re-evaluates store modules but
    // not this one.
    registries[scope].set(name, reset);
}

/** Declare stores that outlive a session, and why. */
export function declareStores(reason: StoreExemption, ...names: string[]): void {
    for (const name of names) exempt.set(name, reason);
}

export function exemptStoreEntries(): [string, StoreExemption][] {
    return [...exempt.entries()].sort(([a], [b]) => a.localeCompare(b));
}

export function resetMatrixSession(): void {
    for (const reset of registries.matrix.values()) reset();
}

export function resetVoiceSession(): void {
    for (const reset of registries.voice.values()) reset();
}

/** Every connect names its account, and only a different one ends the login before it. */
export function enterLogin(account: string): void {
    if (account === loginAccount) return;
    resetLoginSession();
    loginAccount = account;
}

export function resetLoginSession(): void {
    loginAccount = null;
    for (const reset of registries.login.values()) reset();
}

export function sessionStoreNames(scope: SessionScope): string[] {
    return [...registries[scope].keys()].sort();
}
