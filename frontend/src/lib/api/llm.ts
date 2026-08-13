import { Channel, invoke } from "@tauri-apps/api/core";

import type { OperationResponse } from "$lib/bindings/OperationResponse";
import type { SemanticLlmRequest } from "$lib/bindings/SemanticLlmRequest";
import type { SemanticStreamMessage } from "$lib/bindings/SemanticStreamMessage";
import { ApiClientError, toApiClientError } from "$lib/api/error";
import { isTauri } from "$lib/api/transport";

export async function executeLlm(
  input: SemanticLlmRequest,
  onEvent: (event: SemanticStreamMessage) => void,
  signal?: AbortSignal,
): Promise<OperationResponse | null> {
  if (isTauri()) return executeTauri(input, onEvent, signal);
  const response = await fetch("/api/llm", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    signal,
  });
  if (!response.ok) throw new ApiClientError(await response.json());
  if (!response.headers.get("content-type")?.includes("text/event-stream")) {
    return (await response.json()) as OperationResponse;
  }
  await readEventStream(response, onEvent);
  return null;
}

async function executeTauri(
  input: SemanticLlmRequest,
  onEvent: (event: SemanticStreamMessage) => void,
  signal?: AbortSignal,
): Promise<OperationResponse | null> {
  const requestId = crypto.randomUUID();
  const channel = new Channel<SemanticStreamMessage>(onEvent);
  const cancel = () => void invoke("cancel_llm", { requestId });
  signal?.addEventListener("abort", cancel, { once: true });
  try {
    return await invoke<OperationResponse | null>("execute_llm", {
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
  onEvent: (event: SemanticStreamMessage) => void,
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
      if (data) onEvent(JSON.parse(data) as SemanticStreamMessage);
    }
    if (done) break;
  }
}
