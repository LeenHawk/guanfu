import type { RoutingKind } from "$lib/bindings/RoutingKind";
import type { RoutingOperation } from "$lib/bindings/RoutingOperation";
import { m } from "$lib/paraglide/messages.js";

export const operations: RoutingOperation[] = [
  "generate_content",
  "stream_generate_content",
  "count_tokens",
  "list_models",
  "get_model",
  "create_image",
  "edit_image",
  "create_embedding",
  "web_search",
  "rerank",
  "create_speech",
  "create_transcription",
  "create_translation",
  "compact_content",
  "create_video",
  "retrieve_video",
  "list_videos",
  "delete_video",
  "download_video_content",
  "create_conversation",
  "create_realtime_call",
  "connect_realtime",
];

export const contentKinds: RoutingKind[] = [
  "open_ai_responses",
  "open_ai_chat_completions",
  "claude_messages",
  "gemini_generate_content",
];

export const providerKinds: RoutingKind[] = ["open_ai", "claude", "gemini"];

export function isContentGeneration(operation: RoutingOperation): boolean {
  return (
    operation === "generate_content" || operation === "stream_generate_content"
  );
}

export function kindsFor(operation: RoutingOperation): RoutingKind[] {
  return isContentGeneration(operation) ? contentKinds : providerKinds;
}

const operationLabels: Record<RoutingOperation, () => string> = {
  generate_content: m.op_generate_content,
  stream_generate_content: m.op_stream_generate_content,
  count_tokens: m.op_count_tokens,
  list_models: m.op_list_models,
  get_model: m.op_get_model,
  create_image: m.op_create_image,
  edit_image: m.op_edit_image,
  create_embedding: m.op_create_embedding,
  web_search: m.op_web_search,
  rerank: m.op_rerank,
  create_speech: m.op_create_speech,
  create_transcription: m.op_create_transcription,
  create_translation: m.op_create_translation,
  compact_content: m.op_compact_content,
  create_video: m.op_create_video,
  retrieve_video: m.op_retrieve_video,
  list_videos: m.op_list_videos,
  delete_video: m.op_delete_video,
  download_video_content: m.op_download_video_content,
  create_conversation: m.op_create_conversation,
  create_realtime_call: m.op_create_realtime_call,
  connect_realtime: m.op_connect_realtime,
};

const kindLabels: Record<RoutingKind, () => string> = {
  open_ai_responses: m.kind_open_ai_responses,
  open_ai_responses_websocket: m.kind_open_ai_responses_websocket,
  open_ai_chat_completions: m.kind_open_ai_chat_completions,
  claude_messages: m.kind_claude_messages,
  gemini_generate_content: m.kind_gemini_generate_content,
  open_ai: m.kind_open_ai,
  claude: m.kind_claude,
  gemini: m.kind_gemini,
};

export function operationLabel(operation: RoutingOperation): string {
  return operationLabels[operation]?.() ?? operation;
}

export function kindLabel(kind: RoutingKind): string {
  return kindLabels[kind]?.() ?? kind;
}
