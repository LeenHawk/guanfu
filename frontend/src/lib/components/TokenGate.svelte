<script lang="ts">
  import { setAuthToken } from "$lib/api/transport";
  import { m } from "$lib/paraglide/messages.js";

  let { open, onsaved }: { open: boolean; onsaved: () => void } = $props();

  let token = $state("");

  function submit(event: SubmitEvent) {
    event.preventDefault();
    setAuthToken(token.trim());
    token = "";
    onsaved();
  }
</script>

{#if open}
  <div class="modal-backdrop" role="presentation">
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="token-title"
    >
      <h2 id="token-title">{m.token_prompt()}</h2>
      <form onsubmit={submit}>
        <label
          >{m.token_label()}<input
            type="password"
            autocomplete="current-password"
            bind:value={token}
            required
          /></label
        >
        <footer>
          <button class="button primary" type="submit">{m.token_save()}</button>
        </footer>
      </form>
    </div>
  </div>
{/if}
