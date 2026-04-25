import { load } from '@tauri-apps/plugin-store';
import type { ServerBookmark } from './types';

const STORE_FILE = 'config.json';
const BOOKMARKS_KEY = 'bookmarkedServers';

async function getStore() {
    return await load(STORE_FILE, { autoSave: true, defaults: {} });
}

export async function getBookmarkedServers(): Promise<ServerBookmark[]> {
    const store = await getStore();
    return (await store.get<ServerBookmark[]>(BOOKMARKS_KEY)) ?? [];
}

export async function addBookmarkedServer(server: ServerBookmark): Promise<void> {
    const servers = await getBookmarkedServers();
    servers.push(server);
    const store = await getStore();
    await store.set(BOOKMARKS_KEY, servers);
}

export async function removeBookmarkedServer(id: string): Promise<void> {
    const servers = await getBookmarkedServers();
    const filtered = servers.filter((s) => s.id !== id);
    const store = await getStore();
    await store.set(BOOKMARKS_KEY, filtered);
}

export async function updateBookmarkedServer(id: string, updates: Partial<ServerBookmark>): Promise<void> {
    const servers = await getBookmarkedServers();
    const index = servers.findIndex((s) => s.id === id);
    if (index !== -1) {
        servers[index] = { ...servers[index], ...updates };
        const store = await getStore();
        await store.set(BOOKMARKS_KEY, servers);
    }
}
