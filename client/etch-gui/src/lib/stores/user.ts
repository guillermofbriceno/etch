import { writable, get } from 'svelte/store';
import type { MatrixEvent, SystemEvent } from '$lib/ipc';

export type UserInfo = {
    username: string;
    matrixId: string;
    displayName: string | null;
    avatarUrl: string | null;
};

export const currentUser = writable<UserInfo>({
    username: '',
    matrixId: '',
    displayName: null,
    avatarUrl: null,
});

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
