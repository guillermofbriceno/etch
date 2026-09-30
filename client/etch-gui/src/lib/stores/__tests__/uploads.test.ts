import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { uploadLimits, handleMatrixEvent } from '../uploads';
import { toastError } from '../errors';
import { resetMatrixSession } from '../session';
import { resetStores } from './helpers';

vi.mock('../sfx', () => ({
    playSfx: vi.fn(),
    setSfxDeafened: vi.fn(),
    sfxVolume: { subscribe: vi.fn() },
}));

beforeEach(() => {
    vi.useFakeTimers();
    resetStores();
});

afterEach(() => {
    vi.useRealTimers();
});

describe('upload limits', () => {
    it('are unknown until the session reports them', () => {
        expect(get(uploadLimits)).toBeNull();
    });

    it('take the effective limits from UploadLimits', () => {
        handleMatrixEvent({ type: 'UploadLimits', data: { image_bytes: 1_048_576, other_bytes: 1_048_576 } });

        expect(get(uploadLimits)).toEqual({ image_bytes: 1_048_576, other_bytes: 1_048_576 });
    });

    it('are replaced by the next session report', () => {
        handleMatrixEvent({ type: 'UploadLimits', data: { image_bytes: 5_242_880, other_bytes: 2_097_152 } });
        handleMatrixEvent({ type: 'UploadLimits', data: { image_bytes: 1_048_576, other_bytes: 1_048_576 } });

        expect(get(uploadLimits)).toEqual({ image_bytes: 1_048_576, other_bytes: 1_048_576 });
    });

    it('go back to unknown when the Matrix session ends', () => {
        handleMatrixEvent({ type: 'UploadLimits', data: { image_bytes: 5_242_880, other_bytes: 2_097_152 } });

        resetMatrixSession();

        expect(get(uploadLimits)).toBeNull();
    });

    it('ignore unrelated Matrix events', () => {
        handleMatrixEvent({ type: 'UploadLimits', data: { image_bytes: 5_242_880, other_bytes: 2_097_152 } });

        handleMatrixEvent({ type: 'PasswordRequest' });

        expect(get(uploadLimits)).toEqual({ image_bytes: 5_242_880, other_bytes: 2_097_152 });
        expect(get(toastError)).toBeNull();
    });
});

describe('AttachmentFailed', () => {
    it('shows the file name and the reason core gave', () => {
        handleMatrixEvent({
            type: 'AttachmentFailed',
            data: {
                room_id: '!a:example.org',
                file_name: 'clip.mp4',
                reason: 'it is 3.4 MB and the limit for this kind of file is 2 MB',
            },
        });

        expect(get(toastError)).toBe("Couldn't send clip.mp4: it is 3.4 MB and the limit for this kind of file is 2 MB");
    });

    it('does not change the limits', () => {
        handleMatrixEvent({ type: 'AttachmentFailed', data: { room_id: '!a:example.org', file_name: 'a.txt', reason: 'the file could not be read' } });

        expect(get(uploadLimits)).toBeNull();
    });
});
