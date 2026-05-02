<script lang="ts">
    export let initial: string;
    export let size: number;
    export let fontSize: number | undefined = undefined;

    $: resolvedFontSize = fontSize ?? Math.round(size * 0.55);
</script>

<span
    class="avatar-fallback"
    style="width: {size}px; height: {size}px; font-size: {resolvedFontSize}px;"
>{initial}</span>

<style>
    .avatar-fallback {
        display: flex;
        align-items: center;
        justify-content: center;
        border-radius: 50%;
        background-color: #5865f2;
        color: #fff;
        font-weight: 600;
        flex-shrink: 0;
    }
</style>
