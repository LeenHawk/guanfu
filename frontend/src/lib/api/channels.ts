import type { ChannelDto } from "$lib/bindings/ChannelDto";
import type { CredentialDto } from "$lib/bindings/CredentialDto";
import type { NewChannel } from "$lib/bindings/NewChannel";
import type { NewCredential } from "$lib/bindings/NewCredential";
import type { PutRoutingRule } from "$lib/bindings/PutRoutingRule";
import type { RoutingRuleDto } from "$lib/bindings/RoutingRuleDto";
import { invokeCommand, isTauri, requestJson } from "$lib/api/transport";

const json = (value: unknown): RequestInit => ({
  method: "POST",
  body: JSON.stringify(value),
});

export const api = {
  listChannels: () =>
    isTauri()
      ? invokeCommand<ChannelDto[]>("list_channels")
      : requestJson<ChannelDto[]>("/api/channels"),

  createChannel: (input: NewChannel) =>
    isTauri()
      ? invokeCommand<ChannelDto>("create_channel", { input })
      : requestJson<ChannelDto>("/api/channels", json(input)),

  setChannelEnabled: (id: number, enabled: boolean) =>
    isTauri()
      ? invokeCommand<void>("set_channel_enabled", { id, enabled })
      : requestJson<void>(`/api/channels/${id}/enabled`, {
          method: "PUT",
          body: JSON.stringify({ enabled }),
        }),

  deleteChannel: (id: number) =>
    isTauri()
      ? invokeCommand<void>("delete_channel", { id })
      : requestJson<void>(`/api/channels/${id}`, { method: "DELETE" }),

  listCredentials: (channelId: number) =>
    isTauri()
      ? invokeCommand<CredentialDto[]>("list_credentials", { channelId })
      : requestJson<CredentialDto[]>(`/api/channels/${channelId}/credentials`),

  addCredential: (input: NewCredential) =>
    isTauri()
      ? invokeCommand<CredentialDto>("add_credential", { input })
      : requestJson<CredentialDto>(
          `/api/channels/${input.channel_id}/credentials`,
          json(input),
        ),

  removeCredential: (id: number) =>
    isTauri()
      ? invokeCommand<void>("remove_credential", { id })
      : requestJson<void>(`/api/credentials/${id}`, { method: "DELETE" }),

  listRoutingRules: (channelId: number) =>
    isTauri()
      ? invokeCommand<RoutingRuleDto[]>("list_routing_rules", { channelId })
      : requestJson<RoutingRuleDto[]>(
          `/api/channels/${channelId}/routing-rules`,
        ),

  putRoutingRule: (input: PutRoutingRule) =>
    isTauri()
      ? invokeCommand<RoutingRuleDto>("put_routing_rule", { input })
      : requestJson<RoutingRuleDto>(
          `/api/channels/${input.channel_id}/routing-rules`,
          { method: "PUT", body: JSON.stringify(input) },
        ),

  removeRoutingRule: (id: number) =>
    isTauri()
      ? invokeCommand<void>("remove_routing_rule", { id })
      : requestJson<void>(`/api/routing-rules/${id}`, { method: "DELETE" }),
};
