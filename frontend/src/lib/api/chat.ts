import { Channel, invoke } from "@tauri-apps/api/core";

import type { AssetHeadDto } from "$lib/bindings/AssetHeadDto";
import type { AssetKind } from "$lib/bindings/AssetKind";
import type { ChatBootstrap } from "$lib/bindings/ChatBootstrap";
import type { ChatHistoryView } from "$lib/bindings/ChatHistoryView";
import type { ChatRunRequest } from "$lib/bindings/ChatRunRequest";
import type { ImportedCharacter } from "$lib/bindings/ImportedCharacter";
import type { PipelineEvent } from "$lib/bindings/PipelineEvent";
import type { SessionBindings } from "$lib/bindings/SessionBindings";
import { ApiClientError, toApiClientError } from "$lib/api/error";
import { invokeCommand, isTauri, requestJson } from "$lib/api/transport";

export const chatApi = {
  bootstrap: (): Promise<ChatBootstrap> =>
    isTauri()
      ? invokeCommand("bootstrap_chat")
      : requestJson("/api/chat/bootstrap", { method: "POST" }),

  listAssets: (kind?: AssetKind): Promise<AssetHeadDto[]> =>
    isTauri()
      ? invokeCommand("list_assets", { kind: kind ?? null })
      : requestJson(`/api/assets${kind ? `?kind=${kind}` : ""}`),

  deleteAsset: (id: number): Promise<void> =>
    isTauri()
      ? invokeCommand("delete_asset", { id })
      : requestJson(`/api/assets/${id}`, { method: "DELETE" }),

  listHistories: (): Promise<AssetHeadDto[]> =>
    isTauri()
      ? invokeCommand("list_assets", { kind: "chat_history" })
      : requestJson("/api/chat/histories"),

  createHistory: (
    title: string,
    bindings: SessionBindings,
  ): Promise<AssetHeadDto> =>
    isTauri()
      ? invokeCommand("create_chat_history", { title, bindings })
      : requestJson("/api/chat/histories", {
          method: "POST",
          body: JSON.stringify({ title, bindings }),
        }),

  forkHistory: (
    id: number,
    messageCount: number,
    title: string,
  ): Promise<AssetHeadDto> =>
    isTauri()
      ? invokeCommand("fork_chat_history", { id, messageCount, title })
      : requestJson(`/api/chat/histories/${id}/fork`, {
          method: "POST",
          body: JSON.stringify({ title, message_count: messageCount }),
        }),

  loadHistory: (id: number): Promise<ChatHistoryView> =>
    isTauri()
      ? invokeCommand("load_chat_history", { id })
      : requestJson(`/api/chat/histories/${id}`),

  importCharacter: async (file: File): Promise<ImportedCharacter> => {
    const bytes = new Uint8Array(await file.arrayBuffer());
    if (isTauri())
      return invokeCommand("import_character", { bytes: Array.from(bytes) });
    const response = await fetch("/api/characters/import", {
      method: "POST",
      headers: { "content-type": "application/octet-stream" },
      body: bytes,
    });
    if (!response.ok) throw new ApiClientError(await response.json());
    return (await response.json()) as ImportedCharacter;
  },
};

/** 发起一次聊天 run;事件流结束时当轮已提交或已记为失败。 */
export async function runChat(
  input: ChatRunRequest,
  onEvent: (event: PipelineEvent) => void,
  signal?: AbortSignal,
): Promise<void> {
  if (isTauri()) return runChatTauri(input, onEvent, signal);
  const response = await fetch("/api/chat/runs", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    signal,
  });
  if (!response.ok) throw new ApiClientError(await response.json());
  await readEventStream(response, onEvent);
}

async function runChatTauri(
  input: ChatRunRequest,
  onEvent: (event: PipelineEvent) => void,
  signal?: AbortSignal,
): Promise<void> {
  const requestId = crypto.randomUUID();
  const channel = new Channel<PipelineEvent>(onEvent);
  const cancel = () => void invoke("cancel_llm", { requestId });
  signal?.addEventListener("abort", cancel, { once: true });
  try {
    await invoke("run_chat", { requestId, input, onEvent: channel });
  } catch (error) {
    throw toApiClientError(error);
  } finally {
    signal?.removeEventListener("abort", cancel);
  }
}

async function readEventStream(
  response: Response,
  onEvent: (event: PipelineEvent) => void,
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
      if (data) onEvent(JSON.parse(data) as PipelineEvent);
    }
    if (done) break;
  }
}
