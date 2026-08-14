export type Health =
  | "healthy"
  | "degraded"
  | "stopped"
  | "incompatible"
  | "permission_denied";

export interface ServiceStatus {
  available: boolean;
  state: "running" | "stopped" | "unknown";
  enabled: boolean;
  loaded: boolean;
  running: boolean;
  controller_version?: number;
  pid?: number;
  last_exit_status?: number;
  detail?: string;
  ownership: "app_managed" | "external" | "none" | "conflict" | "unknown";
  installation: "standard" | "app_managed" | "external" | "not_installed" | "conflict" | "checking";
  can_migrate: boolean;
  config_path?: string;
  registration: "enabled" | "requires_approval" | "not_registered" | "bundle_incomplete" | "unsupported";
}

export interface AppPreferences {
  show_dock_icon: boolean;
  open_dashboard_at_launch: boolean;
}

export interface PreviewScenario {
  id: string;
  label: string;
}

export interface ProductPreviewStatus {
  available: boolean;
  enabled: boolean;
  scenario: string;
  scenarios: PreviewScenario[];
}

export interface MonitorSnapshot {
  preview: boolean;
  health: Health;
  detail: string;
  socket_path: string;
  checked_at_ms: number;
  status?: Record<string, unknown>;
  capabilities?: Record<string, unknown>;
  configuration_supported: boolean;
  service: ServiceStatus;
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

export interface MmpLayerMetrics {
  loss_rate?: number;
  smoothed_loss?: number;
  srtt_ms?: number;
  etx?: number;
  smoothed_etx?: number;
}

export interface MmpPeerMeasurement {
  peer: string;
  display_name?: string;
  mode?: "full" | "lightweight" | "minimal" | string;
  link_layer?: MmpLayerMetrics;
}

export interface MmpSessionMeasurement {
  remote: string;
  display_name?: string;
  mode?: "full" | "lightweight" | "minimal" | string;
  session_layer?: MmpLayerMetrics;
}

export interface MmpSnapshot {
  peers: MmpPeerMeasurement[];
  sessions: MmpSessionMeasurement[];
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
  activation: "none" | "restart" | null;
}

export interface ApplyResult {
  apply_id: string;
  revision: string;
  activation: "none" | "restart";
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
