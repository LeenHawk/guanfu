<script lang="ts">
  import type { AssetHeadDto } from "$lib/bindings/AssetHeadDto";
  import type { ChannelDto } from "$lib/bindings/ChannelDto";
  import type { VideoJob } from "$lib/bindings/VideoJob";
  import { mediaApi } from "$lib/api/media";
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

  let model = $state("sora-2");
  let prompt = $state("");
  let seconds = $state(4);
  let job = $state<VideoJob | null>(null);
  let running = $state(false);
  let downloaded = $state<AssetHeadDto | null>(null);

  async function create(event: SubmitEvent) {
    event.preventDefault();
    if (!channelId) return;
    running = true;
    downloaded = null;
    try {
      job = await mediaApi.createVideo({
        channel_id: channelId,
        name: prompt.slice(0, 24) || model,
        request: {
          model,
          prompt,
          seconds,
          size: null,
          input_reference: null,
        },
      });
    } catch (error) {
      onerror(error);
    } finally {
      running = false;
    }
  }

  async function poll() {
    if (!channelId || !job) return;
    running = true;
    try {
      job = await mediaApi.pollVideo(channelId, job.id);
    } catch (error) {
      onerror(error);
    } finally {
      running = false;
    }
  }

  async function download() {
    if (!channelId || !job?.content_ref) return;
    running = true;
    try {
      const asset = await mediaApi.downloadVideo(
        channelId,
        job.content_ref,
        prompt.slice(0, 24) || model,
      );
      downloaded = asset;
      onsaved([asset]);
    } catch (error) {
      onerror(error);
    } finally {
      running = false;
    }
  }
</script>

<form class="inline-form studio-form" onsubmit={create}>
  <label
    >{m.chat_channel()}<select bind:value={channelId}>
      {#each channels as channel (channel.id)}
        <option value={channel.id}>{channel.name}</option>
      {/each}
    </select></label
  >
  <label>{m.chat_model()}<input bind:value={model} required /></label>
  <label
    >{m.video_seconds()}<input
      type="number"
      min="1"
      max="60"
      bind:value={seconds}
    /></label
  >
  <label class="wide"
    >{m.prompt()}<textarea rows="2" bind:value={prompt} required
    ></textarea></label
  >
  <button class="button primary" type="submit" disabled={running || !channelId}
    >{m.generate()}</button
  >
</form>

{#if job}
  <div class="data-row">
    <div>
      <strong>{m.video_status()}</strong>
      <small>{job.status}{job.progress ? ` · ${job.progress}%` : ""}</small>
    </div>
    <button class="text-button" type="button" onclick={poll} disabled={running}
      >{m.poll()}</button
    >
    {#if job.content_ref}
      <button
        class="text-button"
        type="button"
        onclick={download}
        disabled={running}>{m.download()}</button
      >
    {/if}
  </div>
{/if}

{#if downloaded}
  {#await import("$lib/api/media").then( (module) => module.mediaSrc(downloaded!.id) ) then src}
    <!-- svelte-ignore a11y_media_has_caption -->
    <video class="media-player" {src} controls></video>
  {/await}
{/if}
