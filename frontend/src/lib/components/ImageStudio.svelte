<script lang="ts">
  import type { AssetHeadDto } from "$lib/bindings/AssetHeadDto";
  import type { ChannelDto } from "$lib/bindings/ChannelDto";
  import { mediaApi } from "$lib/api/media";
  import MediaGallery from "$lib/components/MediaGallery.svelte";
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

  let model = $state("gpt-image-1");
  let prompt = $state("");
  let size = $state("1024x1024");
  let count = $state(1);
  let running = $state(false);
  let produced = $state<AssetHeadDto[]>([]);
  let urls = $state<string[]>([]);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!channelId) return;
    running = true;
    try {
      const [width, height] = size.split("x").map(Number);
      const result = await mediaApi.generateImage({
        channel_id: channelId,
        name: prompt.slice(0, 24) || model,
        request: {
          model,
          prompt,
          count,
          options: {
            width,
            height,
            quality: null,
            background: null,
            output_format: null,
            compression: null,
            moderation: null,
            partial_images: null,
          },
          mode: "complete",
        },
      });
      produced = result.assets;
      urls = result.urls;
      onsaved(result.assets);
    } catch (error) {
      onerror(error);
    } finally {
      running = false;
    }
  }
</script>

<form class="inline-form studio-form" onsubmit={submit}>
  <label
    >{m.chat_channel()}<select bind:value={channelId}>
      {#each channels as channel (channel.id)}
        <option value={channel.id}>{channel.name}</option>
      {/each}
    </select></label
  >
  <label>{m.chat_model()}<input bind:value={model} required /></label>
  <label>{m.size()}<input bind:value={size} /></label>
  <label
    >{m.count()}<input
      type="number"
      min="1"
      max="4"
      bind:value={count}
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

<MediaGallery assets={produced} {urls} />
