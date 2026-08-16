<script lang="ts">
  import { ApiClientError } from "$lib/api/error";
  import { authApi } from "$lib/api/auth";
  import { isTauri } from "$lib/api/transport";
  import AccountPanel from "$lib/components/AccountPanel.svelte";
  import AppHeader from "$lib/components/AppHeader.svelte";
  import AuthGate from "$lib/components/AuthGate.svelte";
  import { messageForError } from "$lib/i18n/errors";
  import { m } from "$lib/paraglide/messages.js";

  let errorCode = $state<string | null>(null);
  let needsSetup = $state(false);
  let reloadKey = $state(0);

  function captureError(error: unknown) {
    errorCode =
      error instanceof ApiClientError
        ? error.payload.code
        : "upstream_unavailable";
    if (errorCode === "unauthorized") {
      void authApi
        .status()
        .then((status) => (needsSetup = status.needs_setup))
        .catch(() => (needsSetup = false));
    }
  }
</script>

<svelte:head><title>{m.app_title()} · {m.account()}</title></svelte:head>

<div class="app-shell">
  <AppHeader />
  {#if errorCode && errorCode !== "unauthorized"}<div
      class="error-banner"
      role="alert"
    >
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
      <div><h2>{m.account()}</h2></div>
    </header>
    {#if isTauri()}
      <!-- 桌面壳是本地单用户进程,没有会话可管理。 -->
      <p class="empty-row">{m.account_local_only()}</p>
    {:else}
      {#key reloadKey}
        <AccountPanel
          onerror={captureError}
          onsignedout={() => (errorCode = "unauthorized")}
        />
      {/key}
    {/if}
  </main>
</div>

<AuthGate
  open={errorCode === "unauthorized"}
  {needsSetup}
  onsignedin={() => {
    errorCode = null;
    reloadKey += 1;
  }}
/>
