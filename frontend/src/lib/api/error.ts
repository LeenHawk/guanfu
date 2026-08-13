import type { ApiError } from "$lib/bindings/ApiError";

export class ApiClientError extends Error {
  constructor(public readonly payload: ApiError) {
    super(payload.code);
  }
}

export function toApiClientError(error: unknown): ApiClientError {
  if (typeof error === "object" && error !== null && "code" in error) {
    return new ApiClientError(error as ApiError);
  }
  return new ApiClientError({ code: "upstream_unavailable", details: null });
}
