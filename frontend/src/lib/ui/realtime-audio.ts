/**
 * Realtime 语音的 PCM16 编解码与播放队列。
 *
 * 会话用 24 kHz 单声道 PCM16:麦克风采样率不一定是 24k,所以采集侧做一次
 * 线性重采样;播放侧把连续到达的块排进同一条时间线,避免逐块 start 造成
 * 的爆音与间隙。
 */
export const REALTIME_RATE = 24000;

export function floatToPcm16Base64(samples: Float32Array): string {
  const buffer = new ArrayBuffer(samples.length * 2);
  const view = new DataView(buffer);
  for (let i = 0; i < samples.length; i += 1) {
    const clamped = Math.max(-1, Math.min(1, samples[i]));
    view.setInt16(i * 2, clamped * 0x7fff, true);
  }
  return bytesToBase64(new Uint8Array(buffer));
}

export function pcm16Base64ToFloat(encoded: string): Float32Array {
  const bytes = base64ToBytes(encoded);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const samples = new Float32Array(bytes.byteLength / 2);
  for (let i = 0; i < samples.length; i += 1) {
    samples[i] = view.getInt16(i * 2, true) / 0x8000;
  }
  return samples;
}

/** 线性重采样到会话采样率。 */
export function resample(
  samples: Float32Array,
  from: number,
  to: number,
): Float32Array {
  if (from === to) return samples;
  const ratio = from / to;
  const length = Math.floor(samples.length / ratio);
  const output = new Float32Array(length);
  for (let i = 0; i < length; i += 1) {
    const position = i * ratio;
    const index = Math.floor(position);
    const next = Math.min(index + 1, samples.length - 1);
    const fraction = position - index;
    output[i] = samples[index] * (1 - fraction) + samples[next] * fraction;
  }
  return output;
}

/** 把陆续到达的音频块接在同一条播放时间线上。 */
export class PlaybackQueue {
  private context: AudioContext;
  private nextStart = 0;

  constructor(context: AudioContext) {
    this.context = context;
  }

  enqueue(samples: Float32Array): void {
    if (samples.length === 0) return;
    const buffer = this.context.createBuffer(1, samples.length, REALTIME_RATE);
    buffer.getChannelData(0).set(samples);
    const source = this.context.createBufferSource();
    source.buffer = buffer;
    source.connect(this.context.destination);
    const start = Math.max(this.context.currentTime, this.nextStart);
    source.start(start);
    this.nextStart = start + buffer.duration;
  }

  /** 打断:丢弃尚未播放的排期。 */
  reset(): void {
    this.nextStart = this.context.currentTime;
  }
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  // 分块避免超长参数列表爆栈。
  const step = 0x8000;
  for (let i = 0; i < bytes.length; i += step) {
    binary += String.fromCharCode(...bytes.subarray(i, i + step));
  }
  return btoa(binary);
}

function base64ToBytes(encoded: string): Uint8Array {
  const binary = atob(encoded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}
