import { invoke } from "@tauri-apps/api/core";

import type { ApiError } from "$lib/bindings/ApiError";
import { ApiClientError, toApiClientError } from "$lib/api/error";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

const TOKEN_KEY = "guanfu-token";

/** 共享令牌;桌面壳是本地单用户进程,不需要它。 */
export function authToken(): string | null {
  if (isTauri() || typeof localStorage === "undefined") return null;
  return localStorage.getItem(TOKEN_KEY);
}

export function setAuthToken(token: string | null): void {
  if (token) localStorage.setItem(TOKEN_KEY, token);
  else localStorage.removeItem(TOKEN_KEY);
}

/** 带上令牌的请求头。 */
export function authHeaders(): Record<string, string> {
  const token = authToken();
  return token ? { authorization: `Bearer ${token}` } : {};
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
    headers: {
      "content-type": "application/json",
      ...authHeaders(),
      ...init?.headers,
    },
  });
  if (response.status === 401) {
    // 令牌无效就丢掉,让界面重新索要,而不是反复撞 401。
    setAuthToken(null);
    throw new ApiClientError({ code: "unauthorized", details: null });
  }
  if (!response.ok) {
    const payload = (await response.json()) as ApiError;
    throw new ApiClientError(payload);
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}
