<script lang="ts">
  import type { RoutingKind } from "$lib/bindings/RoutingKind";
  import type { RoutingOperation } from "$lib/bindings/RoutingOperation";
  import type { RoutingRuleDto } from "$lib/bindings/RoutingRuleDto";
  import {
    kindLabel,
    kindsFor,
    operationLabel,
    operations,
  } from "$lib/i18n/routing";
  import { m } from "$lib/paraglide/messages.js";

  let {
    rules,
    submitting,
    onput,
    onremove,
  }: {
    rules: RoutingRuleDto[];
    submitting: boolean;
    onput: (
      operation: RoutingOperation,
      kind: RoutingKind,
      action: string,
      targetOperation: RoutingOperation,
      targetKind: RoutingKind,
    ) => Promise<void>;
    onremove: (id: number) => Promise<void>;
  } = $props();

  let adding = $state(false);
  let operation = $state<RoutingOperation>("generate_content");
  let kind = $state<RoutingKind>("open_ai_responses");
  let action = $state("passthrough");
  // 目标操作默认跟随源操作(常见情形同操作换格式),可手动改成跨操作。
  let targetOperation = $derived<RoutingOperation>(operation);
  let targetKind = $state<RoutingKind>("claude_messages");
  let kinds = $derived(kindsFor(operation));
  let targetKinds = $derived(kindsFor(targetOperation));

  $effect(() => {
    if (!kinds.includes(kind)) kind = kinds[0];
  });

  $effect(() => {
    if (!targetKinds.includes(targetKind)) targetKind = targetKinds[0];
  });

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    await onput(operation, kind, action, targetOperation, targetKind);
    adding = false;
  }

  function actionLabel(type: string): string {
    if (type === "transform_to") return m.transform_to();
    if (type === "local") return m.local();
    if (type === "unsupported") return m.unsupported();
    return m.passthrough();
  }

  function targetSummary(rule: RoutingRuleDto): string {
    if (rule.implementation.type !== "transform_to") return "";
    const target = rule.implementation.target;
    return target.operation === rule.source.operation
      ? kindLabel(target.kind)
      : `${operationLabel(target.operation)} · ${kindLabel(target.kind)}`;
  }
</script>

<section class="config-section" aria-labelledby="routes-title">
  <header>
    <div>
      <h3 id="routes-title">{m.routing_rules()}</h3>
      <span>{rules.length}</span>
    </div>
    <button class="text-button" type="button" onclick={() => (adding = !adding)}
      >{m.add_route()}</button
    >
  </header>
  {#if adding}
    <form class="inline-form route-form" onsubmit={submit}>
      <label
        >{m.source_operation()}<select bind:value={operation}
          >{#each operations as value (value)}<option {value}
              >{operationLabel(value)}</option
            >{/each}</select
        ></label
      >
      <label
        >{m.source_kind()}<select bind:value={kind}
          >{#each kinds as value (value)}<option {value}
              >{kindLabel(value)}</option
            >{/each}</select
        ></label
      >
      <label
        >{m.route_action()}<select bind:value={action}
          ><option value="passthrough">{m.passthrough()}</option><option
            value="transform_to">{m.transform_to()}</option
          ><option value="local">{m.local()}</option><option value="unsupported"
            >{m.unsupported()}</option
          ></select
        ></label
      >
      {#if action === "transform_to"}
        <label
          >{m.target_operation()}<select bind:value={targetOperation}
            >{#each operations as value (value)}<option {value}
                >{operationLabel(value)}</option
              >{/each}</select
          ></label
        >
        <label
          >{m.target_kind()}<select bind:value={targetKind}
            >{#each targetKinds as value (value)}<option {value}
                >{kindLabel(value)}</option
              >{/each}</select
          ></label
        >
      {/if}
      <button class="button primary" type="submit" disabled={submitting}
        >{m.save()}</button
      >
    </form>
  {/if}
  {#if rules.length === 0}
    <p class="empty-row">{m.empty_routes()}</p>
  {:else}
    <div class="data-list">
      {#each rules as rule (rule.id)}
        <div class="data-row route-row">
          <div>
            <strong>{operationLabel(rule.source.operation)}</strong><small
              >{kindLabel(rule.source.kind)}</small
            >
          </div>
          <span class="route-line" aria-hidden="true"></span>
          <div>
            <strong>{actionLabel(rule.implementation.type)}</strong><small
              >{targetSummary(rule)}</small
            >
          </div>
          <button
            class="danger-link"
            type="button"
            disabled={submitting}
            onclick={() =>
              confirm(m.confirm_delete_item()) && onremove(rule.id)}
            >{m.delete()}</button
          >
        </div>
      {/each}
    </div>
  {/if}
</section>
