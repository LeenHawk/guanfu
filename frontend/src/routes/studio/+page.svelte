<script lang="ts">
  import type { AssetHeadDto } from "$lib/bindings/AssetHeadDto";
  import type { ChannelDto } from "$lib/bindings/ChannelDto";
  import { api } from "$lib/api/channels";
  import { chatApi } from "$lib/api/chat";
  import { ApiClientError } from "$lib/api/error";
  import AppHeader from "$lib/components/AppHeader.svelte";
  import TokenGate from "$lib/components/TokenGate.svelte";
  import AudioStudio from "$lib/components/AudioStudio.svelte";
  import ImageStudio from "$lib/components/ImageStudio.svelte";
  import MediaGallery from "$lib/components/MediaGallery.svelte";
  import RealtimeStudio from "$lib/components/RealtimeStudio.svelte";
  import VideoStudio from "$lib/components/VideoStudio.svelte";
  import { messageForError } from "$lib/i18n/errors";
  import { m } from "$lib/paraglide/messages.js";

  type Tab = "image" | "video" | "audio" | "realtime";

  let tab = $state<Tab>("image");
  let channels = $state<ChannelDto[]>([]);
  let channelId = $state<number | null>(null);
  let media = $state<AssetHeadDto[]>([]);
  let errorCode = $state<string | null>(null);

  $effect(() => {
    void load();
  });

  function captureError(error: unknown) {
    errorCode =
      error instanceof ApiClientError
        ? error.payload.code
        : typeof error === "object" && error && "code" in error
          ? String((error as { code: unknown }).code)
          : "upstream_unavailable";
  }

  async function load() {
    try {
      [channels, media] = await Promise.all([
        api.listChannels(),
        chatApi.listAssets("media"),
      ]);
      channelId ??= channels.find((channel) => channel.enabled)?.id ?? null;
    } catch (error) {
      captureError(error);
    }
  }

  function onsaved(assets: AssetHeadDto[]) {
    media = [...assets, ...media];
  }

  const tabs: { id: Tab; label: () => string }[] = [
    { id: "image", label: m.studio_image },
    { id: "video", label: m.studio_video },
    { id: "audio", label: m.studio_audio },
    { id: "realtime", label: m.studio_realtime },
  ];
</script>

<svelte:head><title>{m.app_title()} · {m.nav_studio()}</title></svelte:head>

<div class="app-shell">
  <AppHeader />
  {#if errorCode}<div class="error-banner" role="alert">
      <span>{messageForError(errorCode)}</span>
      <button
        class="banner-close"
        type="button"
        aria-label={m.dismiss()}
        onclick={() => (errorCode = null)}>×</button
      >
    </div>{/if}
  <main class="workspace">
    <header class="workspace-header">
      <div><h2>{m.nav_studio()}</h2></div>
    </header>
    <nav class="studio-tabs" aria-label={m.nav_studio()}>
      {#each tabs as entry (entry.id)}
        <button
          class="text-button"
          class:active={tab === entry.id}
          type="button"
          aria-current={tab === entry.id ? "page" : undefined}
          onclick={() => (tab = entry.id)}>{entry.label()}</button
        >
      {/each}
    </nav>

    <section class="config-section">
      {#if tab === "image"}
        <ImageStudio
          {channels}
          bind:channelId
          onerror={captureError}
          {onsaved}
        />
      {:else if tab === "video"}
        <VideoStudio
          {channels}
          bind:channelId
          onerror={captureError}
          {onsaved}
        />
      {:else if tab === "audio"}
        <AudioStudio
          {channels}
          bind:channelId
          onerror={captureError}
          {onsaved}
        />
      {:else}
        <RealtimeStudio {channels} bind:channelId onerror={captureError} />
      {/if}
    </section>

    <section class="config-section" aria-labelledby="gallery-title">
      <header>
        <div><h3 id="gallery-title">{m.media_gallery()}</h3></div>
      </header>
      <MediaGallery assets={media} />
    </section>
  </main>
</div>

<TokenGate
  open={errorCode === "unauthorized"}
  onsaved={() => {
    errorCode = null;
    void load();
  }}
/>
