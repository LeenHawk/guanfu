<script lang="ts">
  import { m } from "$lib/paraglide/messages.js";

  let {
    open,
    message,
    submitting,
    onconfirm,
    oncancel,
  }: {
    open: boolean;
    message: string;
    submitting: boolean;
    onconfirm: () => void;
    oncancel: () => void;
  } = $props();
</script>

{#if open}
  <div class="modal-backdrop" role="presentation">
    <div
      class="modal confirm-modal"
      role="alertdialog"
      aria-modal="true"
      aria-describedby="confirm-message"
    >
      <p id="confirm-message">{message}</p>
      <footer>
        <button
          class="button secondary"
          type="button"
          onclick={oncancel}
          disabled={submitting}>{m.cancel()}</button
        >
        <button
          class="button danger"
          type="button"
          onclick={onconfirm}
          disabled={submitting}>{m.delete()}</button
        >
      </footer>
    </div>
  </div>
{/if}
