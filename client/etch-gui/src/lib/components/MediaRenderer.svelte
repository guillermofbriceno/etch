<script lang="ts">
    import { onDestroy } from 'svelte';
    import { openImage, showToast } from '$lib/stores';
    import Icon from './Icon.svelte';
    import { INLINE_PLAYBACK_MAX_BYTES, fetchBlob, fitWithin, formatSize, saveFileAs } from '$lib/media';

    export let src: string;
    export let mimetype: string;
    export let body: string;
    export let size = 0;
    export let width = 0;
    export let height = 0;

    const MAX_WIDTH = 400;
    const MAX_HEIGHT = 300;

    // A format the webview cannot play still has to be downloadable.
    let playbackFailed = false;
    let playbackUrl: string | null = null;
    let loadToken = 0;

    // The protocol never serves SVG as an image, so it is offered as a file.
    $: baseKind = mimetype.startsWith('image/') && mimetype !== 'image/svg+xml' ? 'image'
        : mimetype.startsWith('video/') ? 'video'
        : mimetype.startsWith('audio/') ? 'audio'
        : 'file';
    $: playable = (baseKind === 'video' || baseKind === 'audio') && size <= INLINE_PLAYBACK_MAX_BYTES;
    $: kind = baseKind === 'image' ? 'image' : playable && !playbackFailed ? baseKind : 'file';

    $: loadPlayback(playable ? src : null, mimetype);

    // WebKitGTK's GStreamer source only reads http(s) and blob URLs, so players get a blob.
    async function loadPlayback(url: string | null, type: string) {
        const token = ++loadToken;
        releasePlayback();
        playbackFailed = false;
        if (!url) return;
        try {
            const bytes = await fetchBlob(url);
            if (token !== loadToken) return;
            playbackUrl = URL.createObjectURL(new Blob([bytes], { type }));
        } catch {
            if (token === loadToken) playbackFailed = true;
        }
    }

    function releasePlayback() {
        if (playbackUrl) URL.revokeObjectURL(playbackUrl);
        playbackUrl = null;
    }

    function playbackError() {
        playbackFailed = true;
        releasePlayback();
    }

    onDestroy(() => {
        loadToken++;
        releasePlayback();
    });

    $: box = fitWithin(width, height, MAX_WIDTH, MAX_HEIGHT);
    $: reserved = box ? `width: ${box.width}px; aspect-ratio: ${width} / ${height};` : undefined;

    async function downloadFile() {
        try {
            await saveFileAs(body || 'attachment', await fetchBlob(src));
        } catch (e) {
            showToast(`Failed to download file: ${e}`);
        }
    }
</script>

<div class="media-attachment">
    {#if kind === 'image'}
        <button class="image-btn" on:click={() => openImage(src)}>
            <img {src} alt={body} style={reserved} />
        </button>
    {:else if kind === 'video'}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video controls preload="metadata" src={playbackUrl ?? undefined} style={reserved} on:error={playbackError}></video>
    {:else if kind === 'audio'}
        <div class="audio-card">
            <div class="audio-label">
                <span class="file-name">{body}</span>
                {#if size > 0}<span class="file-size">{formatSize(size)}</span>{/if}
            </div>
            <audio controls preload="metadata" src={playbackUrl ?? undefined} on:error={playbackError}></audio>
        </div>
    {:else}
        <button class="file-download" on:click={downloadFile}>
            <Icon name="file" size={16} class="file-icon" />
            <span class="file-name">{body}</span>
            {#if size > 0}<span class="file-size">{formatSize(size)}</span>{/if}
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
    .media-attachment img,
    .media-attachment video {
        max-width: 400px;
        max-height: 300px;
        border-radius: 4px;
        object-fit: contain;
    }
    .media-attachment video {
        display: block;
        background-color: #000;
    }
    .audio-card {
        display: inline-flex;
        flex-direction: column;
        gap: 8px;
        max-width: 400px;
        padding: 10px 14px;
        background-color: #2f3136;
        border: 1px solid var(--border-subtle);
        border-radius: 4px;
    }
    .audio-label {
        display: flex;
        align-items: baseline;
        gap: 8px;
        min-width: 0;
    }
    .audio-card audio { width: 320px; max-width: 100%; }
    .file-download {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        padding: 10px 14px;
        background-color: #2f3136;
        border: 1px solid var(--border-subtle);
        border-radius: 4px;
        color: var(--text-primary);
        cursor: pointer;
        font: inherit;
        transition: background-color 0.15s ease;
    }
    .file-download:hover { background-color: #36393f; }
    .file-download :global(.file-icon) { flex-shrink: 0; color: #7289da; }
    .file-name { color: var(--text-link); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .file-size { flex-shrink: 0; color: var(--text-muted); font-size: 12px; }
    .file-download :global(.download-icon) { flex-shrink: 0; color: var(--text-secondary); }
</style>
