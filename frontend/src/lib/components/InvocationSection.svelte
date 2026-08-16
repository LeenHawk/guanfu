<script lang="ts">
  import type { OperationResponse } from "$lib/bindings/OperationResponse";
  import type { SemanticStreamMessage } from "$lib/bindings/SemanticStreamMessage";
  import type { ChannelDto } from "$lib/bindings/ChannelDto";
  import { executeLlm } from "$lib/api/llm";
  import { messageForError } from "$lib/i18n/errors";
  import { m } from "$lib/paraglide/messages.js";

  let { channel }: { channel: ChannelDto } = $props();
  let model = $state("gpt-4.1-mini");
  let prompt = $state("");
  let result = $state("");
  let running = $state(false);
  let controller: AbortController | null = null;

  function append(message: SemanticStreamMessage) {
    if (message.type === "error") {
      result += `${messageForError(message.error.code)}\n`;
      return;
    }
    const operationEvent = message.event;
    if (operationEvent.operation !== "generate") return;
    const event = operationEvent.event;
    if (event.type === "delta") {
      const delta = event.data;
      if (delta.type === "text" || delta.type === "refusal") {
        result += delta.data.delta;
      }
    }
  }

  function completeText(response: OperationResponse): string {
    if (response.operation !== "generate")
      return JSON.stringify(response, null, 2);
    return response.response.output
      .filter((item) => item.type === "message")
      .flatMap((item) => item.item.content)
      .filter((content) => content.type === "text")
      .map((content) => content.text)
      .join("");
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
          request: {
            operation: "generate",
            request: {
              model,
              input: [
                {
                  type: "message",
                  message: {
                    role: "user",
                    content: [{ type: "text", text: prompt }],
                  },
                },
              ],
              instructions: [],
              tools: [],
              tool_choice: { type: "auto" },
              output: { type: "text" },
              sampling: {
                temperature: null,
                top_p: null,
                top_k: null,
                seed: null,
                stop: [],
                frequency_penalty: null,
                presence_penalty: null,
              },
              reasoning: null,
              protocol_options: [],
              limits: { max_output_tokens: null, max_tool_calls: null },
              modalities: ["text"],
              mode: "stream",
            },
          },
        },
        append,
        controller.signal,
      );
      if (reply) result = completeText(reply);
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
