<script lang="ts">
  import type { ChannelDto } from "$lib/bindings/ChannelDto";
  import type { RealtimeDownstream } from "$lib/bindings/RealtimeDownstream";
  import { connectRealtime, type RealtimeSession } from "$lib/api/media";
  import {
    floatToPcm16Base64,
    pcm16Base64ToFloat,
    PlaybackQueue,
    REALTIME_RATE,
    resample,
  } from "$lib/ui/realtime-audio";
  import { m } from "$lib/paraglide/messages.js";

  let {
    channels,
    channelId = $bindable(),
    onerror,
  }: {
    channels: ChannelDto[];
    channelId: number | null;
    onerror: (error: unknown) => void;
  } = $props();

  let model = $state("gpt-realtime");
  let voice = $state("alloy");
  let status = $state<"idle" | "connecting" | "ready">("idle");
  let transcript = $state("");
  let typed = $state("");

  let session: RealtimeSession | null = null;
  let context: AudioContext | null = null;
  let capture: MediaStream | null = null;
  let node: ScriptProcessorNode | null = null;
  let playback: PlaybackQueue | null = null;

  function onFrame(frame: RealtimeDownstream) {
    if (frame.type === "ready") {
      status = "ready";
      return;
    }
    if (frame.type === "error") {
      onerror(frame.error);
      return;
    }
    const event = frame.event;
    switch (event.type) {
      case "audio_delta":
        playback?.enqueue(pcm16Base64ToFloat(event.delta));
        break;
      case "output_transcript_delta":
        transcript += event.delta;
        break;
      case "input_transcript_completed":
        transcript += `\n> ${event.transcript}\n`;
        break;
      case "input_speech_started":
        // 用户插话:丢掉尚未播放的助手音频,别继续讲。
        playback?.reset();
        break;
      default:
        break;
    }
  }

  async function connect() {
    if (!channelId) return;
    status = "connecting";
    transcript = "";
    try {
      capture = await navigator.mediaDevices.getUserMedia({ audio: true });
      context = new AudioContext();
      playback = new PlaybackQueue(context);
      session = await connectRealtime(
        channelId,
        {
          session: {
            model,
            instructions: [],
            // 上游只接受 ["text"] 或 ["audio"],不接受两者并存;
            // 语音通话取 audio,文字由输入/输出转写单独给出。
            modalities: ["audio"],
            voice,
            speed: null,
            input_audio_format: { type: "pcm16", rate: REALTIME_RATE },
            output_audio_format: { type: "pcm16", rate: REALTIME_RATE },
            input_transcription: { model: null, language: null, prompt: null },
            noise_reduction: null,
            // 交给服务端 VAD 断句,浏览器只管持续送流。
            turn_detection: { type: "server_vad" },
            tools: [],
            tool_choice: { type: "auto" },
            max_output_tokens: null,
          },
        },
        onFrame,
      );

      const source = context.createMediaStreamSource(capture);
      node = context.createScriptProcessor(4096, 1, 1);
      node.onaudioprocess = (event) => {
        const input = event.inputBuffer.getChannelData(0);
        const resampled = resample(
          new Float32Array(input),
          context!.sampleRate,
          REALTIME_RATE,
        );
        session?.send({
          type: "append_audio",
          audio: floatToPcm16Base64(resampled),
        });
      };
      source.connect(node);
      // 处理节点必须接到目的地才会被调度;静音增益避免回放自己的声音。
      const silence = context.createGain();
      silence.gain.value = 0;
      node.connect(silence);
      silence.connect(context.destination);
    } catch (error) {
      status = "idle";
      onerror(error);
      hangup();
    }
  }

  /// 打字发言:合成麦克风或嘈杂环境下仍能推进一轮对话。
  function sendTyped(event: SubmitEvent) {
    event.preventDefault();
    const text = typed.trim();
    if (!text || !session) return;
    typed = "";
    transcript += `\n> ${text}\n`;
    session.send({
      type: "create_item",
      item: {
        type: "message",
        message: { role: "user", content: [{ type: "text", text }] },
      },
    });
    session.send({ type: "create_response" });
  }

  function hangup() {
    session?.close();
    session = null;
    node?.disconnect();
    node = null;
    capture?.getTracks().forEach((track) => track.stop());
    capture = null;
    void context?.close();
    context = null;
    playback = null;
    status = "idle";
  }
</script>

<div class="inline-form studio-form">
  <label
    >{m.chat_channel()}<select
      bind:value={channelId}
      disabled={status !== "idle"}
    >
      {#each channels as channel (channel.id)}
        <option value={channel.id}>{channel.name}</option>
      {/each}
    </select></label
  >
  <label
    >{m.chat_model()}<input
      bind:value={model}
      disabled={status !== "idle"}
    /></label
  >
  <label
    >{m.voice()}<input bind:value={voice} disabled={status !== "idle"} /></label
  >
  {#if status === "idle"}
    <button
      class="button primary"
      type="button"
      onclick={connect}
      disabled={!channelId}>{m.realtime_connect()}</button
    >
  {:else}
    <button class="button secondary" type="button" onclick={hangup}
      >{m.realtime_hangup()}</button
    >
  {/if}
</div>

{#if status === "ready"}
  <form class="composer-row" onsubmit={sendTyped}>
    <input
      bind:value={typed}
      placeholder={m.realtime_say()}
      aria-label={m.realtime_say()}
    />
    <button class="button primary" type="submit">{m.send()}</button>
  </form>
{/if}

<p class="realtime-status" aria-live="polite">
  {status === "ready"
    ? m.realtime_listening()
    : status === "connecting"
      ? m.loading()
      : m.realtime_idle()}
</p>

{#if transcript}
  <pre class="invocation-result">{transcript}</pre>
{/if}
