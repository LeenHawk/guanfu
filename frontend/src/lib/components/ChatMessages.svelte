<script lang="ts">
  import type { ChatMessage } from "$lib/bindings/ChatMessage";
  import { renderMarkdown } from "$lib/ui/markdown";
  import { messageText } from "$lib/ui/messages";
  import { m } from "$lib/paraglide/messages.js";

  let { messages, streaming }: { messages: ChatMessage[]; streaming: string } =
    $props();

  let visible = $derived(messages.filter((message) => message.role !== "tool"));
</script>

<div class="chat-log" aria-live="polite">
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
