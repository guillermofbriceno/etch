import { writable } from 'svelte/store';
import { declareStores } from './session';

/** Set by the event router from the window's focus and blur. */
export const appFocused = writable(true);

declareStores('device', 'appFocused');
