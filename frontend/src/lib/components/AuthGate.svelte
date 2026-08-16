<script lang="ts">
  import { authApi } from "$lib/api/auth";
  import { ApiClientError } from "$lib/api/error";
  import { messageForError } from "$lib/i18n/errors";
  import { m } from "$lib/paraglide/messages.js";

  let {
    open,
    needsSetup,
    onsignedin,
  }: {
    open: boolean;
    needsSetup: boolean;
    onsignedin: () => void;
  } = $props();

  let name = $state("");
  let password = $state("");
  let bootstrapToken = $state("");
  let busy = $state(false);
  let errorCode = $state<string | null>(null);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    errorCode = null;
    try {
      const credentials = { name: name.trim(), password };
      if (needsSetup) {
        // 首个账号即管理员;之后的账号由管理员创建。
        await authApi.register(
          credentials,
          bootstrapToken ? { bootstrap_token: bootstrapToken } : undefined,
        );
      }
      await authApi.login(credentials);
      password = "";
      bootstrapToken = "";
      onsignedin();
    } catch (error) {
      errorCode =
        error instanceof ApiClientError ? error.payload.code : "unauthorized";
    } finally {
      busy = false;
    }
  }
</script>

{#if open}
  <div class="modal-backdrop" role="presentation">
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="auth-title"
    >
      <h2 id="auth-title">
        {needsSetup ? m.auth_setup_title() : m.auth_login_title()}
      </h2>
      {#if needsSetup}<p class="auth-hint">{m.auth_setup_hint()}</p>{/if}
      <form onsubmit={submit}>
        <label
          >{m.auth_name()}<input
            bind:value={name}
            autocomplete="username"
            required
          /></label
        >
        <label
          >{m.auth_password()}<input
            type="password"
            bind:value={password}
            autocomplete={needsSetup ? "new-password" : "current-password"}
            minlength={needsSetup ? 8 : undefined}
            required
          /></label
        >
        {#if needsSetup}
          <label
            >{m.auth_bootstrap()}<input
              type="password"
              bind:value={bootstrapToken}
            /></label
          >
        {/if}
        {#if errorCode}
          <p class="auth-error" role="alert">{messageForError(errorCode)}</p>
        {/if}
        <footer>
          <button class="button primary" type="submit" disabled={busy}
            >{needsSetup ? m.auth_create() : m.auth_signin()}</button
          >
        </footer>
      </form>
    </div>
  </div>
{/if}
