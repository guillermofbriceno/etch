import { get, type Unsubscriber } from 'svelte/store';
import { TrayIcon } from '@tauri-apps/api/tray';
import { Menu, MenuItem, PredefinedMenuItem } from '@tauri-apps/api/menu';
import { Image } from '@tauri-apps/api/image';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { isMuted, isDeafened, toggleMute, toggleDeafen } from './audio';

let tray: TrayIcon | null = null;
let muteItem: MenuItem | null = null;
let deafenItem: MenuItem | null = null;
let unsubscribers: Unsubscriber[] = [];
let initPromise: Promise<void> | null = null;

type TrayState = 'unmuted' | 'muted' | 'deafened';

let icons: Record<TrayState, Image> | null = null;

async function loadIconBytes(path: string): Promise<Uint8Array> {
    const resp = await fetch(path);
    return new Uint8Array(await resp.arrayBuffer());
}

async function loadIcons(): Promise<void> {
    const [unmuted, muted, deafened] = await Promise.all([
        loadIconBytes('/icons/tray-unmuted.png').then((b) => Image.fromBytes(b)),
        loadIconBytes('/icons/tray-muted.png').then((b) => Image.fromBytes(b)),
        loadIconBytes('/icons/tray-deafened.png').then((b) => Image.fromBytes(b)),
    ]);
    icons = { unmuted, muted, deafened };
}

function computeState(): TrayState {
    if (get(isDeafened)) return 'deafened';
    if (get(isMuted)) return 'muted';
    return 'unmuted';
}

const tooltips: Record<TrayState, string> = {
    unmuted: 'Etch',
    muted: 'Etch (Muted)',
    deafened: 'Etch (Deafened)',
};

let lastState: TrayState | null = null;
let lastMuteText: string | null = null;
let lastDeafenText: string | null = null;
let syncTimer: ReturnType<typeof setTimeout> | null = null;

function scheduleSyncTray(): void {
    if (syncTimer !== null) return;
    syncTimer = setTimeout(async () => {
        syncTimer = null;
        try {
            await doSyncTray();
        } catch (e) {
            console.error('Tray sync failed:', e);
        }
    }, 0);
}

async function doSyncTray(): Promise<void> {
    if (!tray || !icons) return;
    const state = computeState();
    if (state !== lastState) {
        await tray.setIcon(icons[state]);
        await tray.setTooltip(tooltips[state]);
        lastState = state;
    }
    const muteText = get(isMuted) ? 'Unmute' : 'Mute';
    if (muteItem && muteText !== lastMuteText) {
        await muteItem.setText(muteText);
        lastMuteText = muteText;
    }
    const deafenText = get(isDeafened) ? 'Undeafen' : 'Deafen';
    if (deafenItem && deafenText !== lastDeafenText) {
        await deafenItem.setText(deafenText);
        lastDeafenText = deafenText;
    }
}

async function doInitTray(): Promise<void> {
    await loadIcons();
    if (!icons) return;

    const win = getCurrentWindow();

    muteItem = await MenuItem.new({
        id: 'tray-toggle-mute',
        text: 'Mute',
        action: () => { toggleMute(); },
    });

    deafenItem = await MenuItem.new({
        id: 'tray-toggle-deafen',
        text: 'Deafen',
        action: () => { toggleDeafen(); },
    });

    const separator = await PredefinedMenuItem.new({ item: 'Separator' });

    const showHideItem = await MenuItem.new({
        id: 'tray-show-hide',
        text: 'Show/Hide',
        action: async () => {
            const visible = await win.isVisible();
            if (visible) {
                await win.hide();
            } else {
                await win.show();
                await win.setFocus();
            }
        },
    });

    const quitItem = await MenuItem.new({
        id: 'tray-quit',
        text: 'Quit',
        action: async () => { await win.close(); },
    });

    const menu = await Menu.new({
        items: [muteItem, deafenItem, separator, showHideItem, quitItem],
    });

    lastState = 'unmuted';
    lastMuteText = 'Mute';
    lastDeafenText = 'Deafen';

    tray = await TrayIcon.new({
        id: 'etch-tray',
        icon: icons.unmuted,
        tooltip: 'Etch',
        menu,
    });

    unsubscribers.push(
        isMuted.subscribe(() => { scheduleSyncTray(); }),
        isDeafened.subscribe(() => { scheduleSyncTray(); }),
    );
}

export function initTray(): void {
    if (!initPromise) {
        initPromise = doInitTray().catch((e) => {
            console.error('Tray init failed:', e);
            initPromise = null;
        });
    }
}

export function destroyTray(): void {
    for (const unsub of unsubscribers) unsub();
    unsubscribers = [];

    if (syncTimer !== null) {
        clearTimeout(syncTimer);
        syncTimer = null;
    }

    if (tray) {
        tray.close().catch((e) => console.error('Tray close failed:', e));
        tray = null;
    }
    muteItem = null;
    deafenItem = null;
    lastState = null;
    lastMuteText = null;
    lastDeafenText = null;
    icons = null;
    initPromise = null;
}
