import { Channel, invoke } from "@tauri-apps/api/core";

import type { ChatEvent } from "$lib/bindings/ChatEvent";
import type { CompleteReply } from "$lib/bindings/CompleteReply";
import type { LlmRequestDto } from "$lib/bindings/LlmRequestDto";
import { ApiClientError, toApiClientError } from "$lib/api/error";
import { isTauri } from "$lib/api/transport";

export async function executeLlm(
  input: LlmRequestDto,
  onEvent: (event: ChatEvent) => void,
  signal?: AbortSignal,
): Promise<CompleteReply | null> {
  if (isTauri()) return executeTauri(input, onEvent, signal);
  const response = await fetch("/api/llm", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    signal,
  });
  if (!response.ok) throw new ApiClientError(await response.json());
  if (!response.headers.get("content-type")?.includes("text/event-stream")) {
    return (await response.json()) as CompleteReply;
  }
  await readEventStream(response, onEvent);
  return null;
}

async function executeTauri(
  input: LlmRequestDto,
  onEvent: (event: ChatEvent) => void,
  signal?: AbortSignal,
): Promise<CompleteReply | null> {
  const requestId = crypto.randomUUID();
  const channel = new Channel<ChatEvent>(onEvent);
  const cancel = () => void invoke("cancel_llm", { requestId });
  signal?.addEventListener("abort", cancel, { once: true });
  try {
    return await invoke<CompleteReply | null>("execute_llm", {
      requestId,
      input,
      onEvent: channel,
    });
  } catch (error) {
    throw toApiClientError(error);
  } finally {
    signal?.removeEventListener("abort", cancel);
  }
}

async function readEventStream(
  response: Response,
  onEvent: (event: ChatEvent) => void,
): Promise<void> {
  const reader = response.body
    ?.pipeThrough(new TextDecoderStream())
    .getReader();
  if (!reader) return;
  let buffer = "";
  for (;;) {
    const { done, value } = await reader.read();
    buffer += value ?? "";
    const frames = buffer.split("\n\n");
    buffer = frames.pop() ?? "";
    for (const frame of frames) {
      const data = frame
        .split("\n")
        .filter((line) => line.startsWith("data:"))
        .map((line) => line.slice(5).trimStart())
        .join("\n");
      if (data) onEvent(JSON.parse(data) as ChatEvent);
    }
    if (done) break;
  }
}
