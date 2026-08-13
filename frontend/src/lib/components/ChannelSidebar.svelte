<script lang="ts">
  import type { ChannelDto } from "$lib/bindings/ChannelDto";
  import { m } from "$lib/paraglide/messages.js";

  let {
    channels,
    selectedId,
    onselect,
    oncreate,
  }: {
    channels: ChannelDto[];
    selectedId: number | null;
    onselect: (id: number) => void;
    oncreate: () => void;
  } = $props();
</script>

<aside class="channel-sidebar" aria-label={m.channels()}>
  <div class="sidebar-heading">
    <h2>{m.channels()}</h2>
    <button
      class="icon-button"
      type="button"
      onclick={oncreate}
      aria-label={m.new_channel()}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 5v14M5 12h14" />
      </svg>
    </button>
  </div>
  <nav>
    {#each channels as channel (channel.id)}
      <button
        type="button"
        class:active={selectedId === channel.id}
        onclick={() => onselect(channel.id)}
      >
        <span class:disabled={!channel.enabled}></span>
        <span>
          <strong>{channel.name}</strong>
          <small>{channel.base_url}</small>
        </span>
      </button>
    {/each}
  </nav>
</aside>
