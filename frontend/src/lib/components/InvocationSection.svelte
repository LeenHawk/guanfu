<script lang="ts">
  import type { ChatEvent } from "$lib/bindings/ChatEvent";
  import type { ChannelDto } from "$lib/bindings/ChannelDto";
  import { executeLlm } from "$lib/api/llm";
  import { m } from "$lib/paraglide/messages.js";

  let { channel }: { channel: ChannelDto } = $props();
  let model = $state("gpt-4.1-mini");
  let prompt = $state("");
  let result = $state("");
  let running = $state(false);
  let controller: AbortController | null = null;

  function append(event: ChatEvent) {
    if (event.type === "frame") {
      result += `${event.event}\n${JSON.stringify(event.data, null, 2)}\n\n`;
    }
    if (event.type === "error") result += `${event.error.code}\n`;
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    result = "";
    running = true;
    controller = new AbortController();
    try {
      const reply = await executeLlm(
        {
          channel_id: channel.id,
          operation: {
            operation: "stream_generate_content",
            kind: "open_ai_responses",
          },
          model,
          stream: true,
          body: { model, input: prompt, stream: true },
        },
        append,
        controller.signal,
      );
      if (reply) result = JSON.stringify(reply.body, null, 2);
    } catch (error) {
      if (!(error instanceof DOMException && error.name === "AbortError")) {
        result = error instanceof Error ? error.message : String(error);
      }
    } finally {
      running = false;
      controller = null;
    }
  }
</script>

<section class="config-section" aria-labelledby="invocation-title">
  <header>
    <div><h3 id="invocation-title">{m.connection_test()}</h3></div>
  </header>
  <form class="invocation-form" onsubmit={submit}>
    <label>{m.test_model()}<input required bind:value={model} /></label>
    <label
      >{m.test_prompt()}<textarea required rows="3" bind:value={prompt}
      ></textarea></label
    >
    <button class="button primary" type="submit" disabled={running}
      >{m.send()}</button
    >
    {#if running}<button
        class="button secondary"
        type="button"
        onclick={() => controller?.abort()}>{m.stop()}</button
      >{/if}
  </form>
  <h4>{m.test_result()}</h4>
  <pre class="invocation-result">{result || m.empty_result()}</pre>
</section>
