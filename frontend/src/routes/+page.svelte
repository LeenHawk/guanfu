<script lang="ts">
  import type { AssetHeadDto } from "$lib/bindings/AssetHeadDto";
  import type { ChannelDto } from "$lib/bindings/ChannelDto";
  import type { ChatBootstrap } from "$lib/bindings/ChatBootstrap";
  import type { ChatHistoryView } from "$lib/bindings/ChatHistoryView";
  import { api } from "$lib/api/channels";
  import { chatApi, runChat } from "$lib/api/chat";
  import { ApiClientError } from "$lib/api/error";
  import AppHeader from "$lib/components/AppHeader.svelte";
  import ChatComposer from "$lib/components/ChatComposer.svelte";
  import ChatMessages from "$lib/components/ChatMessages.svelte";
  import ChatSidebar from "$lib/components/ChatSidebar.svelte";
  import { messageForError } from "$lib/i18n/errors";
  import { deltaText, messageText } from "$lib/ui/messages";
  import { m } from "$lib/paraglide/messages.js";

  let bootstrap = $state<ChatBootstrap | null>(null);
  let histories = $state<AssetHeadDto[]>([]);
  let characters = $state<AssetHeadDto[]>([]);
  let channels = $state<ChannelDto[]>([]);
  let selected = $state<ChatHistoryView | null>(null);
  let selectedId = $state<number | null>(null);
  let channelId = $state<number | null>(null);
  let model = $state("gpt-5.6-sol");
  let characterId = $state<number | null>(null);
  let streaming = $state("");
  let running = $state(false);
  let submitting = $state(false);
  let errorCode = $state<string | null>(null);
  let controller: AbortController | null = null;

  $effect(() => {
    void load();
  });

  function captureError(error: unknown) {
    errorCode =
      error instanceof ApiClientError
        ? error.payload.code
        : "upstream_unavailable";
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

  async function load() {
    await run(async () => {
      [bootstrap, histories, characters, channels] = await Promise.all([
        chatApi.bootstrap(),
        chatApi.listHistories(),
        chatApi.listAssets("character"),
        api.listChannels(),
      ]);
      channelId ??= channels.find((channel) => channel.enabled)?.id ?? null;
      if (selectedId === null && histories.length > 0)
        await select(histories[0].id);
    });
  }

  async function select(id: number) {
    selectedId = id;
    selected = await chatApi.loadHistory(id);
    characterId = selected.bindings.character ?? characterId;
  }

  function createChat() {
    void run(async () => {
      const title = `${m.new_chat()} ${histories.length + 1}`;
      const head = await chatApi.createHistory(title, {
        character: characterId,
        persona: null,
        preset: bootstrap?.preset_asset_id ?? null,
        pipeline: bootstrap?.pipeline_asset_id ?? null,
        channel_id: channelId,
        model,
      });
      histories = [...histories, head];
      await select(head.id);
    });
  }

  function importCharacter(file: File) {
    void run(async () => {
      const imported = await chatApi.importCharacter(file);
      characters = await chatApi.listAssets("character");
      characterId = imported.character.id;
    });
  }

  /** 发一轮:流式增量只在本地累积,提交由后端在当轮结束时完成。 */
  async function send(text: string | null) {
    if (!bootstrap || !selected || !channelId) return;
    running = true;
    streaming = "";
    errorCode = null;
    controller = new AbortController();
    const bindings = [
      { slot: "history", asset_ids: [selected.head.id] },
      { slot: "preset", asset_ids: [bootstrap.preset_asset_id] },
      ...(characterId ? [{ slot: "character", asset_ids: [characterId] }] : []),
    ];
    try {
      await runChat(
        {
          pipeline_asset_id: bootstrap.pipeline_asset_id,
          bindings,
          channel_id: channelId,
          model,
          user_message: text,
          trigger: text ? "normal" : "regenerate",
        },
        (event) => {
          if (event.type === "progress") streaming += deltaText(event);
          else if (event.type === "failed") errorCode = event.error.code;
        },
        controller.signal,
      );
      await select(selected.head.id);
    } catch (error) {
      if (!(error instanceof DOMException && error.name === "AbortError"))
        captureError(error);
    } finally {
      running = false;
      streaming = "";
      controller = null;
    }
  }

  /** 重试:去掉上一轮 assistant 回复后按同样输入再跑一次。 */
  function retry() {
    const messages = selected?.messages ?? [];
    const last = [...messages].reverse().find((item) => item.role === "user");
    void send(last ? messageText(last) : null);
  }
</script>

<svelte:head><title>{m.app_title()} · {m.nav_chat()}</title></svelte:head>

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
  <div class="management-layout">
    <ChatSidebar
      {histories}
      {characters}
      {selectedId}
      {submitting}
      onselect={(id) => void run(() => select(id))}
      oncreate={createChat}
      onimport={importCharacter}
    />
    {#if selected}
      <main class="workspace chat-workspace">
        <header class="workspace-header">
          <div>
            <h2>{selected.title}</h2>
            {#if running}<p>{m.thinking()}</p>{/if}
          </div>
        </header>
        <ChatMessages messages={selected.messages} {streaming} />
        <ChatComposer
          {channels}
          {characters}
          bind:channelId
          bind:model
          bind:characterId
          {running}
          onsend={(text) => void send(text)}
          onstop={() => controller?.abort()}
          onretry={retry}
        />
      </main>
    {:else}
      <main class="center-state empty-state">
        <span class="seal" aria-hidden="true">复</span>
        <h2>{m.empty_chats()}</h2>
        <p>{m.empty_chats_hint()}</p>
        <button class="button primary" type="button" onclick={createChat}
          >{m.new_chat()}</button
        >
      </main>
    {/if}
  </div>
</div>
