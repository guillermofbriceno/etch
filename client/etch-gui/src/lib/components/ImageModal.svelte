<script lang="ts">
    import { closeOverlay, showToast } from '$lib/stores';
    import { fetchBlob } from '$lib/media';
    import { writeFile } from '@tauri-apps/plugin-fs';
    import { tempDir, join } from '@tauri-apps/api/path';
    import { openPath } from '@tauri-apps/plugin-opener';

    export let url: string;

    function handleKeydown(event: KeyboardEvent) {
        if (event.key === 'Escape') closeOverlay();
    }

    async function openOriginal() {
        try {
            const bytes = await fetchBlob(url);

            const urlPath = new URL(url).pathname;
            const urlExt = urlPath.includes('.') ? urlPath.split('.').pop() : null;
            const ext = urlExt || 'png';

            const tmp = await tempDir();
            const tmpPath = await join(tmp, `etch-preview-${Date.now()}.${ext}`);

            await writeFile(tmpPath, bytes);
            await openPath(tmpPath);
        } catch (e) {
            showToast(`Failed to open image: ${e}`);
        }
    }
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="image-modal">
    <div class="media-container" on:click|stopPropagation>
        <img src={url} alt="Expanded media" class="expanded-image" />

        <div class="media-actions">
            <button class="open-link" on:click={openOriginal}>Open original</button>
        </div>
    </div>
</div>

<style>
    .image-modal {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        width: 100%;
        height: 100%;
    }

    .media-container {
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        max-width: 90vw;
        max-height: 90vh;
    }

    .expanded-image {
        max-width: 100%;
        max-height: calc(90vh - 40px);
        object-fit: contain;
        border-radius: 4px;
        box-shadow: 0 8px 16px rgba(0, 0, 0, 0.24);
        user-select: none;
    }

    .media-actions { margin-top: 8px; padding-left: 4px; }

    .open-link {
        background: none;
        border: none;
        padding: 0;
        color: #00aff4;
        font-size: 14px;
        font-weight: 500;
        text-decoration: none;
        font-family: 'Inter', sans-serif;
        cursor: pointer;
        transition: text-decoration 0.1s ease;
    }

    .open-link:hover { text-decoration: underline; }
</style>
