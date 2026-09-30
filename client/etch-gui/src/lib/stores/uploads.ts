import { writable } from 'svelte/store';
import type { MatrixEvent } from '$lib/ipc';
import { registerSessionStore } from './session';
import { showToast } from './errors';

export type UploadLimits = { image_bytes: number; other_bytes: number };

// Null until the session reports its limits; checks are skipped meanwhile because core enforces them too.
export const uploadLimits = writable<UploadLimits | null>(null);

registerSessionStore('matrix', 'uploadLimits', () => uploadLimits.set(null));

export function handleMatrixEvent(me: MatrixEvent): void {
    switch (me.type) {
        case 'UploadLimits':
            uploadLimits.set({ image_bytes: me.data.image_bytes, other_bytes: me.data.other_bytes });
            break;
        case 'AttachmentFailed':
            showToast(`Couldn't send ${me.data.file_name}: ${me.data.reason}`);
            break;
    }
}
