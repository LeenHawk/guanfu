<script lang="ts">
  import type { ChannelDto } from "$lib/bindings/ChannelDto";
  import type { CredentialDto } from "$lib/bindings/CredentialDto";
  import type { RoutingKind } from "$lib/bindings/RoutingKind";
  import type { RoutingOperation } from "$lib/bindings/RoutingOperation";
  import type { RoutingRuleDto } from "$lib/bindings/RoutingRuleDto";
  import { m } from "$lib/paraglide/messages.js";
  import CredentialSection from "$lib/components/CredentialSection.svelte";
  import InvocationSection from "$lib/components/InvocationSection.svelte";
  import RoutingSection from "$lib/components/RoutingSection.svelte";

  let {
    channel,
    credentials,
    rules,
    submitting,
    ontoggle,
    ondelete,
    onaddcredential,
    onremovecredential,
    onputroute,
    onremoveroute,
  }: {
    channel: ChannelDto;
    credentials: CredentialDto[];
    rules: RoutingRuleDto[];
    submitting: boolean;
    ontoggle: () => Promise<void>;
    ondelete: () => Promise<void>;
    onaddcredential: (
      label: string,
      secret: string,
      weight: number,
    ) => Promise<void>;
    onremovecredential: (id: number) => Promise<void>;
    onputroute: (
      operation: RoutingOperation,
      kind: RoutingKind,
      action: string,
      targetOperation: RoutingOperation,
      targetKind: RoutingKind,
    ) => Promise<void>;
    onremoveroute: (id: number) => Promise<void>;
  } = $props();
</script>

<main class="workspace">
  <header class="workspace-header">
    <div>
      <h2>{channel.name}</h2>
      <p>{channel.base_url}</p>
    </div>
    <div class="channel-actions">
      <button
        class="status-toggle"
        class:on={channel.enabled}
        type="button"
        onclick={ontoggle}
        disabled={submitting}
      >
        <span></span>{channel.enabled ? m.enabled() : m.disabled()}
      </button>
      <button
        class="danger-link"
        type="button"
        onclick={() => confirm(m.confirm_delete_channel()) && ondelete()}
        >{m.delete()}</button
      >
    </div>
  </header>
  <CredentialSection
    {credentials}
    {submitting}
    onadd={onaddcredential}
    onremove={onremovecredential}
  />
  <RoutingSection
    {rules}
    {submitting}
    onput={onputroute}
    onremove={onremoveroute}
  />
  <InvocationSection {channel} />
</main>
