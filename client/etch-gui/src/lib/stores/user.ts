import { writable, get } from 'svelte/store';
import type { MatrixEvent, SystemEvent } from '$lib/ipc';
import { registerSessionStore } from './session';

export type UserInfo = {
    username: string;
    matrixId: string;
    displayName: string | null;
    avatarUrl: string | null;
};

// Matrix session. Identifies who we are logged in as on the connected
// homeserver, so it means nothing once that connection is gone.
export const currentUser = writable<UserInfo>({
    username: '',
    matrixId: '',
    displayName: null,
    avatarUrl: null,
});

export function resetUser(): void {
    currentUser.set({ username: '', matrixId: '', displayName: null, avatarUrl: null });
}

registerSessionStore('matrix', 'currentUser', resetUser);

export function handleMatrixEvent(me: MatrixEvent): void {
    if (me.type === 'CurrentUser') {
        currentUser.set({
            username: me.data.username,
            matrixId: me.data.matrix_id,
            displayName: me.data.display_name,
            avatarUrl: me.data.avatar_url,
        });
    }
}

export function handleSystemEvent(se: SystemEvent): void {
    if (se.type !== 'UserProfileChanged') return;
    const { username, display_name, avatar_url } = se.data;
    const user = get(currentUser);
    if (user.username === username) {
        currentUser.update(u => ({
            ...u,
            displayName: display_name ?? u.displayName,
            avatarUrl: avatar_url ?? u.avatarUrl,
        }));
    }
}
