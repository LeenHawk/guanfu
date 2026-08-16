import { m } from "$lib/paraglide/messages.js";

/** 后端稳定 error code → 本地化文案;未知 code 落到上游不可用。 */
export function messageForError(code: string): string {
  const messages: Record<string, () => string> = {
    database: m.error_database,
    upstream_unavailable: m.error_upstream_unavailable,
    invalid_data: m.error_invalid_data,
    invalid_route: m.error_invalid_route,
    channel_not_found: m.error_channel_not_found,
    no_usable_credential: m.error_no_usable_credential,
    unsupported_route: m.error_unsupported_route,
    upstream_rejected: m.error_upstream_rejected,
  };
  return messages[code]?.() ?? m.error_upstream_unavailable();
}
