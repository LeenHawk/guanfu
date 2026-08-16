import type { ChatMessage } from "$lib/bindings/ChatMessage";

/** 消息的可见文本;reasoning 与工具轮不进聊天气泡。 */
export function messageText(message: ChatMessage): string {
  switch (message.role) {
    case "user":
      return message.content
        .filter((part) => part.type === "text")
        .map((part) => (part.type === "text" ? part.text : ""))
        .join("\n");
    case "assistant":
      return message.output
        .filter((item) => item.type === "message")
        .flatMap((item) => (item.type === "message" ? item.item.content : []))
        .map((content) =>
          content.type === "text" || content.type === "refusal"
            ? content.text
            : "",
        )
        .join("");
    case "system":
      return message.text;
    default:
      return "";
  }
}

/** 流式进度里的可见增量。 */
export function deltaText(event: {
  type: string;
  [key: string]: unknown;
}): string {
  if (event.type !== "progress") return "";
  const operation = event.event as
    | { operation?: string; event?: { type?: string; data?: unknown } }
    | undefined;
  if (operation?.operation !== "generate") return "";
  const inner = operation.event;
  if (inner?.type !== "delta") return "";
  const delta = inner.data as
    { type?: string; data?: { delta?: string } } | undefined;
  if (delta?.type !== "text" && delta?.type !== "refusal") return "";
  return delta.data?.delta ?? "";
}
