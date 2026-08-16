<script lang="ts">
  import type { AssetHeadDto } from "$lib/bindings/AssetHeadDto";
  import type { ChannelDto } from "$lib/bindings/ChannelDto";
  import { mediaApi, mediaSrc } from "$lib/api/media";
  import { m } from "$lib/paraglide/messages.js";

  let {
    channels,
    channelId = $bindable(),
    onerror,
    onsaved,
  }: {
    channels: ChannelDto[];
    channelId: number | null;
    onerror: (error: unknown) => void;
    onsaved: (assets: AssetHeadDto[]) => void;
  } = $props();

  let speechModel = $state("gpt-4o-mini-tts");
  let voice = $state("alloy");
  let text = $state("");
  let spoken = $state<AssetHeadDto | null>(null);

  let transcribeModel = $state("gpt-4o-mini-transcribe");
  let transcript = $state("");
  let running = $state(false);

  async function synthesize(event: SubmitEvent) {
    event.preventDefault();
    if (!channelId) return;
    running = true;
    try {
      spoken = await mediaApi.speech({
        channel_id: channelId,
        name: text.slice(0, 24) || speechModel,
        request: {
          model: speechModel,
          input: text,
          voice,
          instructions: null,
          format: "mp3",
          speed: null,
          mode: "complete",
        },
      });
      onsaved([spoken]);
    } catch (error) {
      onerror(error);
    } finally {
      running = false;
    }
  }

  async function transcribe(event: Event) {
    const file = (event.currentTarget as HTMLInputElement).files?.[0];
    (event.currentTarget as HTMLInputElement).value = "";
    if (!file || !channelId) return;
    running = true;
    try {
      const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
      const result = await mediaApi.transcribe({
        channel_id: channelId,
        name: "",
        request: {
          model: transcribeModel,
          audio: {
            type: "data",
            media_type: file.type || "audio/mpeg",
            bytes,
          },
          language: null,
          prompt: null,
          temperature: null,
          timestamps: [],
          diarization: null,
          mode: "complete",
        },
      });
      transcript = result.text;
    } catch (error) {
      onerror(error);
    } finally {
      running = false;
    }
  }
</script>

<form class="inline-form studio-form" onsubmit={synthesize}>
  <label
    >{m.chat_channel()}<select bind:value={channelId}>
      {#each channels as channel (channel.id)}
        <option value={channel.id}>{channel.name}</option>
      {/each}
    </select></label
  >
  <label>{m.chat_model()}<input bind:value={speechModel} required /></label>
  <label>{m.voice()}<input bind:value={voice} required /></label>
  <label class="wide"
    >{m.speech_text()}<textarea rows="2" bind:value={text} required
    ></textarea></label
  >
  <button class="button primary" type="submit" disabled={running || !channelId}
    >{m.synthesize()}</button
  >
</form>

{#if spoken}
  {#await mediaSrc(spoken.id) then src}
    <audio class="media-player" {src} controls></audio>
  {/await}
{/if}

<div class="inline-form studio-form">
  <label>{m.chat_model()}<input bind:value={transcribeModel} /></label>
  <label class="wide"
    >{m.pick_audio()}<input
      type="file"
      accept="audio/*"
      onchange={transcribe}
      disabled={running || !channelId}
    /></label
  >
</div>

{#if transcript}
  <h4>{m.transcript()}</h4>
  <pre class="invocation-result">{transcript}</pre>
{/if}
