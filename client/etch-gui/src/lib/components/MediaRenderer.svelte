<script lang="ts">
    import { openImage, showToast } from '$lib/stores';
    import { fetchBlob } from '$lib/media';
    import { save } from '@tauri-apps/plugin-dialog';
    import { writeFile } from '@tauri-apps/plugin-fs';

    export let src: string;
    export let mimetype: string;
    export let body: string;

    async function downloadFile() {
        const dest = await save({ defaultPath: body || 'attachment' });
        if (!dest) return;

        try {
            const bytes = await fetchBlob(src);
            await writeFile(dest, bytes);
        } catch (e) {
            showToast(`Failed to download file: ${e}`);
        }
    }
</script>

<div class="media-attachment">
    {#if mimetype.startsWith('image/')}
        <img
            {src}
            alt={body}
            on:click={() => openImage(src)}
        />
    {:else}
        <button class="file-download" on:click={downloadFile}>
            <svg width="16" height="16" viewBox="0 0 24 24" class="file-icon">
                <path fill="currentColor" d="M14 2H6C4.9 2 4 2.9 4 4V20C4 21.1 4.9 22 6 22H18C19.1 22 20 21.1 20 20V8L14 2ZM18 20H6V4H13V9H18V20Z"/>
            </svg>
            <span class="file-name">{body}</span>
            <svg width="16" height="16" viewBox="0 0 24 24" class="download-icon">
                <path fill="currentColor" d="M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z"/>
            </svg>
        </button>
    {/if}
</div>

<style>
    .media-attachment { margin-top: 4px; }
    .media-attachment img {
        max-width: 400px;
        max-height: 300px;
        border-radius: 4px;
        cursor: pointer;
    }
    .file-download {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        padding: 10px 14px;
        background-color: #2f3136;
        border: 1px solid #202225;
        border-radius: 4px;
        color: #dcddde;
        cursor: pointer;
        font: inherit;
        transition: background-color 0.15s ease;
    }
    .file-download:hover { background-color: #36393f; }
    .file-icon { flex-shrink: 0; color: #7289da; }
    .file-name { color: #00aff4; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .download-icon { flex-shrink: 0; color: #b9bbbe; }
</style>
