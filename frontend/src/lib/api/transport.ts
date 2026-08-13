import { invoke } from "@tauri-apps/api/core";

import type { ApiError } from "$lib/bindings/ApiError";
import { ApiClientError, toApiClientError } from "$lib/api/error";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function invokeCommand<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw toApiClientError(error);
  }
}

export async function requestJson<T>(
  path: string,
  init?: RequestInit,
): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: { "content-type": "application/json", ...init?.headers },
  });
  if (!response.ok) {
    const payload = (await response.json()) as ApiError;
    throw new ApiClientError(payload);
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}
