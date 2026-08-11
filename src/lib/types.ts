export type Health =
  | "healthy"
  | "degraded"
  | "stopped"
  | "incompatible"
  | "permission_denied";

export interface MonitorSnapshot {
  health: Health;
  detail: string;
  socket_path: string;
  checked_at_ms: number;
  status?: Record<string, unknown>;
  capabilities?: Record<string, unknown>;
  configuration_supported: boolean;
}

export interface LanDiscoveryCounters {
  services_resolved?: number;
  candidate_addresses?: number;
  handshakes_started?: number;
  handshake_start_failures?: number;
  skipped_own_advert?: number;
  skipped_missing_npub?: number;
  skipped_scope_mismatch?: number;
  skipped_unusable_address?: number;
  skipped_no_compatible_udp?: number;
  skipped_invalid_npub?: number;
  skipped_duplicate_peer?: number;
  skipped_connected_or_connecting?: number;
}

export interface LanDiscoveryStatus {
  enabled?: boolean;
  state?: "disabled" | "running" | "failed" | "unavailable";
  service_type?: string;
  scope?: string | null;
  advertised_port?: number | null;
  udp_bindings?: Array<{ name?: string; bind_addr?: string; outbound_only?: boolean }>;
  loopback_only?: boolean;
  warnings?: string[];
  counters?: LanDiscoveryCounters;
}

export interface Peer {
  npub?: string;
  display_name?: string;
  node_addr?: string;
  ipv6_addr?: string;
  connectivity?: string;
  transport_type?: string;
  transport_addr?: string;
  direction?: string;
  is_parent?: boolean;
  is_child?: boolean;
  tree_depth?: number;
  stats?: Record<string, unknown>;
  mmp?: Record<string, unknown>;
}

export interface Transport {
  transport_id?: number;
  type?: string;
  state?: string;
  name?: string;
  local_addr?: string;
  mtu?: number;
  onion_address?: string;
  stats?: Record<string, unknown>;
}

export interface ConfigSnapshot {
  source: "operator" | "managed";
  base_path: string;
  managed_path: string;
  revision: string;
  yaml: string;
  guided?: Record<string, unknown>;
  secrets?: Record<string, unknown>;
  last_apply?: ApplyStatus;
}

export interface ConfigDiff {
  path?: string;
  before?: unknown;
  after?: unknown;
}

export interface ValidationResult {
  valid: boolean;
  errors: Array<{ path: string; message: string }>;
  yaml?: string;
  diff: ConfigDiff[];
  warnings: string[];
  activation: "none" | "hot_peers" | "restart" | null;
}

export interface ApplyResult {
  apply_id: string;
  revision: string;
  activation: "none" | "hot_peers" | "restart";
  diff: ConfigDiff[];
}

export interface ApplyStatus {
  apply_id?: string;
  state?: "pending" | "applied" | "rolled_back" | "failed";
  error?: string;
  updated_at_ms?: number;
}

export interface InvokeError {
  kind?: string;
  message?: string;
}
