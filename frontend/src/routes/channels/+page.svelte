<script lang="ts">
  import type { ChannelDto } from "$lib/bindings/ChannelDto";
  import type { CredentialDto } from "$lib/bindings/CredentialDto";
  import type { RoutingKind } from "$lib/bindings/RoutingKind";
  import type { RoutingOperation } from "$lib/bindings/RoutingOperation";
  import type { RoutingRuleDto } from "$lib/bindings/RoutingRuleDto";
  import { api } from "$lib/api/channels";
  import { ApiClientError } from "$lib/api/error";
  import AppHeader from "$lib/components/AppHeader.svelte";
  import AuthGate from "$lib/components/AuthGate.svelte";
  import { authApi } from "$lib/api/auth";
  import ChannelDialog from "$lib/components/ChannelDialog.svelte";
  import ChannelSidebar from "$lib/components/ChannelSidebar.svelte";
  import ChannelWorkspace from "$lib/components/ChannelWorkspace.svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import { messageForError } from "$lib/i18n/errors";
  import { m } from "$lib/paraglide/messages.js";

  let channels = $state<ChannelDto[]>([]);
  let credentials = $state<CredentialDto[]>([]);
  let rules = $state<RoutingRuleDto[]>([]);
  let selectedId = $state<number | null>(null);
  let loading = $state(true);
  let switching = $state(false);
  let submitting = $state(false);
  let dialogOpen = $state(false);
  let errorCode = $state<string | null>(null);
  let needsSetup = $state(false);
  let confirming = $state<{
    message: string;
    action: () => Promise<void>;
  } | null>(null);
  let selected = $derived(
    channels.find((channel) => channel.id === selectedId) ?? null,
  );

  $effect(() => {
    void loadChannels();
  });

  function captureError(error: unknown) {
    errorCode =
      error instanceof ApiClientError
        ? error.payload.code
        : "upstream_unavailable";
    // 401 时问一下服务端有没有账号,决定弹"创建管理员"还是"登录"。
    if (errorCode === "unauthorized") {
      void authApi
        .status()
        .then((status) => (needsSetup = status.needs_setup))
        .catch(() => (needsSetup = false));
    }
  }

  async function run(task: () => Promise<void>) {
    submitting = true;
    errorCode = null;
    try {
      await task();
    } catch (error) {
      captureError(error);
    } finally {
      submitting = false;
    }
  }

  async function loadChannels() {
    loading = true;
    try {
      channels = await api.listChannels();
      if (selectedId === null && channels.length > 0)
        await selectChannel(channels[0].id);
    } catch (error) {
      captureError(error);
    } finally {
      loading = false;
    }
  }

  async function selectChannel(id: number) {
    selectedId = id;
    switching = true;
    try {
      [credentials, rules] = await Promise.all([
        api.listCredentials(id),
        api.listRoutingRules(id),
      ]);
    } finally {
      switching = false;
    }
  }

  function requestConfirm(message: string, action: () => Promise<void>) {
    confirming = { message, action };
  }

  async function runConfirmed() {
    const current = confirming;
    if (!current) return;
    await run(current.action);
    confirming = null;
  }

  async function createChannel(name: string, baseUrl: string) {
    await run(async () => {
      const channel = await api.createChannel({ name, base_url: baseUrl });
      channels = [...channels, channel];
      await selectChannel(channel.id);
      dialogOpen = false;
    });
  }
</script>

<svelte:head><title>{m.app_title()} · {m.app_subtitle()}</title></svelte:head>

<div class="app-shell">
  <AppHeader />
  {#if errorCode}<div class="error-banner" role="alert">
      <span>{messageForError(errorCode)}</span>
      <button
        class="banner-close"
        type="button"
        aria-label={m.dismiss()}
        onclick={() => (errorCode = null)}>×</button
      >
    </div>{/if}
  {#if loading}
    <main class="center-state"><p>{m.loading()}</p></main>
  {:else if channels.length === 0}
    <main class="center-state empty-state">
      <span class="seal" aria-hidden="true">复</span>
      <h2>{m.empty_channels()}</h2>
      <p>{m.empty_channels_hint()}</p>
      <button
        class="button primary"
        type="button"
        onclick={() => (dialogOpen = true)}>{m.new_channel()}</button
      >
    </main>
  {:else}
    <div class="management-layout">
      <ChannelSidebar
        {channels}
        {selectedId}
        onselect={(id) => void run(() => selectChannel(id))}
        oncreate={() => (dialogOpen = true)}
      />
      {#if selected}
        <ChannelWorkspace
          channel={selected}
          {credentials}
          {rules}
          {submitting}
          busy={switching}
          ontoggle={() =>
            run(async () => {
              await api.setChannelEnabled(selected!.id, !selected!.enabled);
              channels = channels.map((item) =>
                item.id === selected!.id
                  ? { ...item, enabled: !item.enabled }
                  : item,
              );
            })}
          ondelete={() =>
            requestConfirm(m.confirm_delete_channel(), async () => {
              await api.deleteChannel(selected!.id);
              channels = channels.filter((item) => item.id !== selected!.id);
              selectedId = channels[0]?.id ?? null;
              if (selectedId) await selectChannel(selectedId);
            })}
          onaddcredential={(label, secret, weight) =>
            run(async () => {
              const item = await api.addCredential({
                channel_id: selected!.id,
                label,
                secret,
                weight,
              });
              credentials = [...credentials, item];
            })}
          onremovecredential={(id) =>
            requestConfirm(m.confirm_delete_item(), async () => {
              await api.removeCredential(id);
              credentials = credentials.filter((item) => item.id !== id);
            })}
          onputroute={(
            operation: RoutingOperation,
            kind: RoutingKind,
            action: string,
            targetOperation: RoutingOperation,
            targetKind: RoutingKind,
          ) =>
            run(async () => {
              const implementation =
                action === "transform_to"
                  ? {
                      type: "transform_to" as const,
                      target: { operation: targetOperation, kind: targetKind },
                    }
                  : { type: action as "passthrough" | "local" | "unsupported" };
              const item = await api.putRoutingRule({
                channel_id: selected!.id,
                source: { operation, kind },
                implementation,
                sort_order: rules.length,
                enabled: true,
              });
              rules = [
                ...rules.filter(
                  (rule) =>
                    rule.id !== item.id &&
                    !(
                      rule.source.operation === operation &&
                      rule.source.kind === kind
                    ),
                ),
                item,
              ];
            })}
          onremoveroute={(id) =>
            requestConfirm(m.confirm_delete_item(), async () => {
              await api.removeRoutingRule(id);
              rules = rules.filter((item) => item.id !== id);
            })}
        />
      {:else}
        <main class="center-state"><p>{m.select_channel()}</p></main>
      {/if}
    </div>
  {/if}
</div>

<AuthGate
  open={errorCode === "unauthorized"}
  {needsSetup}
  onsignedin={() => {
    errorCode = null;
    void loadChannels();
  }}
/>

<ChannelDialog
  open={dialogOpen}
  {submitting}
  onclose={() => (dialogOpen = false)}
  onsubmit={createChannel}
/>

<ConfirmDialog
  open={confirming !== null}
  message={confirming?.message ?? ""}
  {submitting}
  onconfirm={() => void runConfirmed()}
  oncancel={() => (confirming = null)}
/>
