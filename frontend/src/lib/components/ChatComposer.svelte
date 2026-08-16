<script lang="ts">
  import type { AssetHeadDto } from "$lib/bindings/AssetHeadDto";
  import type { ChannelDto } from "$lib/bindings/ChannelDto";
  import { m } from "$lib/paraglide/messages.js";

  let {
    channels,
    characters,
    channelId = $bindable(),
    model = $bindable(),
    characterId = $bindable(),
    running,
    onsend,
    onstop,
    onretry,
  }: {
    channels: ChannelDto[];
    characters: AssetHeadDto[];
    channelId: number | null;
    model: string;
    characterId: number | null;
    running: boolean;
    onsend: (text: string) => void;
    onstop: () => void;
    onretry: () => void;
  } = $props();

  let draft = $state("");

  function submit(event: SubmitEvent) {
    event.preventDefault();
    const text = draft.trim();
    if (!text || running) return;
    draft = "";
    onsend(text);
  }

  function onkeydown(event: KeyboardEvent) {
    // Enter 发送,Shift+Enter 换行——长文本仍可分段。
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      (event.currentTarget as HTMLElement).closest("form")?.requestSubmit();
    }
  }
</script>

<form class="chat-composer" onsubmit={submit}>
  <div class="composer-controls">
    <label>
      <span class="sr-only">{m.characters()}</span>
      <select bind:value={characterId} aria-label={m.characters()}>
        <option value={null}>{m.no_character()}</option>
        {#each characters as character (character.id)}
          <option value={character.id}>{character.name}</option>
        {/each}
      </select>
    </label>
    <label>
      <span class="sr-only">{m.chat_channel()}</span>
      <select bind:value={channelId} aria-label={m.chat_channel()}>
        {#if channels.length === 0}
          <option value={null}>{m.no_channel()}</option>
        {/if}
        {#each channels as channel (channel.id)}
          <option value={channel.id}>{channel.name}</option>
        {/each}
      </select>
    </label>
    <label>
      <span class="sr-only">{m.chat_model()}</span>
      <input bind:value={model} aria-label={m.chat_model()} required />
    </label>
    <button
      class="text-button"
      type="button"
      onclick={onretry}
      disabled={running}>{m.retry()}</button
    >
  </div>
  <div class="composer-row">
    <textarea
      bind:value={draft}
      {onkeydown}
      rows="2"
      placeholder={m.message_placeholder()}
      aria-label={m.message_placeholder()}></textarea>
    {#if running}
      <button class="button secondary" type="button" onclick={onstop}
        >{m.stop()}</button
      >
    {:else}
      <button class="button primary" type="submit" disabled={!channelId}
        >{m.send()}</button
      >
    {/if}
  </div>
</form>
