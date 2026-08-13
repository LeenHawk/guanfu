<script lang="ts">
  import { m } from "$lib/paraglide/messages.js";

  let {
    open,
    submitting,
    onclose,
    onsubmit,
  }: {
    open: boolean;
    submitting: boolean;
    onclose: () => void;
    onsubmit: (name: string, baseUrl: string) => Promise<void>;
  } = $props();

  let name = $state("");
  let baseUrl = $state("");

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    await onsubmit(name.trim(), baseUrl.trim());
    name = "";
    baseUrl = "";
  }
</script>

{#if open}
  <div class="modal-backdrop" role="presentation">
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="new-channel-title"
    >
      <h2 id="new-channel-title">{m.new_channel()}</h2>
      <form onsubmit={submit}>
        <label>{m.channel_name()}<input required bind:value={name} /></label>
        <label
          >{m.base_url()}<input
            required
            type="url"
            placeholder="https://api.example.com"
            bind:value={baseUrl}
          /></label
        >
        <footer>
          <button class="button secondary" type="button" onclick={onclose}
            >{m.cancel()}</button
          >
          <button class="button primary" type="submit" disabled={submitting}
            >{m.create()}</button
          >
        </footer>
      </form>
    </div>
  </div>
{/if}
