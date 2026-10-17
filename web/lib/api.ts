"use client";

export interface Account {
  id: string;
  name: string;
  status: "active" | "disabled" | "cooling";
  failure_count: number;
  last_failure_at?: string | null;
  created_at: string;
}

export interface AccountStatus {
  id: string;
  name: string;
  status: string;
  healthy: boolean;
  failure_count: number;
}

export interface RequestLog {
  ts: string;
  method: string;
  path: string;
  model: string;
  status: number;
  latency_ms: number;
  session_id?: string;
  account_id?: string;
  ttft_ms?: number;
  prompt_tokens?: number;
  completion_tokens?: number;
  total_tokens?: number;
  credit?: number;
  cached_tokens?: number;
  source_ip?: string;
  key_name?: string;
}

export interface GatewayInfo {
  version: string;
  listen: string;
  circuit_threshold: number;
  circuit_cooldown_secs: number;
  session_ttl_secs: number;
  session_max_size: number;
  failure_threshold: number;
  retry_max: number;
  retry_backoff_ms: number;
  account_recover_after_secs: number;
  default_model: string;
  sticky_sessions: number;
  api_keys: number;
  admin_keys: number;
  user_keys: number;
  /** A3: 上游连接建立超时（秒） */
  upstream_connect_timeout_secs: number;
  /** A3: 流式首字节超时（秒），首帧后不受限 */
  stream_first_byte_timeout_secs: number;
  /** A3: 非流式整体超时（秒） */
  non_stream_timeout_secs: number;
  upstream: string;
}

export interface ModelEntry {
  id: string;
  object: string;
  owned_by: string;
}

export interface ApiKeyInfo {
  name: string;
  hint: string;
  created_at?: string | null;
  expires_at?: string | null;
  token_quota: number;
  tokens_used: number;
  model_allowlist: string[];
  enabled: boolean;
  expired: boolean;
  /** A2: admin 或 user；旧数据缺省读作 admin */
  role: "admin" | "user";
  /** A2: 主密钥不可删改 */
  is_master: boolean;
  /** A2: 仍为旧 SHA-256 摘要时提示轮换 */
  legacy_hash: boolean;
}

export interface StatsResponse {
  totals: {
    requests: number;
    errors: number;
    success_rate: number;
    total_tokens: number;
    avg_latency_ms: number;
    avg_ttft_ms: number;
  };
  by_day: Array<{
    date: string;
    requests: number;
    errors: number;
    total_tokens: number;
    avg_latency_ms: number;
  }>;
  by_model: Array<{ model: string; requests: number; total_tokens: number }>;
  by_key: Array<{ key_name: string; requests: number; total_tokens: number }>;
}

export interface RouteRule {
  alias?: string;
  pattern?: string;
  target: string;
  effort?: string;
  note?: string;
}

export interface RouterConfig {
  rules: RouteRule[];
  downgrade?: {
    sources: string[];
    target: string;
    enabled: boolean;
  } | null;
}

const BASE = "/admin/api";

export class ApiError extends Error {
  status: number;
  constructor(message: string, status: number) {
    super(message);
    this.status = status;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    credentials: "same-origin",
    headers: {
      "Content-Type": "application/json",
      ...(init?.headers ?? {}),
    },
    ...init,
  });
  if (res.status === 401) {
    if (typeof window !== "undefined" && !window.location.pathname.endsWith("/login")) {
      const base = window.location.pathname.startsWith("/admin") ? "/admin" : "";
      window.location.href = `${base}/login`;
    }
    throw new ApiError("unauthorized", 401);
  }
  if (!res.ok) {
    let message = `request failed (${res.status})`;
    try {
      const data = await res.json();
      message = data?.error?.message ?? data?.error ?? message;
    } catch {
      // keep default message
    }
    throw new ApiError(String(message), res.status);
  }
  if (res.status === 204) {
    return undefined as T;
  }
  return res.json() as Promise<T>;
}

export const api = {
  listAccounts: () => request<Account[]>("/accounts"),
  createAccount: (name: string, credential: string) =>
    request<Account>("/accounts", {
      method: "POST",
      body: JSON.stringify({ name, credential }),
    }),
  patchAccount: (id: string, status: "active" | "disabled") =>
    request<Account>(`/accounts/${encodeURIComponent(id)}`, {
      method: "PATCH",
      body: JSON.stringify({ status }),
    }),
  deleteAccount: (id: string) =>
    request<void>(`/accounts/${encodeURIComponent(id)}`, { method: "DELETE" }),
  status: () => request<AccountStatus[]>("/status"),
  logs: () => request<RequestLog[]>("/logs"),
  stats: () => request<StatsResponse>("/stats"),
  info: () => request<GatewayInfo>("/info"),
  listKeys: () => request<ApiKeyInfo[]>("/keys"),
  createKey: (payload: {
    name: string;
    key: string;
    expires_at?: string | null;
    token_quota?: number;
    model_allowlist?: string[];
    /** A2: 缺省 user，需显式传 admin 才创建管理密钥 */
    role?: "admin" | "user";
  }) =>
    request<ApiKeyInfo>("/keys", {
      method: "POST",
      body: JSON.stringify(payload),
    }),
  patchKey: (
    name: string,
    payload: {
      enabled?: boolean;
      token_quota?: number;
      expires_at?: string | null;
      role?: "admin" | "user";
    }
  ) =>
    request<ApiKeyInfo>(`/keys/${encodeURIComponent(name)}`, {
      method: "PATCH",
      body: JSON.stringify(payload),
    }),
  deleteKey: (name: string) =>
    request<void>(`/keys/${encodeURIComponent(name)}`, { method: "DELETE" }),
  getRouter: () => request<RouterConfig>("/router"),
  putRouter: (cfg: RouterConfig) =>
    request<RouterConfig>("/router", {
      method: "PUT",
      body: JSON.stringify(cfg),
    }),
  oauthStart: () =>
    request<{ auth_url: string; state: string }>("/oauth/start", { method: "POST" }),
};

/// Data-plane call: goes through the gateway key stored in the gw_key cookie.
export async function listModels(): Promise<ModelEntry[]> {
  const key = getCookie("gw_key");
  const res = await fetch("/v1/models", {
    headers: key ? { Authorization: `Bearer ${key}` } : {},
  });
  if (!res.ok) {
    throw new ApiError(`models request failed (${res.status})`, res.status);
  }
  const data = (await res.json()) as { data: ModelEntry[] };
  return data.data;
}

export function getCookie(name: string): string {
  if (typeof document === "undefined") return "";
  const match = document.cookie
    .split(";")
    .map((c) => c.trim())
    .find((c) => c.startsWith(`${name}=`));
  return match ? decodeURIComponent(match.slice(name.length + 1)) : "";
}

export function setCookie(name: string, value: string, days = 7) {
  if (typeof document === "undefined") return;
  const expires = new Date(Date.now() + days * 86400_000).toUTCString();
  document.cookie = `${name}=${encodeURIComponent(value)}; expires=${expires}; path=/; SameSite=Lax`;
}

export function deleteCookie(name: string) {
  if (typeof document === "undefined") return;
  document.cookie = `${name}=; expires=Thu, 01 Jan 1970 00:00:00 GMT; path=/`;
}
