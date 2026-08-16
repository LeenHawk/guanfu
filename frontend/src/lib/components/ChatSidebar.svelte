<script lang="ts">
  import type { AssetHeadDto } from "$lib/bindings/AssetHeadDto";
  import { m } from "$lib/paraglide/messages.js";

  let {
    histories,
    characters,
    selectedId,
    submitting,
    onselect,
    oncreate,
    onimport,
  }: {
    histories: AssetHeadDto[];
    characters: AssetHeadDto[];
    selectedId: number | null;
    submitting: boolean;
    onselect: (id: number) => void;
    oncreate: () => void;
    onimport: (file: File) => void;
  } = $props();

  let fileInput: HTMLInputElement;

  function pickFile(event: Event) {
    const file = (event.currentTarget as HTMLInputElement).files?.[0];
    if (file) onimport(file);
    (event.currentTarget as HTMLInputElement).value = "";
  }
</script>

<nav class="channel-sidebar" aria-label={m.nav_chat()}>
  <div class="sidebar-heading">
    <h2>{m.nav_chat()}</h2>
    <button
      class="icon-button"
      type="button"
      onclick={oncreate}
      disabled={submitting}
      aria-label={m.new_chat()}>+</button
    >
  </div>
  <ul class="sidebar-list">
    {#each histories as history (history.id)}
      <li>
        <button
          class="sidebar-item"
          class:active={history.id === selectedId}
          type="button"
          onclick={() => onselect(history.id)}
        >
          <strong>{history.name}</strong>
        </button>
      </li>
    {/each}
  </ul>

  <div class="sidebar-heading">
    <h2>{m.characters()}</h2>
    <button
      class="icon-button"
      type="button"
      onclick={() => fileInput.click()}
      disabled={submitting}
      aria-label={m.import_character()}>↑</button
    >
  </div>
  <input
    bind:this={fileInput}
    class="sr-only"
    type="file"
    accept=".png,.json"
    onchange={pickFile}
  />
  <ul class="sidebar-list">
    {#each characters as character (character.id)}
      <li><span class="sidebar-item static">{character.name}</span></li>
    {/each}
  </ul>
</nav>
