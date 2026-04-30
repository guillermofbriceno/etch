<script lang="ts">
    import { icons } from '$lib/icons';
    import type { IconName } from '$lib/icons';

    export let name: IconName;
    export let size: number = 24;
    let className = '';
    export { className as class };
</script>

<svg class={className || undefined} width={size} height={size} viewBox="0 0 24 24">
    {@html icons[name]}
</svg>
