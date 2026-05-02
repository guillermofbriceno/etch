<script lang="ts">
    import { sfxVolume, transmissionMode, setTransmissionMode, vadThreshold, setVadThreshold, voiceHold, setVoiceHold, useMumbleSettings, setUseMumbleSettings, deafenSuppressesNotifs, setDeafenSuppressesNotifs } from '$lib/stores';
    import type { TransmissionMode } from '$lib/stores';

    function onInputProfileChange(mode: TransmissionMode) {
        setTransmissionMode(mode);
    }
</script>

<div class="tab-pane">
    <h2>Voice & Audio</h2>

    <h3 class="section-header">Voice</h3>

    <div class="setting-group">
        <label class="checkbox-option">
            <input type="checkbox" checked={$useMumbleSettings} on:change={(e) => setUseMumbleSettings(e.currentTarget.checked)} />
            <span class="checkbox-label">Use Mumble's Settings</span>
        </label>
        <p class="setting-desc">Defer to Mumble's built-in voice settings. Requires Restart.</p>
    </div>

    <div class="setting-group" class:disabled={$useMumbleSettings}>
        <span class="setting-label">Input Profile</span>
        <div class="radio-group">
            <label class="radio-option">
                <input type="radio" name="input-profile" value="voice_activation" checked={$transmissionMode === 'voice_activation'} on:change={() => onInputProfileChange('voice_activation')} />
                <div class="radio-content">
                    <span class="radio-label">Voice Isolation</span>
                    <span class="radio-desc">Automatically transmits when you speak</span>
                </div>
            </label>
            <label class="radio-option">
                <input type="radio" name="input-profile" value="continuous" checked={$transmissionMode === 'continuous'} on:change={() => onInputProfileChange('continuous')} />
                <div class="radio-content">
                    <span class="radio-label">Continuous</span>
                    <span class="radio-desc">Always transmitting audio</span>
                </div>
            </label>
            <label class="radio-option">
                <input type="radio" name="input-profile" value="push_to_talk" checked={$transmissionMode === 'push_to_talk'} on:change={() => onInputProfileChange('push_to_talk')} />
                <div class="radio-content">
                    <span class="radio-label">Push to Talk</span>
                    <span class="radio-desc">Transmits only while a key is held</span>
                </div>
            </label>
        </div>
    </div>

    <div class="setting-group volume-slider" class:disabled={$useMumbleSettings || $transmissionMode !== 'voice_activation'}>
        <label for="speech-threshold">Speech Threshold</label>
        <div class="slider-container">
            <input type="range" id="speech-threshold" min="0" max="100" value={$vadThreshold} on:change={(e) => setVadThreshold(+e.currentTarget.value)} class="range-input" disabled={$useMumbleSettings || $transmissionMode !== 'voice_activation'} />
            <span class="volume-readout">{$vadThreshold}%</span>
        </div>
    </div>

    <div class="setting-group volume-slider" class:disabled={$useMumbleSettings}>
        <label for="voice-hold">Voice Hold</label>
        <div class="slider-container">
            <input type="range" id="voice-hold" min="50" max="1000" step="10" value={$voiceHold} on:change={(e) => setVoiceHold(+e.currentTarget.value)} class="range-input" />
            <span class="volume-readout">{$voiceHold}ms</span>
        </div>
    </div>

    <h3 class="section-header">Audio</h3>

    <div class="setting-group volume-slider">
        <label for="sfx-volume">Sound Effects Volume</label>
        <div class="slider-container">
            <input type="range" id="sfx-volume" min="0" max="100" bind:value={$sfxVolume} class="range-input" />
            <span class="volume-readout">{$sfxVolume}%</span>
        </div>
    </div>

    <div class="setting-group">
        <label class="checkbox-option">
            <input type="checkbox" checked={$deafenSuppressesNotifs} on:change={(e) => setDeafenSuppressesNotifs(e.currentTarget.checked)} />
            <span class="checkbox-label">Deafen suppresses new message notifications</span>
        </label>
        <p class="setting-desc">When unchecked, new message sounds play even while deafened.</p>
    </div>
</div>
