import type { AssetHeadDto } from "$lib/bindings/AssetHeadDto";
import type { Credentials } from "$lib/bindings/Credentials";
import type { SessionDto } from "$lib/bindings/SessionDto";
import type { UserDto } from "$lib/bindings/UserDto";
import { isTauri, requestJson, setAuthToken } from "$lib/api/transport";

/** 服务端是否还没有任何账号——决定显示"创建管理员"还是"登录"。 */
export interface AuthStatus {
  needs_setup: boolean;
}

export const authApi = {
  status: (): Promise<AuthStatus> =>
    isTauri()
      ? Promise.resolve({ needs_setup: false })
      : requestJson("/api/auth/status"),

  register: (
    credentials: Credentials,
    extra?: { bootstrap_token?: string; is_admin?: boolean },
  ): Promise<UserDto> =>
    requestJson("/api/auth/register", {
      method: "POST",
      body: JSON.stringify({ ...credentials, ...extra }),
    }),

  login: async (credentials: Credentials): Promise<SessionDto> => {
    const session = await requestJson<SessionDto>("/api/auth/login", {
      method: "POST",
      body: JSON.stringify(credentials),
    });
    setAuthToken(session.token);
    return session;
  },

  logout: (): void => setAuthToken(null),

  listUsers: (): Promise<UserDto[]> => requestJson("/api/users"),

  /** 共享 / 取消共享一个资产。 */
  setShared: (id: number, shared: boolean): Promise<AssetHeadDto> =>
    requestJson(`/api/assets/${id}/share`, {
      method: "PUT",
      body: JSON.stringify({ shared }),
    }),
};
