<script lang="ts">
    import { openImage } from '$lib/stores';

    export let src: string;
    export let mimetype: string;
    export let body: string;
</script>

<div class="media-attachment">
    {#if mimetype.startsWith('image/')}
        <img
            {src}
            alt={body}
            on:click={() => openImage(src)}
        />
    {:else if mimetype.startsWith('video/')}
        <video controls>
            <source {src} type={mimetype} />
        </video>
    {:else if mimetype.startsWith('audio/')}
        <audio controls>
            <source {src} type={mimetype} />
        </audio>
    {:else}
        <a class="file-download" href={src} target="_blank" rel="noopener noreferrer">
            {body}
        </a>
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
    .media-attachment video {
        max-width: 400px;
        max-height: 300px;
        border-radius: 4px;
    }
    .media-attachment audio { max-width: 400px; }
    .file-download {
        display: inline-flex;
        align-items: center;
        padding: 8px 12px;
        background-color: #2f3136;
        border: 1px solid #202225;
        border-radius: 4px;
        color: #00aff4;
        text-decoration: none;
    }
    .file-download:hover { text-decoration: underline; }
</style>
