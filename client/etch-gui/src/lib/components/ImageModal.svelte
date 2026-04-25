<script lang="ts">
    import { closeOverlay } from '$lib/stores';

    export let url: string;

    function handleKeydown(event: KeyboardEvent) {
        if (event.key === 'Escape') closeOverlay();
    }
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="image-modal">
    <div class="media-container" on:click|stopPropagation>
        <img src={url} alt="Expanded media" class="expanded-image" />

        <div class="media-actions">
            <a href={url} target="_blank" rel="noopener noreferrer" class="open-link">
                Open original
            </a>
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
        color: #00aff4;
        font-size: 14px;
        font-weight: 500;
        text-decoration: none;
        font-family: 'Inter', sans-serif;
        transition: text-decoration 0.1s ease;
    }

    .open-link:hover { text-decoration: underline; }
</style>
