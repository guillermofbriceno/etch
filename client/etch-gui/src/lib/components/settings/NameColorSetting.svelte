<script lang="ts">
    import { currentUser, nameColors, chosenColorOf } from '$lib/stores';
    import { sendCoreCommand } from '$lib/ipc';
    import { automaticHue, hexHue, hueColor, isReadable, parseHex, userColor } from '$lib/userColor';

    const TRACK_STEP = 15;
    const trackStops = Array.from({ length: 360 / TRACK_STEP + 1 }, (_, i) => hueColor(i * TRACK_STEP));
    const hueTrack = `linear-gradient(to right, ${trackStops.join(', ')})`;

    // The field's text, or null for Automatic; undefined until the user edits, so the controls follow the stored color until then.
    let draft: string | null | undefined = undefined;
    // Kept because a hex color only approximates the slider's hue, and reading it back would nudge the slider.
    let sliderHue: number | undefined = undefined;

    $: userId = $currentUser.matrixId;
    $: name = $currentUser.displayName ?? $currentUser.username;
    $: stored = chosenColorOf($nameColors, userId);
    $: automaticColor = userColor(userId, null);
    $: text = draft === undefined ? stored?.color ?? null : draft;
    $: hex = text === null ? null : parseHex(text);
    $: tooDark = hex !== null && !isReadable(hex);
    $: hue = sliderHue ?? (hex === null ? automaticHue(userId) : hexHue(hex));
    // What Apply sends; undefined while the field holds no usable color.
    $: choice = text === null ? null : hex !== null && !tooDark ? { color: hex } : undefined;
    $: canApply = choice !== undefined && choice?.color !== stored?.color;

    function edit(next: string | null, fromSlider?: number) {
        draft = next;
        sliderHue = fromSlider;
    }

    function apply() {
        if (choice === undefined) return;
        sendCoreCommand({ type: 'Matrix', data: { type: 'SetNameColor', data: choice } });
    }
</script>

<div class="setting-group" role="group" aria-labelledby="name-color-label">
    <label id="name-color-label" for="name-color-hex">Name Color</label>
    <div class="name-preview" style="color: {hex ?? automaticColor}">{name}</div>
    <input
        type="range"
        class="hue-slider"
        aria-label="Hue"
        min="0"
        max="359"
        value={hue}
        style="background: {hueTrack}"
        on:input={(e) => edit(hueColor(+e.currentTarget.value), +e.currentTarget.value)}
    />
    <input
        type="text"
        id="name-color-hex"
        class="text-input hex-input"
        spellcheck="false"
        autocomplete="off"
        value={text ?? automaticColor}
        on:input={(e) => edit(e.currentTarget.value)}
    />
    {#if tooDark}
        <span class="too-dark">Too dark to read on the chat background.</span>
    {/if}
    <label class="checkbox-option">
        <input type="checkbox" checked={text === null} on:change={(e) => edit(e.currentTarget.checked ? null : automaticColor)} />
        <span class="swatch" style="background-color: {automaticColor}"></span>
        <span class="checkbox-label">Automatic</span>
    </label>
    <button class="action-btn" on:click={apply} disabled={!canApply}>Apply</button>
</div>

<style>
    .name-preview {
        background-color: var(--bg-primary);
        border: 1px solid var(--border-input);
        border-radius: 4px;
        padding: 10px 12px;
        margin-bottom: 12px;
        font-size: var(--font-size-chat);
        font-weight: 500;
    }

    .hue-slider {
        height: 8px;
        margin: 0 0 12px;
        border-radius: 4px;
        -webkit-appearance: none;
        appearance: none;
        outline: none;
        cursor: pointer;
    }

    .hue-slider::-webkit-slider-thumb {
        -webkit-appearance: none;
        appearance: none;
        box-sizing: border-box;
        width: 16px;
        height: 16px;
        border-radius: 50%;
        background: var(--text-bright);
        border: 2px solid var(--bg-primary);
        cursor: pointer;
    }

    .hex-input {
        align-self: flex-start;
        width: 7em;
        margin-bottom: 12px;
    }

    .too-dark {
        color: var(--status-danger);
        font-size: 13px;
        margin-bottom: 12px;
    }

    .swatch {
        width: 12px;
        height: 12px;
        border-radius: 50%;
        flex-shrink: 0;
    }
</style>
