import { writable } from 'svelte/store';
import { declareStores } from './session';

const FOLLOW_KEY = 'follow-new-messages';

export const followNewMessages = writable<boolean>(localStorage.getItem(FOLLOW_KEY) === 'true');

declareStores('device', 'followNewMessages');

export function setFollowNewMessages(value: boolean): void {
    followNewMessages.set(value);
    localStorage.setItem(FOLLOW_KEY, String(value));
}
