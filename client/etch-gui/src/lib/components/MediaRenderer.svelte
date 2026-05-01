<script lang="ts">
    import { openImage, showToast } from '$lib/stores';
    import Icon from './Icon.svelte';
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
        <button class="image-btn" on:click={() => openImage(src)}>
            <img {src} alt={body} />
        </button>
    {:else}
        <button class="file-download" on:click={downloadFile}>
            <Icon name="file" size={16} class="file-icon" />
            <span class="file-name">{body}</span>
            <Icon name="download" size={16} class="download-icon" />
        </button>
    {/if}
</div>

<style>
    .media-attachment { margin-top: 4px; }
    .image-btn {
        display: inline-block;
        background: none;
        border: none;
        padding: 0;
        cursor: pointer;
        line-height: 0;
    }
    .media-attachment img {
        max-width: 400px;
        max-height: 300px;
        border-radius: 4px;
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
    .file-download :global(.file-icon) { flex-shrink: 0; color: #7289da; }
    .file-name { color: #00aff4; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .file-download :global(.download-icon) { flex-shrink: 0; color: #b9bbbe; }
</style>
