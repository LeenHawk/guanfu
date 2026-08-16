<script lang="ts">
  import type { ChatMessage } from "$lib/bindings/ChatMessage";
  import { renderMarkdown } from "$lib/ui/markdown";
  import { messageText } from "$lib/ui/messages";
  import { m } from "$lib/paraglide/messages.js";

  let { messages, streaming }: { messages: ChatMessage[]; streaming: string } =
    $props();

  /**
   * 只渲染最近若干条。
   *
   * 每条消息都要过一遍 markdown,几百轮的历史全量渲染会让切换对话明显卡顿;
   * 而且几乎没人一进来就要看最早那几条。往前翻按需展开。
   */
  const WINDOW = 60;
  let shown = $state(WINDOW);

  let all = $derived(messages.filter((message) => message.role !== "tool"));
  // 换一段历史就把窗口收回去,否则会带着上一段的展开量。
  $effect(() => {
    void messages;
    shown = WINDOW;
  });
  let hidden = $derived(Math.max(0, all.length - shown));
  let visible = $derived(hidden > 0 ? all.slice(hidden) : all);
</script>

<div class="chat-log" aria-live="polite">
  {#if hidden > 0}
    <button
      class="text-button load-earlier"
      type="button"
      onclick={() => (shown += WINDOW)}
      >{m.load_earlier({ count: hidden })}</button
    >
  {/if}
  {#if visible.length === 0 && !streaming}
    <p class="empty-row">{m.empty_messages()}</p>
  {/if}
  {#each visible as message, index (index)}
    <article class="chat-bubble {message.role}">
      <!-- eslint-disable-next-line svelte/no-at-html-tags -->
      {@html renderMarkdown(messageText(message))}
    </article>
  {/each}
  {#if streaming}
    <article class="chat-bubble assistant streaming">
      <!-- eslint-disable-next-line svelte/no-at-html-tags -->
      {@html renderMarkdown(streaming)}
    </article>
  {/if}
</div>
