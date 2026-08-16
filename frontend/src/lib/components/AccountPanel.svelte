<script lang="ts">
  import type { SessionSummary } from "$lib/bindings/SessionSummary";
  import { authApi } from "$lib/api/auth";
  import { m } from "$lib/paraglide/messages.js";

  let {
    onerror,
    onsignedout,
  }: {
    onerror: (error: unknown) => void;
    onsignedout: () => void;
  } = $props();

  let sessions = $state<SessionSummary[]>([]);
  let busy = $state(false);

  $effect(() => {
    void load();
  });

  async function load() {
    try {
      sessions = await authApi.listSessions();
    } catch (error) {
      onerror(error);
    }
  }

  async function act(task: () => Promise<unknown>) {
    busy = true;
    try {
      await task();
      await load();
    } catch (error) {
      onerror(error);
    } finally {
      busy = false;
    }
  }

  function when(ms: number): string {
    // 服务端给的是毫秒时间戳,按 locale 显示而不是拼字符串。
    return new Date(ms).toLocaleString();
  }

  let others = $derived(sessions.filter((session) => !session.current).length);
</script>

<section class="config-section" aria-labelledby="account-title">
  <header>
    <div>
      <h3 id="account-title">{m.sessions()}</h3>
      <span>{sessions.length}</span>
    </div>
    <div class="channel-actions">
      <button
        class="text-button"
        type="button"
        disabled={busy || others === 0}
        onclick={() => act(() => authApi.revokeOthers())}
        >{m.revoke_others()}</button
      >
      <button
        class="text-button"
        type="button"
        disabled={busy}
        onclick={() =>
          act(async () => {
            await authApi.logout();
            onsignedout();
          })}>{m.sign_out()}</button
      >
    </div>
  </header>
  {#if others === 0 && sessions.length <= 1}
    <p class="empty-row">{m.no_other_sessions()}</p>
  {/if}
  <div class="data-list">
    {#each sessions as session (session.id)}
      <div class="data-row session-row">
        <div>
          <strong
            >{session.current
              ? m.session_current()
              : session.id.slice(0, 12)}</strong
          >
          <small
            >{m.session_created()}
            {when(session.created_at_ms)} ·
            {m.session_expires()}
            {when(session.expires_at_ms)}</small
          >
        </div>
        {#if !session.current}
          <button
            class="danger-link"
            type="button"
            disabled={busy}
            onclick={() => act(() => authApi.revokeSession(session.id))}
            >{m.revoke()}</button
          >
        {/if}
      </div>
    {/each}
  </div>
</section>
