<script lang="ts">
  import type { CredentialDto } from "$lib/bindings/CredentialDto";
  import { m } from "$lib/paraglide/messages.js";

  let {
    credentials,
    submitting,
    onadd,
    onremove,
  }: {
    credentials: CredentialDto[];
    submitting: boolean;
    onadd: (label: string, secret: string, weight: number) => Promise<void>;
    onremove: (id: number) => void;
  } = $props();

  let adding = $state(false);
  let label = $state("");
  let secret = $state("");
  let weight = $state(1);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    await onadd(label.trim(), secret, weight);
    label = "";
    secret = "";
    weight = 1;
    adding = false;
  }
</script>

<section class="config-section" aria-labelledby="credentials-title">
  <header>
    <div>
      <h3 id="credentials-title">{m.credentials()}</h3>
      <span>{credentials.length}</span>
    </div>
    <button class="text-button" type="button" onclick={() => (adding = !adding)}
      >{m.add_credential()}</button
    >
  </header>
  {#if adding}
    <form class="inline-form credentials-form" onsubmit={submit}>
      <label>{m.credential_label()}<input required bind:value={label} /></label>
      <label
        >{m.credential_secret()}<input
          required
          type="password"
          autocomplete="new-password"
          bind:value={secret}
        /></label
      >
      <label
        >{m.credential_weight()}<input
          required
          type="number"
          min="0"
          bind:value={weight}
        /></label
      >
      <button class="button primary" type="submit" disabled={submitting}
        >{m.save()}</button
      >
    </form>
  {/if}
  {#if credentials.length === 0}
    <p class="empty-row">{m.empty_credentials()}</p>
  {:else}
    <div class="data-list">
      {#each credentials as credential (credential.id)}
        <div class="data-row credential-row">
          <span class="key-mark" aria-hidden="true"></span>
          <div>
            <strong>{credential.label}</strong><small
              >•••••••• · {m.credential_weight()} {credential.weight}</small
            >
          </div>
          <button
            class="danger-link"
            type="button"
            disabled={submitting}
            onclick={() => onremove(credential.id)}>{m.delete()}</button
          >
        </div>
      {/each}
    </div>
  {/if}
</section>
