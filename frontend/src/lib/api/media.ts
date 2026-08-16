import { Channel, invoke } from "@tauri-apps/api/core";

import type { AssetHeadDto } from "$lib/bindings/AssetHeadDto";
import type { CreateVideoRequest } from "$lib/bindings/CreateVideoRequest";
import type { GenerateImageRequest } from "$lib/bindings/GenerateImageRequest";
import type { RealtimeClientEvent } from "$lib/bindings/RealtimeClientEvent";
import type { RealtimeDownstream } from "$lib/bindings/RealtimeDownstream";
import type { SpeechRequest } from "$lib/bindings/SpeechRequest";
import type { Transcription } from "$lib/bindings/Transcription";
import type { TranscriptionRequest } from "$lib/bindings/TranscriptionRequest";
import type { VideoJob } from "$lib/bindings/VideoJob";
import type { MediaResult } from "$lib/bindings/MediaResult";
import { toApiClientError } from "$lib/api/error";
import {
  authToken,
  invokeCommand,
  isTauri,
  requestJson,
} from "$lib/api/transport";

type Input<T> = { channel_id: number; name: string; request: T };

export const mediaApi = {
  generateImage: (input: Input<GenerateImageRequest>): Promise<MediaResult> =>
    isTauri()
      ? invokeCommand("generate_image", { input })
      : requestJson("/api/media/images", {
          method: "POST",
          body: JSON.stringify(input),
        }),

  speech: (input: Input<SpeechRequest>): Promise<AssetHeadDto> =>
    isTauri()
      ? invokeCommand("create_speech", { input })
      : requestJson("/api/media/speech", {
          method: "POST",
          body: JSON.stringify(input),
        }),

  transcribe: (input: Input<TranscriptionRequest>): Promise<Transcription> =>
    isTauri()
      ? invokeCommand("transcribe", { input })
      : requestJson("/api/media/transcriptions", {
          method: "POST",
          body: JSON.stringify(input),
        }),

  createVideo: (input: Input<CreateVideoRequest>): Promise<VideoJob> =>
    isTauri()
      ? invokeCommand("create_video", { input })
      : requestJson("/api/media/videos", {
          method: "POST",
          body: JSON.stringify(input),
        }),

  pollVideo: (channelId: number, id: string): Promise<VideoJob> => {
    const input = { channel_id: channelId, id, name: "" };
    return isTauri()
      ? invokeCommand("poll_video", { input })
      : requestJson("/api/media/videos/poll", {
          method: "POST",
          body: JSON.stringify(input),
        });
  },

  downloadVideo: (
    channelId: number,
    contentRef: string,
    name: string,
  ): Promise<AssetHeadDto> => {
    const input = { channel_id: channelId, id: contentRef, name };
    return isTauri()
      ? invokeCommand("download_video", { input })
      : requestJson("/api/media/videos/download", {
          method: "POST",
          body: JSON.stringify(input),
        });
  },
};

/** 媒体内容的可引用地址;桌面端没有 HTTP 端点,改用 data URL。 */
export async function mediaSrc(id: number): Promise<string> {
  if (!isTauri()) return `/api/media/${id}/content`;
  return invokeCommand<string>("media_data_url", { id });
}

/** 一个 realtime 会话;两壳同一套下行帧。 */
export interface RealtimeSession {
  send: (event: RealtimeClientEvent) => void;
  close: () => void;
}

export async function connectRealtime(
  channelId: number,
  request: unknown,
  onFrame: (frame: RealtimeDownstream) => void,
): Promise<RealtimeSession> {
  if (isTauri()) return connectRealtimeTauri(channelId, request, onFrame);

  const url = new URL("/api/realtime", window.location.href);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  const socket = new WebSocket(url);
  await new Promise<void>((resolve, reject) => {
    socket.addEventListener("open", () => resolve(), { once: true });
    socket.addEventListener("error", () => reject(new Error("ws error")), {
      once: true,
    });
  });
  // 首帧带渠道与会话配置,服务端据此连上游。
  socket.send(
    JSON.stringify({ token: authToken(), channel_id: channelId, request }),
  );
  socket.addEventListener("message", (event) =>
    onFrame(JSON.parse(event.data) as RealtimeDownstream),
  );
  return {
    send: (event) => socket.send(JSON.stringify(event)),
    close: () => socket.close(),
  };
}

async function connectRealtimeTauri(
  channelId: number,
  request: unknown,
  onFrame: (frame: RealtimeDownstream) => void,
): Promise<RealtimeSession> {
  const sessionId = crypto.randomUUID();
  const channel = new Channel<RealtimeDownstream>(onFrame);
  // 命令在会话结束前不返回,所以不等待它。
  void invoke("connect_realtime", {
    sessionId,
    channelId,
    input: request,
    onEvent: channel,
  }).catch((error) =>
    onFrame({ type: "error", error: toApiClientError(error).payload }),
  );
  return {
    send: (event) => void invoke("send_realtime", { sessionId, event }),
    close: () => void invoke("close_realtime", { sessionId }),
  };
}
