import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { talkingUsers } from '$lib/stores';
import type { VoiceUser } from '$lib/stores/voiceState';
import VoiceUserList from '../VoiceUserList.svelte';
import { resetStores } from '$lib/stores/__tests__/helpers';

function makeUser(overrides: Partial<VoiceUser> & { session_id: number; name: string }): VoiceUser {
    return {
        display_name: null,
        avatar_url: null,
        channel_id: 1,
        muted: false,
        deafened: false,
        hash: null,
        ...overrides,
    };
}

beforeEach(() => {
    resetStores();
});

describe('VoiceUserList', () => {
    it('renders user display names', () => {
        const users = [
            makeUser({ session_id: 1, name: 'alice', display_name: 'Alice' }),
            makeUser({ session_id: 2, name: 'bob', display_name: 'Bob' }),
            makeUser({ session_id: 3, name: 'carol', display_name: 'Carol' }),
        ];
        render(VoiceUserList, { props: { users } });

        expect(screen.getByText('Alice')).toBeInTheDocument();
        expect(screen.getByText('Bob')).toBeInTheDocument();
        expect(screen.getByText('Carol')).toBeInTheDocument();
    });

    it('falls back to name when display_name is null', () => {
        const users = [makeUser({ session_id: 1, name: 'bob_raw' })];
        render(VoiceUserList, { props: { users } });

        expect(screen.getByText('bob_raw')).toBeInTheDocument();
    });

    it('deafened user shows two status icons (mic + headphones)', () => {
        const users = [makeUser({ session_id: 1, name: 'deaf', display_name: 'Deaf', deafened: true })];
        const { container } = render(VoiceUserList, { props: { users } });

        const icons = container.querySelectorAll('.voice-status-icons svg');
        expect(icons).toHaveLength(2);
    });

    it('muted user shows one status icon', () => {
        const users = [makeUser({ session_id: 1, name: 'mute', display_name: 'Mute', muted: true })];
        const { container } = render(VoiceUserList, { props: { users } });

        const icons = container.querySelectorAll('.voice-status-icons svg');
        expect(icons).toHaveLength(1);
        // Muted icon has size 14
        expect(icons[0].getAttribute('width')).toBe('14');
    });

    it('talking user shows talking icon', () => {
        talkingUsers.set(new Set([1]));
        const users = [makeUser({ session_id: 1, name: 'talk', display_name: 'Talk' })];
        const { container } = render(VoiceUserList, { props: { users } });

        const icons = container.querySelectorAll('.voice-status-icons svg');
        expect(icons).toHaveLength(1);
        // Talking icon is a filled green circle
        expect(icons[0].innerHTML).toContain('fill="#3ba55d"');
    });

    it('silent user shows silent icon', () => {
        const users = [makeUser({ session_id: 1, name: 'quiet', display_name: 'Quiet' })];
        const { container } = render(VoiceUserList, { props: { users } });

        const icons = container.querySelectorAll('.voice-status-icons svg');
        expect(icons).toHaveLength(1);
        // Silent icon is a stroked gray circle
        expect(icons[0].innerHTML).toContain('stroke="#72767d"');
    });

    it('right-click prevents default context menu', async () => {
        const users = [makeUser({ session_id: 1, name: 'alice', display_name: 'Alice' })];
        render(VoiceUserList, { props: { users } });

        const li = screen.getByText('Alice').closest('li')!;
        const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
        li.dispatchEvent(event);

        expect(event.defaultPrevented).toBe(true);
    });
});
