<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import {
    formatDiffValue,
    lanDiscoveryIssue,
    newGuidedPeer,
    readGuidedDraft,
    writeGuidedDraft,
    type GuidedDraft,
  } from "$lib/configDraft";
  import type {
    AclSnapshot,
    ApplyResult,
    ApplyStatus,
    ConfigSnapshot,
    InvokeError,
    LanDiscoveryStatus,
    MonitorSnapshot,
    Peer,
    Transport,
    ValidationResult,
  } from "$lib/types";
  import { disconnectConfirmation } from "$lib/uiPolicy";
  import { formatFipsVersion } from "$lib/format";
  import Icon from "$lib/Icon.svelte";

  type View = "overview" | "peers" | "transports" | "access" | "settings";
  type AclList = "allow" | "deny";
  type AclRemoval = { list: AclList; entry: string };
  type SettingsSection = "identity" | "network" | "discovery" | "transports" | "peers";

  const initialSnapshot: MonitorSnapshot = {
    health: "stopped",
    detail: "Looking for the FIPS daemon…",
    socket_path: "/var/run/fips/control.sock",
    checked_at_ms: Date.now(),
    configuration_supported: false,
  };

  let snapshot = $state<MonitorSnapshot>(initialSnapshot);
  let activeView = $state<View>("overview");
  let peers = $state<Peer[]>([]);
  let transports = $state<Transport[]>([]);
  let acl = $state<AclSnapshot | null>(null);
  let aclLoading = $state(false);
  let aclError = $state("");
  let aclModal = $state<AclList | null>(null);
  let aclRemoval = $state<AclRemoval | null>(null);
  let aclEntry = $state("");
  let aclBusy = $state(false);
  let detailLoading = $state(false);
  let detailError = $state("");
  let selectedPeer = $state<Peer | null>(null);
  let connectOpen = $state(false);
  let connectNpub = $state("");
  let connectAddress = $state("");
  let connectTransport = $state("udp");
  let actionBusy = $state(false);
  let toast = $state("");

  let config = $state<ConfigSnapshot | null>(null);
  let draftYaml = $state("");
  let guided = $state<GuidedDraft | null>(null);
  let settingsMode = $state<"guided" | "yaml">("guided");
  let settingsSection = $state<SettingsSection>("identity");
  let configLoading = $state(false);
  let configError = $state("");
  let draftError = $state("");
  let validation = $state<ValidationResult | null>(null);
  let applyBusy = $state(false);
  let applyMessage = $state("");
  let developmentPath = $state("/var/run/fips/control.sock");

  const status = $derived((snapshot.status ?? {}) as Record<string, unknown>);
  const lanDiscovery = $derived(
    status.lan_discovery && typeof status.lan_discovery === "object"
      ? (status.lan_discovery as LanDiscoveryStatus)
      : null,
  );
  const guidedLanIssue = $derived(guided ? lanDiscoveryIssue(guided) : null);
  const online = $derived(snapshot.health === "healthy" || snapshot.health === "degraded");
  const allowEntries = $derived(acl?.allow_file_entries ?? acl?.allow_entries ?? []);
  const denyEntries = $derived(acl?.deny_file_entries ?? acl?.deny_entries ?? []);
  const healthLabel = $derived(
    snapshot.health === "permission_denied"
      ? "Permission denied"
      : snapshot.health.charAt(0).toUpperCase() + snapshot.health.slice(1),
  );

  function isTauri(): boolean {
    return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  }

  function errorMessage(error: unknown): string {
    if (typeof error === "string") return error;
    const value = error as InvokeError;
    return value?.message ?? "The operation could not be completed.";
  }

  function text(value: unknown, fallback = "—"): string {
    return typeof value === "string" && value.length > 0 ? value : fallback;
  }

  function number(value: unknown, fallback = 0): number {
    return typeof value === "number" ? value : fallback;
  }

  function humanDuration(value: unknown): string {
    const total = number(value);
    if (total < 60) return `${total}s`;
    const days = Math.floor(total / 86400);
    const hours = Math.floor((total % 86400) / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    return [days && `${days}d`, hours && `${hours}h`, minutes && `${minutes}m`]
      .filter(Boolean)
      .join(" ");
  }

  function compact(value: unknown): string {
    const amount = number(value);
    return Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(amount);
  }

  function shortId(value: unknown): string {
    const id = text(value);
    return id.length > 24 ? `${id.slice(0, 12)}…${id.slice(-8)}` : id;
  }

  function sparkPoints(value: unknown): string {
    if (!Array.isArray(value) || value.length < 2) return "0,28 100,28";
    const samples = value.map((sample) => number(sample));
    const min = Math.min(...samples);
    const max = Math.max(...samples);
    const range = max - min || 1;
    return samples
      .map((sample, index) => {
        const x = (index / (samples.length - 1)) * 100;
        const y = 31 - ((sample - min) / range) * 26;
        return `${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(" ");
  }

  function sparklines(): Record<string, unknown> {
    const value = status.sparklines;
    return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
  }

  function lanStateLabel(): string {
    if (!lanDiscovery) return "Diagnostics unavailable";
    if (lanDiscovery.state === "running") return "Running";
    if (lanDiscovery.state === "disabled") return "Disabled";
    if (lanDiscovery.state === "failed") return "Failed";
    return lanDiscovery.enabled ? "Unavailable" : "Disabled";
  }

  function lanStateClass(): string {
    if (lanDiscovery?.state === "running" && !lanDiscovery.loopback_only) return "healthy";
    if (lanDiscovery?.state === "disabled") return "stopped";
    return "degraded";
  }

  function lanBindingLabel(): string {
    const bindings = lanDiscovery?.udp_bindings ?? [];
    if (!bindings.length) return "No UDP listener";
    return bindings
      .map((binding) => binding.bind_addr ?? binding.name ?? "UDP")
      .join(", ");
  }

  function lanSkipReasons(): Array<{ label: string; value: number }> {
    const counters = lanDiscovery?.counters ?? {};
    return [
      { label: "Own advertisement", value: counters.skipped_own_advert ?? 0 },
      { label: "Scope mismatch", value: counters.skipped_scope_mismatch ?? 0 },
      { label: "Missing npub", value: counters.skipped_missing_npub ?? 0 },
      { label: "Unusable address", value: counters.skipped_unusable_address ?? 0 },
      { label: "No compatible UDP", value: counters.skipped_no_compatible_udp ?? 0 },
      { label: "Invalid npub", value: counters.skipped_invalid_npub ?? 0 },
      { label: "Duplicate candidate", value: counters.skipped_duplicate_peer ?? 0 },
      { label: "Already connected", value: counters.skipped_connected_or_connecting ?? 0 },
    ].filter((reason) => reason.value > 0);
  }

  async function refreshOverview() {
    if (!isTauri()) return;
    try {
      snapshot = await invoke<MonitorSnapshot>("get_snapshot");
      developmentPath = snapshot.socket_path;
      await loadDetails();
      if (activeView === "access") await loadAcl(true);
      invoke("refresh_now").catch(() => {});
    } catch (error) {
      detailError = errorMessage(error);
    }
  }

  async function copyNpub() {
    if (!isTauri()) return;
    try {
      await invoke("copy_node_npub");
      toast = "Node npub copied.";
    } catch (error) {
      toast = errorMessage(error);
    }
  }

  async function loadDetails() {
    if (!isTauri() || !online || activeView === "settings") return;
    detailLoading = true;
    detailError = "";
    try {
      if (activeView === "overview" || activeView === "peers" || activeView === "access") {
        const result = await invoke<{ peers?: Peer[] }>("get_peers");
        peers = result.peers ?? [];
      }
      if (activeView === "overview" || activeView === "transports") {
        const result = await invoke<{ transports?: Transport[] }>("get_transports");
        transports = result.transports ?? [];
      }
    } catch (error) {
      detailError = errorMessage(error);
    } finally {
      detailLoading = false;
    }
  }

  async function loadAcl(force = false) {
    if (!isTauri() || aclLoading || (acl && !force)) return;
    aclLoading = true;
    aclError = "";
    try {
      acl = await invoke<AclSnapshot>("get_acl");
    } catch (error) {
      aclError = errorMessage(error);
    } finally {
      aclLoading = false;
    }
  }

  async function selectView(view: View) {
    activeView = view;
    selectedPeer = null;
    if (view === "settings") await loadConfig();
    else if (view === "access") {
      await loadAcl(true);
      await loadDetails();
    }
    else await loadDetails();
  }

  function accessModeLabel(): string {
    if (!acl) return "Unavailable";
    if (acl.deny_all && allowEntries.length) return "Strict allowlist";
    if (denyEntries.length) return "Denylist active";
    if (allowEntries.length) return "Explicit allows";
    return "Default allow";
  }

  function openAclModal(list: AclList) {
    aclModal = list;
    aclRemoval = null;
    aclEntry = "";
    aclError = "";
  }

  function requestRemoveAclEntry(list: AclList, entry: string) {
    aclRemoval = { list, entry };
    aclError = "";
  }

  async function refreshAclAfterChange(list: AclList, entry: string, shouldExist: boolean) {
    for (let attempt = 0; attempt < 6; attempt += 1) {
      await new Promise((resolve) => window.setTimeout(resolve, 350));
      await loadAcl(true);
      const entries = list === "allow" ? allowEntries : denyEntries;
      if (entries.some((candidate) => candidate.toLowerCase() === entry.toLowerCase()) === shouldExist) return;
    }
  }

  async function restoreDashboardFocus() {
    if (!isTauri()) return;
    await invoke("focus_dashboard").catch(() => {});
  }

  async function addAclEntry(list: AclList) {
    const entry = aclEntry.trim();
    if (!entry) return;
    aclBusy = true;
    aclError = "";
    try {
      const result = await invoke<{ changed?: boolean }>("add_acl_entry", { list, entry });
      aclModal = null;
      aclEntry = "";
      toast = result.changed === false
        ? `${entry} is already on the ${list}list.`
        : `${entry} added to the ${list}list.`;
      await refreshAclAfterChange(list, entry, true);
      await restoreDashboardFocus();
    } catch (error) {
      aclError = errorMessage(error);
      await restoreDashboardFocus();
    } finally {
      aclBusy = false;
    }
  }

  async function removeAclEntry(list: AclList, entry: string) {
    aclBusy = true;
    aclError = "";
    try {
      await invoke("remove_acl_entry", { list, entry });
      aclRemoval = null;
      toast = `${entry} removed from the ${list}list.`;
      await refreshAclAfterChange(list, entry, false);
      await restoreDashboardFocus();
    } catch (error) {
      aclError = errorMessage(error);
      await restoreDashboardFocus();
    } finally {
      aclBusy = false;
    }
  }

  async function connect() {
    if (!connectNpub.trim() || !connectAddress.trim()) return;
    actionBusy = true;
    try {
      await invoke("connect_peer", {
        npub: connectNpub.trim(),
        address: connectAddress.trim(),
        transport: connectTransport,
      });
      connectOpen = false;
      connectNpub = "";
      connectAddress = "";
      toast = "Connection requested.";
      await loadDetails();
    } catch (error) {
      toast = errorMessage(error);
    } finally {
      actionBusy = false;
    }
  }

  async function disconnect(peer: Peer) {
    const npub = peer.npub ?? "";
    const label = peer.display_name || shortId(npub);
    if (!npub || !window.confirm(disconnectConfirmation(label))) {
      return;
    }
    actionBusy = true;
    try {
      await invoke("disconnect_peer", { npub });
      selectedPeer = null;
      toast = "Peer disconnected.";
      await loadDetails();
    } catch (error) {
      toast = errorMessage(error);
    } finally {
      actionBusy = false;
    }
  }

  async function loadConfig(force = false) {
    if (!isTauri() || configLoading || (config && !force)) return;
    configLoading = true;
    configError = "";
    validation = null;
    try {
      config = await invoke<ConfigSnapshot>("get_config");
      draftYaml = config.yaml;
      guided = readGuidedDraft(draftYaml);
      developmentPath = snapshot.socket_path;
    } catch (error) {
      configError = errorMessage(error);
    } finally {
      configLoading = false;
    }
  }

  function syncGuided() {
    if (!guided) return;
    try {
      draftYaml = writeGuidedDraft(draftYaml, guided);
      draftError = "";
      validation = null;
    } catch (error) {
      draftError = `Advanced YAML must be valid before guided changes can be synchronized: ${errorMessage(error)}`;
    }
  }

  function repairLanDiscoveryTransport() {
    if (!guided || !guidedLanIssue) return;
    guided.udpEnabled = true;
    guided.udpBind = guidedLanIssue.suggestedBind;
    syncGuided();
    toast = `UDP will listen on ${guided.udpBind} after you review and apply the draft.`;
  }

  function syncYamlToGuided() {
    try {
      guided = readGuidedDraft(draftYaml);
      draftError = "";
      validation = null;
    } catch (error) {
      draftError = `Invalid YAML: ${errorMessage(error)}`;
    }
  }

  function addPeer() {
    if (!guided) return;
    guided.peers = [...guided.peers, newGuidedPeer()];
    syncGuided();
  }

  function removePeer(index: number) {
    if (!guided) return;
    guided.peers = guided.peers.filter((_, candidate) => candidate !== index);
    syncGuided();
  }

  async function reviewConfig() {
    if (!config || draftError || applyBusy) return;
    applyMessage = "";
    try {
      validation = await invoke<ValidationResult>("validate_config", {
        expectedRevision: config.revision,
        yaml: draftYaml,
      });
    } catch (error) {
      validation = null;
      configError = errorMessage(error);
    }
  }

  async function waitForApply(applyId: string): Promise<ApplyStatus | null> {
    const deadline = Date.now() + 45_000;
    while (Date.now() < deadline) {
      await new Promise((resolve) => window.setTimeout(resolve, 1_000));
      try {
        const status = await invoke<ApplyStatus>("get_apply_status");
        if (status?.apply_id === applyId && status.state && status.state !== "pending") return status;
      } catch {
        // A restart closes the control socket briefly. Keep polling until the daemon returns.
      }
    }
    return null;
  }

  async function applyDraft() {
    if (!config || !validation || applyBusy) return;
    applyBusy = true;
    configError = "";
    applyMessage = "Applying configuration…";
    try {
      const result = await invoke<ApplyResult>("apply_config", {
        expectedRevision: config.revision,
        yaml: draftYaml,
      });
      if (result.activation === "restart") {
        applyMessage = "FIPS is restarting with the new configuration…";
        const status = await waitForApply(result.apply_id);
        if (status?.state === "rolled_back" || status?.state === "failed") {
          applyMessage = `The new configuration could not start and was rolled back${status.error ? `: ${status.error}` : "."}`;
        } else if (status?.state === "applied") {
          applyMessage = "Configuration applied and FIPS restarted successfully.";
        } else {
          applyMessage = "The apply is still pending. FIPS Monitor will keep showing daemon health as it reconnects.";
        }
      } else {
        applyMessage = result.activation === "hot_peers"
          ? "Peer configuration applied without restarting FIPS."
          : "Configuration saved. No runtime restart was needed.";
      }
      config = null;
      validation = null;
      await loadConfig(true);
      invoke("refresh_now").catch(() => {});
    } catch (error) {
      configError = errorMessage(error);
      applyMessage = "";
    } finally {
      applyBusy = false;
    }
  }

  async function resetConfig() {
    if (!config || applyBusy) return;
    if (!window.confirm(`Stop using ${config.managed_path} and restore the untouched operator configuration? FIPS will restart.`)) return;
    applyBusy = true;
    try {
      const result = await invoke<ApplyResult>("reset_config", { expectedRevision: config.revision });
      applyMessage = "Restoring the operator configuration and restarting FIPS…";
      const status = await waitForApply(result.apply_id);
      applyMessage = status?.state === "applied"
        ? "The operator configuration is active again."
        : "Reset requested. Monitoring will resume when FIPS reconnects.";
      config = null;
      await loadConfig(true);
    } catch (error) {
      configError = errorMessage(error);
    } finally {
      applyBusy = false;
    }
  }

  async function changeSocketPath() {
    if (!isTauri()) return;
    try {
      developmentPath = await invoke<string>("set_socket_path", { socketPath: developmentPath });
      toast = "Development socket updated.";
      config = null;
      await refreshOverview();
    } catch (error) {
      toast = errorMessage(error);
    }
  }

  onMount(() => {
    if (!isTauri()) return;
    let snapshotUnlisten: UnlistenFn | undefined;
    let navigateUnlisten: UnlistenFn | undefined;
    void listen<MonitorSnapshot>("monitor://snapshot", (event) => {
      snapshot = event.payload;
      developmentPath = snapshot.socket_path;
      if (document.visibilityState === "visible" && activeView !== "settings" && online) {
        void loadDetails();
        if (activeView === "access") void loadAcl(true);
      }
    }).then((unlisten) => (snapshotUnlisten = unlisten));
    void listen<string>("app://navigate", (event) => {
      void selectView(event.payload === "settings" ? "settings" : event.payload === "access" ? "access" : "overview");
    }).then((unlisten) => (navigateUnlisten = unlisten));
    void refreshOverview();
    return () => {
      snapshotUnlisten?.();
      navigateUnlisten?.();
    };
  });
</script>

<svelte:head><title>FIPS Monitor</title></svelte:head>

<div class="app-shell">
  <aside class="sidebar">
    <div class="brand">
      <div class="brand-mark" aria-hidden="true"><Icon name="brand" size={34} strokeWidth={1.45} /></div>
      <div><strong>FIPS</strong><span>Monitor</span></div>
    </div>

    <nav aria-label="Main navigation">
      <button class:active={activeView === "overview"} onclick={() => selectView("overview")}>
        <span class="nav-icon"><Icon name="overview" /></span> Overview
      </button>
      <button class:active={activeView === "peers"} onclick={() => selectView("peers")}>
        <span class="nav-icon"><Icon name="peers" /></span> Peers <em>{peers.length || number(status.peer_count)}</em>
      </button>
      <button class:active={activeView === "transports"} onclick={() => selectView("transports")}>
        <span class="nav-icon"><Icon name="transports" /></span> Transports <em>{transports.length || number(status.transport_count)}</em>
      </button>
      <button class:active={activeView === "access"} onclick={() => selectView("access")}>
        <span class="nav-icon"><Icon name="access" /></span> Access control <em>{allowEntries.length + denyEntries.length || ""}</em>
      </button>
      <button class:active={activeView === "settings"} onclick={() => selectView("settings")}>
        <span class="nav-icon"><Icon name="settings" /></span> Settings
      </button>
    </nav>

    <div class="sidebar-node">
      <span class="status-dot {snapshot.health}"></span>
      <div><strong>{healthLabel}</strong><small>{online ? formatFipsVersion(status.version) : "Local node"}</small></div>
    </div>
  </aside>

  <main>
    <header class="topbar">
      <div>
        <p>LOCAL NODE</p>
        <h1>{activeView === "overview" ? "Network overview" : activeView === "peers" ? "Authenticated peers" : activeView === "transports" ? "Transport health" : activeView === "access" ? "Peer access control" : "Node configuration"}</h1>
      </div>
      <div class="header-actions">
        <span class="checked">Checked {new Date(snapshot.checked_at_ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })}</span>
        <button class="icon-button" aria-label="Refresh" title="Refresh" onclick={refreshOverview}><Icon name="refresh" size={17} /></button>
        {#if activeView === "peers"}<button class="primary small" onclick={() => (connectOpen = true)}>Connect peer</button>{/if}
        {#if activeView === "access"}<button type="button" class="primary small" onclick={() => openAclModal("allow")}>Add rule</button>{/if}
      </div>
    </header>

    <div class="content">
      {#if snapshot.health !== "healthy"}
        <section class="health-banner {snapshot.health}">
          <div class="health-symbol">{snapshot.health === "permission_denied" ? "⌕" : snapshot.health === "stopped" ? "○" : "!"}</div>
          <div><strong>{healthLabel}</strong><p>{snapshot.detail}</p></div>
          {#if snapshot.health === "stopped"}
            <button onclick={() => selectView("settings")}>Connection settings</button>
          {:else if snapshot.health === "permission_denied"}
            <span class="hint">Group changes require a fresh login session.</span>
          {/if}
        </section>
      {/if}

      {#if activeView === "overview"}
        <section class="hero-grid">
          <article class="node-card">
            <div class="card-heading"><span>NODE HEALTH</span><span class="pill {snapshot.health}">{healthLabel}</span></div>
            <div class="identity-row">
              <div class="node-glyph"><Icon name="node" size={44} strokeWidth={1.35} /></div>
              <div class="identity-copy">
                <h2>{text(status.ipv6_addr, "FIPS node")}</h2>
                <div class="identity-key">
                  <code>{text(status.npub)}</code>
                  {#if text(status.npub) !== "—"}
                    <button class="copy-button" aria-label="Copy node npub" title="Copy node npub" onclick={copyNpub}>
                      <Icon name="copy" size={14} />
                    </button>
                  {/if}
                </div>
              </div>
            </div>
            <div class="metrics four">
              <div><span>UPTIME</span><strong>{humanDuration(status.uptime_secs)}</strong></div>
              <div><span>MESH ESTIMATE</span><strong>{compact(status.estimated_mesh_size)}</strong></div>
              <div><span>ROLE</span><strong>{status.is_root ? "Root" : status.is_leaf_only ? "Leaf" : "Mesh"}</strong></div>
              <div><span>IDENTITY</span><strong>{status.persistent ? "Persistent" : "Ephemeral"}</strong></div>
            </div>
          </article>
          <article class="tun-card">
            <div class="card-heading"><span>TUN INTERFACE</span><span class="mini-dot {text(status.tun_state).toLowerCase()}"></span></div>
            <strong class="tun-name">{text(status.tun_name)}</strong>
            <p>{text(status.tun_state)} · IPv6 MTU {number(status.effective_ipv6_mtu, 1280)}</p>
            <div class="tun-route"><span>Mesh address</span><code title={text(status.ipv6_addr)}>{text(status.ipv6_addr)}</code></div>
          </article>
        </section>

        <section class="stat-grid">
          <article><div><span>AUTHENTICATED PEERS</span><strong>{number(status.peer_count)}</strong></div><svg viewBox="0 0 100 34" preserveAspectRatio="none"><polyline points={sparkPoints(sparklines().peer_count)} /></svg></article>
          <article><div><span>ACTIVE SESSIONS</span><strong>{number(status.session_count)}</strong></div><svg viewBox="0 0 100 34" preserveAspectRatio="none"><polyline points={sparkPoints(sparklines().tree_depth)} /></svg></article>
          <article><div><span>MESH NODES</span><strong>{compact(status.estimated_mesh_size)}</strong></div><svg viewBox="0 0 100 34" preserveAspectRatio="none"><polyline points={sparkPoints(sparklines().mesh_size)} /></svg></article>
          <article><div><span>LIVE TRANSPORTS</span><strong>{number(status.transport_count)}</strong></div><div class="transport-dots">{#each transports.slice(0, 5) as transport}<i title={transport.type ?? "transport"}></i>{/each}</div></article>
        </section>

        {#if lanDiscovery}
          <button class="lan-summary" onclick={() => selectView("transports")}>
            <span class="mini-dot {lanDiscovery.state === 'running' && !lanDiscovery.loopback_only ? 'running' : ''}"></span>
            <span><strong>LAN discovery</strong><small>{lanDiscovery.loopback_only ? "UDP is only reachable from this Mac" : `${lanStateLabel()} · ${lanBindingLabel()}`}</small></span>
            <span><b>{number(lanDiscovery.counters?.candidate_addresses)}</b><small>peer candidates</small></span>
            <span><b>{number(lanDiscovery.counters?.handshakes_started)}</b><small>handshakes</small></span>
            <i>View diagnostics →</i>
          </button>
        {/if}

        <section class="dashboard-grid">
          <article class="panel traffic-panel">
            <div class="panel-title"><div><span>TRAFFIC & QUALITY</span><h3>Last 30 seconds</h3></div><span class="legend"><i></i> In <i></i> Out</span></div>
            <svg class="traffic-chart" viewBox="0 0 100 42" preserveAspectRatio="none">
              <line x1="0" y1="10" x2="100" y2="10"/><line x1="0" y1="24" x2="100" y2="24"/><line x1="0" y1="38" x2="100" y2="38"/>
              <polyline class="bytes-in" points={sparkPoints(sparklines().bytes_in)} />
              <polyline class="bytes-out" points={sparkPoints(sparklines().bytes_out)} />
            </svg>
            <div class="quality-row"><span>PACKET LOSS</span><strong>{compact((Array.isArray(sparklines().loss_rate) ? sparklines().loss_rate as unknown[] : []).at(-1))}%</strong><div class="quality-track"><i style={`width:${Math.min(number((Array.isArray(sparklines().loss_rate) ? sparklines().loss_rate as unknown[] : []).at(-1)), 100)}%`}></i></div></div>
          </article>
          <article class="panel peer-panel">
            <div class="panel-title"><div><span>PEERS</span><h3>Current routes</h3></div><button class="text-button" onclick={() => selectView("peers")}>View all →</button></div>
            {#if peers.length}
              {#each peers.slice(0, 4) as peer}
                <button class="compact-peer" onclick={() => { selectedPeer = peer; activeView = "peers"; }}>
                  <span class="peer-avatar">{(peer.display_name || peer.transport_type || "P").slice(0, 1).toUpperCase()}</span>
                  <span><strong>{peer.display_name || shortId(peer.npub)}</strong><small>{peer.transport_type ?? "unknown"} · {peer.connectivity ?? "connected"}</small></span>
                  <i class="live-dot"></i>
                </button>
              {/each}
            {:else}<div class="empty-mini">No authenticated peers yet.</div>{/if}
          </article>
        </section>
      {:else if activeView === "peers"}
        <section class="panel table-panel">
          <div class="panel-title"><div><span>MESH LINKS</span><h3>{peers.length} authenticated peer{peers.length === 1 ? "" : "s"}</h3></div></div>
          {#if detailLoading}<div class="loading">Refreshing peers…</div>
          {:else if peers.length === 0}<div class="empty"><div class="empty-icon">◌</div><h3>No authenticated peers</h3><p>Connect a known peer or enable LAN/Nostr discovery in Settings.</p><button class="primary" onclick={() => (connectOpen = true)}>Connect peer</button></div>
          {:else}
            <div class="data-table peer-table">
              <div class="table-head"><span>PEER</span><span>TRANSPORT</span><span>RELATION</span><span>ADDRESS</span><span>STATE</span></div>
              {#each peers as peer}
                <button class="table-row" class:selected={selectedPeer?.npub === peer.npub} onclick={() => (selectedPeer = peer)}>
                  <span class="peer-cell"><i>{(peer.display_name || "P").slice(0, 1).toUpperCase()}</i><span><strong>{peer.display_name || shortId(peer.npub)}</strong><small>{shortId(peer.npub)}</small></span></span>
                  <span><b class="transport-tag">{peer.transport_type ?? "—"}</b></span>
                  <span>{peer.is_parent ? "Parent" : peer.is_child ? "Child" : "Peer"}</span>
                  <code>{peer.transport_addr ?? peer.ipv6_addr ?? "—"}</code>
                  <span><i class="live-dot"></i>{peer.connectivity ?? "Connected"}</span>
                </button>
              {/each}
            </div>
          {/if}
        </section>
        {#if selectedPeer}
          <aside class="detail-drawer">
            <button class="drawer-close" onclick={() => (selectedPeer = null)}>×</button>
            <div class="peer-avatar large">{(selectedPeer.display_name || "P").slice(0, 1).toUpperCase()}</div>
            <h2>{selectedPeer.display_name || "Mesh peer"}</h2><code>{selectedPeer.npub}</code>
            <dl><div><dt>Connectivity</dt><dd>{selectedPeer.connectivity ?? "—"}</dd></div><div><dt>Mesh IPv6</dt><dd>{selectedPeer.ipv6_addr ?? "—"}</dd></div><div><dt>Transport</dt><dd>{selectedPeer.transport_type ?? "—"}</dd></div><div><dt>Direction</dt><dd>{selectedPeer.direction ?? "—"}</dd></div><div><dt>Tree depth</dt><dd>{selectedPeer.tree_depth ?? "—"}</dd></div></dl>
            <button class="danger" disabled={actionBusy} onclick={() => disconnect(selectedPeer!)}>Disconnect peer</button>
          </aside>
        {/if}
      {:else if activeView === "transports"}
        {#if lanDiscovery}
          <section class="lan-diagnostics">
            <div class="lan-diagnostics-head">
              <div><span>LAN DISCOVERY</span><h2>Local mDNS rendezvous</h2><p>{text(lanDiscovery.service_type)}{lanDiscovery.scope ? ` · scope ${lanDiscovery.scope}` : ""}</p></div>
              <span class="pill {lanStateClass()}">{lanStateLabel()}</span>
            </div>
            <div class="diagnostic-metrics">
              <div><span>UDP LISTENER</span><strong>{lanBindingLabel()}</strong></div>
              <div><span>ADVERTISED PORT</span><strong>{lanDiscovery.advertised_port ?? "—"}</strong></div>
              <div><span>SERVICES RESOLVED</span><strong>{number(lanDiscovery.counters?.services_resolved)}</strong></div>
              <div><span>CANDIDATES</span><strong>{number(lanDiscovery.counters?.candidate_addresses)}</strong></div>
              <div><span>HANDSHAKES</span><strong>{number(lanDiscovery.counters?.handshakes_started)}</strong></div>
              <div><span>START FAILURES</span><strong>{number(lanDiscovery.counters?.handshake_start_failures)}</strong></div>
            </div>
            {#if lanDiscovery.warnings?.length}
              <div class="diagnostic-warning"><strong>Configuration issue</strong>{#each lanDiscovery.warnings as warning}<p>{warning}</p>{/each}<button onclick={() => selectView("settings")}>Open Settings</button></div>
            {:else if lanDiscovery.state === "running" && number(lanDiscovery.counters?.candidate_addresses) === 0}
              <p class="diagnostic-note">The browser is running but has not produced a dialable peer candidate. Check that the other node has LAN discovery enabled and that macOS allows multicast and incoming UDP.</p>
            {/if}
            {#if lanSkipReasons().length}
              <div class="skip-reasons"><span>FILTERED EVENTS</span>{#each lanSkipReasons() as reason}<span><b>{reason.value}</b> {reason.label}</span>{/each}</div>
            {/if}
          </section>
        {:else}
          <section class="lan-diagnostics unsupported"><div><span>LAN DISCOVERY</span><h2>Runtime diagnostics unavailable</h2><p>This FIPS daemon predates discovery diagnostics. Monitoring and configuration continue to work normally.</p></div></section>
        {/if}
        <section class="transport-grid">
          {#each transports as transport}
            <article class="panel transport-card">
              <div class="transport-icon"><Icon name={transport.type === "udp" ? "udp" : transport.type === "tcp" ? "tcp" : transport.type === "tor" ? "tor" : "link"} size={21} /></div>
              <div class="transport-title"><div><span>{transport.type ?? "transport"}</span><h3>{transport.name || transport.local_addr || `Transport ${transport.transport_id}`}</h3></div><span class="pill {String(transport.state).toLowerCase() === 'running' ? 'healthy' : 'degraded'}">{transport.state ?? "Unknown"}</span></div>
              <dl><div><dt>Local address</dt><dd>{transport.local_addr ?? "—"}</dd></div><div><dt>MTU</dt><dd>{transport.mtu ?? "—"}</dd></div>{#if transport.onion_address}<div><dt>Onion address</dt><dd>{transport.onion_address}</dd></div>{/if}</dl>
            </article>
          {:else}
            <article class="panel empty"><div class="empty-icon">⇄</div><h3>No transport data</h3><p>{online ? "The daemon has no configured transport instances." : "Transport details are available when FIPS is running."}</p></article>
          {/each}
        </section>
      {:else if activeView === "access"}
        <section class="access-shell">
          {#if aclLoading && !acl}
            <div class="panel loading">Loading peer access policy…</div>
          {:else if aclError && !acl}
            <article class="panel upgrade-card"><div class="upgrade-icon">⌁</div><h2>Access controls unavailable</h2><p>{aclError}</p><p class="muted">This daemon may predate the <code>show_acl</code> control query.</p><button onclick={() => loadAcl(true)}>Try again</button></article>
          {:else if acl}
            <section class="access-summary">
              <div><span>PEER ACCESS POLICY</span><h2>{accessModeLabel()}</h2><p>{acl.enforcement_active ? "The daemon is enforcing the loaded ACL files." : "The daemon reported that ACL enforcement is not active."}</p></div>
              <span class="pill {acl.enforcement_active ? 'healthy' : 'stopped'}">{acl.enforcement_active ? "Enforced" : "Not enforced"}</span>
              <div class="access-summary-stats"><div><strong>{allowEntries.length}</strong><span>Allowed entries</span></div><div><strong>{denyEntries.length}</strong><span>Blocked entries</span></div><div><strong>{peers.length}</strong><span>Connected peers</span></div></div>
            </section>

            <div class="acl-columns">
              <article class="panel acl-card allow-card">
                <div class="panel-title"><div><span>WHITELIST</span><h3>Always allow these peers</h3></div><button type="button" class="secondary small" onclick={() => openAclModal("allow")}>Add</button></div>
                <p class="acl-path">{acl.allow_file ?? "peers.allow"}</p>
                {#if allowEntries.length}
                  <div class="acl-entry-list">{#each allowEntries as entry}<div><code>{entry}</code><button type="button" class="danger-text" disabled={aclBusy} onclick={() => requestRemoveAclEntry("allow", entry)}>Remove</button></div>{/each}</div>
                {:else}<div class="empty embedded"><p>No explicit whitelist entries.</p><button type="button" onclick={() => openAclModal("allow")}>Add the first allowed peer</button></div>{/if}
              </article>
              <article class="panel acl-card deny-card">
                <div class="panel-title"><div><span>BLOCKLIST</span><h3>Reject these peers</h3></div><button type="button" class="secondary small" onclick={() => openAclModal("deny")}>Add</button></div>
                <p class="acl-path">{acl.deny_file ?? "peers.deny"}</p>
                {#if denyEntries.length}
                  <div class="acl-entry-list">{#each denyEntries as entry}<div><code>{entry}</code><button type="button" class="danger-text" disabled={aclBusy} onclick={() => requestRemoveAclEntry("deny", entry)}>Remove</button></div>{/each}</div>
                {:else}<div class="empty embedded"><p>No blocked peers.</p><button type="button" onclick={() => openAclModal("deny")}>Block a peer</button></div>{/if}
              </article>
            </div>

            <section class="panel access-peer-panel">
              <div class="panel-title"><div><span>LIVE CONNECTIONS</span><h3>Cut an active connection</h3></div><button class="text-button" onclick={() => selectView("peers")}>Open peers →</button></div>
              {#if peers.length}
                <div class="access-peer-list">{#each peers as peer}<div><span class="peer-cell"><i>{(peer.display_name || "P").slice(0, 1).toUpperCase()}</i><span><strong>{peer.display_name || shortId(peer.npub)}</strong><small>{shortId(peer.npub)} · {peer.transport_type ?? "unknown"}</small></span></span><button class="danger" disabled={actionBusy} onclick={() => disconnect(peer)}>Disconnect</button></div>{/each}</div>
              {:else}<div class="empty embedded"><p>No active peer connections.</p></div>{/if}
            </section>

            <div class="access-note"><strong>Changes are live.</strong> FIPS reloads these ACL files automatically. Entries accept npubs, configured aliases, or the <code>ALL</code> wildcard. Adding <code>ALL</code> to the blocklist makes the whitelist strict.</div>
          {/if}
          {#if aclError && acl}<div class="inline-error">{aclError}</div>{/if}
        </section>
      {:else}
        <section class="settings-shell">
          {#if configLoading}<div class="panel loading">Loading daemon configuration…</div>
          {:else if configError && !config}
            <article class="panel upgrade-card"><div class="upgrade-icon">↑</div><h2>Configuration controls unavailable</h2><p>{configError}</p><p class="muted">Status and peer monitoring continue to work with older FIPS releases.</p><button onclick={() => loadConfig(true)}>Try again</button></article>
          {:else if config && guided}
            <div class="settings-header">
              <div><span>ACTIVE SOURCE</span><strong>{config.source === "managed" ? "Managed by FIPS Monitor" : "Operator configuration"}</strong><code>{config.source === "managed" ? config.managed_path : config.base_path}</code></div>
              <div class="segmented"><button class:active={settingsMode === "guided"} onclick={() => (settingsMode = "guided")}>Guided</button><button class:active={settingsMode === "yaml"} onclick={() => (settingsMode = "yaml")}>Advanced YAML</button></div>
            </div>

            {#if settingsMode === "guided"}
              <div class="settings-layout">
                <nav class="settings-nav">
                  <button class:active={settingsSection === "identity"} onclick={() => (settingsSection = "identity")}>Identity & node</button>
                  <button class:active={settingsSection === "network"} onclick={() => (settingsSection = "network")}>TUN & DNS</button>
                  <button class:active={settingsSection === "discovery"} onclick={() => (settingsSection = "discovery")}>Discovery</button>
                  <button class:active={settingsSection === "transports"} onclick={() => (settingsSection = "transports")}>Transports</button>
                  <button class:active={settingsSection === "peers"} onclick={() => (settingsSection = "peers")}>Persistent peers <em>{guided.peers.length}</em></button>
                </nav>
                <div class="settings-form">
                  {#if settingsSection === "identity"}
                    <div class="form-title"><span>IDENTITY & NODE</span><h2>How this node participates</h2><p>Identity secrets stay inside the daemon. Preserved values are never returned to this app.</p></div>
                    <label class="toggle-row"><span><strong>Persistent identity</strong><small>Keep the same npub and mesh address across restarts.</small></span><input type="checkbox" bind:checked={guided.persistent} onchange={syncGuided}/><i></i></label>
                    <label class="toggle-row"><span><strong>Leaf-only node</strong><small>Participate at the edge without routing traffic for other nodes.</small></span><input type="checkbox" bind:checked={guided.leafOnly} onchange={syncGuided}/><i></i></label>
                    <label class="field"><span>Log level</span><select bind:value={guided.logLevel} onchange={syncGuided}><option value="error">Error</option><option value="warn">Warn</option><option value="info">Info</option><option value="debug">Debug</option><option value="trace">Trace</option></select></label>
                  {:else if settingsSection === "network"}
                    <div class="form-title"><span>TUN & DNS</span><h2>Local mesh networking</h2><p>Manage the macOS tunnel and the local <code>.fips</code> resolver.</p></div>
                    <label class="toggle-row"><span><strong>TUN interface</strong><small>Create the local IPv6 mesh interface.</small></span><input type="checkbox" bind:checked={guided.tunEnabled} onchange={syncGuided}/><i></i></label>
                    <div class="field-row"><label class="field"><span>Interface name</span><input bind:value={guided.tunName} onchange={syncGuided}/></label><label class="field"><span>MTU</span><input type="number" min="1280" max="9000" bind:value={guided.tunMtu} onchange={syncGuided}/></label></div>
                    <label class="toggle-row"><span><strong>DNS responder</strong><small>Resolve mesh names for applications on this Mac.</small></span><input type="checkbox" bind:checked={guided.dnsEnabled} onchange={syncGuided}/><i></i></label>
                    <label class="field"><span>DNS port</span><input type="number" min="1" max="65535" bind:value={guided.dnsPort} onchange={syncGuided}/></label>
                  {:else if settingsSection === "discovery"}
                    <div class="form-title"><span>DISCOVERY</span><h2>Find other FIPS nodes</h2><p>Discovery finds endpoints; Noise authentication still verifies peer identity.</p></div>
                    <label class="toggle-row"><span><strong>LAN discovery</strong><small>Use mDNS to find nodes on the same local network.</small></span><input type="checkbox" bind:checked={guided.lanDiscovery} onchange={syncGuided}/><i></i></label>
                    {#if guidedLanIssue}<div class="inline-warning"><div><strong>LAN peers cannot connect yet</strong><p>{guidedLanIssue.message}</p></div><button onclick={repairLanDiscoveryTransport}>Use {guidedLanIssue.suggestedBind}</button></div>{/if}
                    <label class="toggle-row"><span><strong>Nostr rendezvous</strong><small>Advertise and discover endpoints through configured relays.</small></span><input type="checkbox" bind:checked={guided.nostrDiscovery} onchange={syncGuided}/><i></i></label>
                    <div class="info-box">Relay URLs, policy, application scope, STUN servers, and privacy controls remain available in Advanced YAML.</div>
                  {:else if settingsSection === "transports"}
                    <div class="form-title"><span>TRANSPORTS</span><h2>Reach the mesh</h2><p>Configure the common macOS transports. Tor and Nym settings remain in Advanced YAML.</p></div>
                    <div class="transport-setting"><label class="toggle-row"><span><strong>UDP</strong><small>Low-overhead mesh traffic and NAT traversal.</small></span><input type="checkbox" bind:checked={guided.udpEnabled} onchange={syncGuided}/><i></i></label>{#if guided.udpEnabled}<label class="field"><span>Bind address</span><input bind:value={guided.udpBind} onchange={syncGuided}/></label>{/if}</div>
                    {#if guidedLanIssue}<div class="inline-warning"><div><strong>LAN discovery needs a reachable UDP listener</strong><p>{guidedLanIssue.message}</p></div><button onclick={repairLanDiscoveryTransport}>Use {guidedLanIssue.suggestedBind}</button></div>{/if}
                    <div class="transport-setting"><label class="toggle-row"><span><strong>TCP</strong><small>Stream transport for reachable peers.</small></span><input type="checkbox" bind:checked={guided.tcpEnabled} onchange={syncGuided}/><i></i></label>{#if guided.tcpEnabled}<label class="field"><span>Bind address</span><input bind:value={guided.tcpBind} onchange={syncGuided}/></label>{/if}</div>
                  {:else}
                    <div class="form-title peer-form-title"><div><span>PERSISTENT PEERS</span><h2>Bootstrap connections</h2><p>These peers are restored whenever FIPS starts.</p></div><button class="secondary small" onclick={addPeer}>Add peer</button></div>
                    {#each guided.peers as peer, index}
                      <div class="peer-editor">
                        <div class="peer-editor-title"><strong>Peer {index + 1}</strong><button class="danger-text" onclick={() => removePeer(index)}>Remove</button></div>
                        <label class="field wide"><span>npub</span><input placeholder="npub1…" bind:value={peer.npub} onchange={syncGuided}/></label>
                        <div class="field-row"><label class="field"><span>Alias</span><input placeholder="Office gateway" bind:value={peer.alias} onchange={syncGuided}/></label><label class="field"><span>Connect policy</span><select bind:value={peer.connectPolicy} onchange={syncGuided}><option value="auto_connect">Auto connect</option><option value="manual">Manual</option></select></label></div>
                        {#if peer.addresses[0]}<div class="field-row"><label class="field"><span>Transport</span><select bind:value={peer.addresses[0].transport} onchange={syncGuided}><option value="udp">UDP</option><option value="tcp">TCP</option><option value="tor">Tor</option><option value="nym">Nym</option></select></label><label class="field"><span>Address</span><input placeholder="host.example:2121" bind:value={peer.addresses[0].addr} onchange={syncGuided}/></label></div>{/if}
                        <label class="toggle-row compact"><span><strong>Allow Nostr rendezvous</strong></span><input type="checkbox" bind:checked={peer.viaNostr} onchange={syncGuided}/><i></i></label>
                      </div>
                    {:else}<div class="empty embedded"><p>No persistent peers are configured.</p><button onclick={addPeer}>Add the first peer</button></div>{/each}
                  {/if}
                </div>
              </div>
            {:else}
              <div class="yaml-panel settings-form"><div class="form-title"><span>ADVANCED YAML</span><h2>Complete macOS configuration</h2><p>Every daemon key is available here. Secret sentinels preserve existing values without revealing them.</p></div><textarea aria-label="FIPS YAML configuration" spellcheck="false" bind:value={draftYaml} oninput={syncYamlToGuided}></textarea><div class="editor-footer"><code>{draftYaml.length.toLocaleString()} / 131,072 bytes</code><span>node.control settings are read-only in managed mode</span></div></div>
            {/if}

            {#if draftError}<div class="inline-error">{draftError}</div>{/if}
            {#if configError}<div class="inline-error">{configError}</div>{/if}
            {#if applyMessage}<div class="apply-message">{applyMessage}</div>{/if}

            {#if validation}
              <section class="review-panel">
                <div class="review-head"><div><span>{validation.valid ? "REVIEW CHANGES" : "VALIDATION ERRORS"}</span><h2>{validation.valid ? `${validation.diff.length} semantic change${validation.diff.length === 1 ? "" : "s"}` : "Configuration needs attention"}</h2></div>{#if validation.activation}<span class="impact {validation.activation}">{validation.activation === "restart" ? "Daemon restart" : validation.activation === "hot_peers" ? "Live peer update" : "No runtime change"}</span>{/if}</div>
                {#if validation.errors.length}<div class="validation-errors">{#each validation.errors as error}<div><code>{error.path}</code><p>{error.message}</p></div>{/each}</div>{/if}
                {#if validation.warnings.length}<div class="warnings">{#each validation.warnings as warning}<p>⚠ {warning}</p>{/each}</div>{/if}
                {#if validation.valid}<div class="diff-list">{#each validation.diff as change}<div><code>{change.path || "/"}</code><span><del>{formatDiffValue(change.before)}</del><b>→</b><ins>{formatDiffValue(change.after)}</ins></span></div>{:else}<p class="muted">Only formatting changed. The managed file will be updated without restarting FIPS.</p>{/each}</div>{/if}
              </section>
            {/if}

            <div class="settings-actions">
              <button class="danger-text" disabled={applyBusy || config.source !== "managed"} onclick={resetConfig}>Restore operator file</button>
              <span></span>
              <button class="settings-action" disabled={applyBusy} onclick={() => { draftYaml = config!.yaml; guided = readGuidedDraft(draftYaml); validation = null; }}>Discard changes</button>
              {#if validation?.valid}<button class="primary settings-action" disabled={applyBusy} onclick={applyDraft}>{applyBusy ? "Applying…" : "Apply configuration"}</button>{:else}<button class="primary settings-action" disabled={applyBusy || !!draftError} onclick={reviewConfig}>{validation ? "Validate again" : "Review changes"}</button>{/if}
            </div>
          {/if}

          <details class="developer-settings">
            <summary>Development connection</summary>
            <p>Override the fixed system socket when testing a source-built daemon. Production uses <code>/var/run/fips/control.sock</code>.</p>
            <div class="path-field"><input bind:value={developmentPath}/><button onclick={changeSocketPath}>Use socket</button></div>
          </details>
        </section>
      {/if}

      {#if detailError}<div class="floating-error">{detailError}</div>{/if}
    </div>
  </main>
</div>

{#if connectOpen}
  <div class="modal-backdrop" role="presentation" onclick={(event) => event.target === event.currentTarget && (connectOpen = false)}>
    <form class="modal" onsubmit={(event) => { event.preventDefault(); void connect(); }}>
      <button type="button" class="drawer-close" onclick={() => (connectOpen = false)}>×</button>
      <span>DIRECT CONNECTION</span><h2>Connect a peer</h2><p>Verify the peer’s npub and endpoint through a trusted channel before connecting.</p>
      <label class="field"><span>Peer npub</span><input required placeholder="npub1…" bind:value={connectNpub}/></label>
      <div class="field-row"><label class="field"><span>Transport</span><select bind:value={connectTransport}><option value="udp">UDP</option><option value="tcp">TCP</option><option value="tor">Tor</option><option value="nym">Nym</option></select></label><label class="field"><span>Endpoint</span><input required placeholder="host.example:2121" bind:value={connectAddress}/></label></div>
      <div class="modal-actions"><button type="button" onclick={() => (connectOpen = false)}>Cancel</button><button class="primary" disabled={actionBusy}>{actionBusy ? "Connecting…" : "Connect peer"}</button></div>
    </form>
  </div>
{/if}

{#if aclModal}
  <div class="modal-backdrop" role="presentation" onclick={(event) => event.target === event.currentTarget && (aclModal = null)}>
    <form class="modal" onsubmit={(event) => { event.preventDefault(); void addAclEntry(aclModal!); }}>
      <button type="button" class="drawer-close" onclick={() => (aclModal = null)}>×</button>
      <span>PEER ACCESS CONTROL</span><h2>{aclModal === "allow" ? "Add to whitelist" : "Add to blocklist"}</h2><p>Use a peer npub, a configured host alias, or <code>ALL</code>. FIPS reloads the file automatically. macOS may ask for administrator approval.</p>
      <label class="field"><span>{aclModal === "allow" ? "Allowed peer" : "Blocked peer"}</span><input required placeholder="npub1… or alias" bind:value={aclEntry}/></label>
      {#if aclError}<div class="modal-error"><strong>Rule was not saved</strong><span>{aclError}</span></div>{/if}
      <div class="modal-actions"><button type="button" onclick={() => (aclModal = null)}>Cancel</button><button class="primary" disabled={aclBusy}>{aclBusy ? "Saving…" : "Save rule"}</button></div>
    </form>
  </div>
{/if}

{#if aclRemoval}
  <div class="modal-backdrop" role="presentation" onclick={(event) => event.target === event.currentTarget && (aclRemoval = null)}>
    <form class="modal" onsubmit={(event) => { event.preventDefault(); void removeAclEntry(aclRemoval!.list, aclRemoval!.entry); }}>
      <button type="button" class="drawer-close" onclick={() => (aclRemoval = null)}>×</button>
      <span>PEER ACCESS CONTROL</span><h2>Remove rule?</h2>
      <p>Remove <code>{aclRemoval.entry}</code> from the {aclRemoval.list === "allow" ? "whitelist" : "blocklist"}? FIPS will reload the policy after the file changes. macOS may ask for administrator approval.</p>
      {#if aclError}<div class="modal-error"><strong>Rule was not removed</strong><span>{aclError}</span></div>{/if}
      <div class="modal-actions"><button type="button" onclick={() => (aclRemoval = null)}>Cancel</button><button type="submit" class="danger" disabled={aclBusy}>{aclBusy ? "Removing…" : "Remove rule"}</button></div>
    </form>
  </div>
{/if}

{#if toast}<button class="toast" onclick={() => (toast = "")}>{toast}<span>×</span></button>{/if}

<style>
  :global(*) { box-sizing: border-box; }
  :global(html) { background: #07100e; color-scheme: dark; }
  :global(body) { margin: 0; min-width: 820px; min-height: 600px; overflow: hidden; font-family: -apple-system, BlinkMacSystemFont, "SF Pro Display", "Helvetica Neue", sans-serif; color: #eaf4ef; background: #07100e; -webkit-font-smoothing: antialiased; }
  :global(button), :global(input), :global(select), :global(textarea) { font: inherit; }
  :global(button) { color: inherit; }
  .app-shell { height: 100vh; display: grid; grid-template-columns: 218px 1fr; background: radial-gradient(circle at 68% -25%, rgba(49, 177, 135, .09), transparent 42%), #08120f; }
  .sidebar { position: relative; display: flex; flex-direction: column; padding: 27px 17px 20px; border-right: 1px solid #1a2924; background: rgba(5, 13, 11, .94); }
  .brand { display: flex; align-items: center; gap: 13px; padding: 0 9px 29px; }
  .brand > div:last-child { display: flex; flex-direction: column; line-height: 1.05; }
  .brand strong { color: #f6fcf9; font-size: 18px; letter-spacing: .07em; }
  .brand span { color: #799188; font-size: 12px; letter-spacing: .14em; text-transform: uppercase; }
  .brand-mark { width: 34px; height: 34px; color: #59e5b1; filter: drop-shadow(0 0 8px rgba(81,224,172,.18)); }
  nav { display: flex; flex-direction: column; gap: 4px; }
  nav button { display: flex; align-items: center; gap: 12px; width: 100%; padding: 10px 12px; border: 0; border-radius: 8px; color: #83958f; background: transparent; text-align: left; cursor: pointer; transition: .16s; }
  nav button:hover { color: #d8e9e2; background: #101d19; } nav button.active { color: #e9fff7; background: #14251f; }
  nav button em { margin-left: auto; font-size: 10px; font-style: normal; color: #5f746c; }.nav-icon { display: grid; width: 18px; place-items: center; color: #6e827b; }.active .nav-icon { color: #51e0ac; }
  .sidebar-node { margin-top: auto; display: flex; gap: 10px; align-items: center; padding: 13px 11px; border: 1px solid #1b2c26; border-radius: 10px; background: #0c1814; }
  .sidebar-node div { display: flex; flex-direction: column; gap: 2px; }.sidebar-node strong { font-size: 12px; font-weight: 600; }.sidebar-node small { font-size: 10px; color: #62776f; }
  .status-dot,.mini-dot,.live-dot { display: inline-block; flex: 0 0 auto; width: 8px; height: 8px; border-radius: 50%; background: #67746f; box-shadow: 0 0 0 3px rgba(103,116,111,.12); }.status-dot.healthy,.live-dot,.mini-dot.running { background: #4bdda6; box-shadow: 0 0 0 3px rgba(75,221,166,.1), 0 0 12px rgba(75,221,166,.35); }.status-dot.degraded { background: #e4b85d; }.status-dot.permission_denied,.status-dot.incompatible { background: #e47869; }
  main { min-width: 0; position: relative; overflow: hidden; }
  .topbar { height: 88px; display: flex; align-items: center; justify-content: space-between; padding: 0 31px; border-bottom: 1px solid #1a2924; background: rgba(8,18,15,.8); }
  .topbar p,.card-heading>span:first-child,.panel-title>div>span,.form-title>span,.review-head>div>span,.modal>span,.settings-header>div>span { margin: 0 0 4px; font-size: 9px; line-height: 1; font-weight: 700; letter-spacing: .16em; color: #61776e; }
  h1,h2,h3,p { margin-top: 0; }.topbar h1 { margin: 0; font-size: 21px; font-weight: 570; letter-spacing: -.02em; }.header-actions { display: flex; align-items: center; gap: 10px; }.checked { color: #52665e; font-size: 10px; }
  button { border: 1px solid #293a34; border-radius: 7px; padding: 8px 13px; background: #111e1a; cursor: pointer; transition: border-color .16s, background .16s, transform .12s; } button:hover:not(:disabled) { border-color: #3c5a50; background: #172721; } button:active:not(:disabled) { transform: translateY(1px); } button:disabled { opacity: .45; cursor: default; }
  button.primary { border-color: #4adea6; color: #06110d; background: #52e3ad; font-weight: 650; } button.primary:hover:not(:disabled) { background: #6bedbc; border-color: #6bedbc; }.small { padding: 7px 11px; font-size: 11px; }.icon-button { display: grid; width: 32px; height: 32px; padding: 0; place-items: center; color: #88a097; }
  .content { position: relative; height: calc(100vh - 88px); padding: 23px 30px 34px; overflow: auto; }
  .health-banner { display: flex; align-items: center; gap: 13px; margin-bottom: 18px; padding: 12px 15px; border: 1px solid #59482c; border-radius: 9px; background: #1e1a11; }.health-banner.permission_denied,.health-banner.incompatible { border-color: #59322d; background: #201311; }.health-banner strong { font-size: 12px; }.health-banner p { margin: 2px 0 0; color: #a59476; font-size: 11px; }.health-banner button,.health-banner .hint { margin-left: auto; font-size: 10px; }.health-symbol { display: grid; place-items: center; width: 27px; height: 27px; border-radius: 50%; color: #e9bd67; background: #322a18; }
  .hero-grid { display: grid; grid-template-columns: minmax(0, 1.7fr) minmax(220px, .72fr); gap: 14px; }.node-card,.tun-card,.panel,.stat-grid article { border: 1px solid #1d3029; border-radius: 11px; background: linear-gradient(145deg, rgba(17,33,27,.94), rgba(10,22,18,.96)); box-shadow: 0 15px 40px rgba(0,0,0,.08); }.node-card,.tun-card { padding: 18px 19px; }.card-heading,.panel-title,.review-head,.transport-title { display: flex; align-items: flex-start; justify-content: space-between; }.pill { padding: 4px 8px; border-radius: 100px; color: #92a59e; background: #182721; font-size: 9px; font-weight: 650; }.pill.healthy { color: #57e5b1; background: rgba(60,192,143,.1); }.pill.degraded,.pill.stopped { color: #e3b863; background: rgba(227,184,99,.1); }.pill.permission_denied,.pill.incompatible { color: #e9897b; background: rgba(233,137,123,.1); }
  .identity-row { display: flex; align-items: center; gap: 15px; margin: 19px 0 20px; }.identity-copy { min-width: 0; flex: 1; }.identity-row h2 { margin: 0 0 3px; overflow: hidden; font-size: 16px; font-weight: 580; text-overflow: ellipsis; white-space: nowrap; }.identity-key { display: flex; min-width: 0; align-items: flex-start; gap: 6px; }.identity-key code { min-width: 0; overflow-wrap: anywhere; line-height: 1.45; }.identity-row code,.tun-route code { color: #718980; font-size: 10px; }.copy-button { display: grid; flex: 0 0 25px; width: 25px; height: 25px; margin-top: -5px; padding: 0; place-items: center; border-color: transparent; color: #718980; background: transparent; }.copy-button:hover:not(:disabled) { color: #66ddb1; background: #13251f; }.node-glyph { display: grid; flex: 0 0 48px; height: 48px; place-items: center; color: #52dfa9; filter: drop-shadow(0 0 8px rgba(75,221,166,.16)); }
  .metrics { display: grid; gap: 16px; padding-top: 14px; border-top: 1px solid #1d3029; }.metrics.four { grid-template-columns: repeat(4,1fr); }.metrics div { display: flex; flex-direction: column; gap: 5px; }.metrics span,.quality-row>span { color: #586e65; font-size: 8px; font-weight: 700; letter-spacing: .13em; }.metrics strong { font-size: 12px; font-weight: 570; }
  .tun-card { display: flex; min-width: 0; flex-direction: column; }.tun-name { margin: 24px 0 3px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 25px; font-weight: 500; }.tun-card>p { color: #62776e; font-size: 10px; }.tun-route { min-width: 0; margin-top: auto; padding-top: 15px; border-top: 1px solid #1d3029; }.tun-route span { display: block; margin-bottom: 4px; color: #53685f; font-size: 8px; text-transform: uppercase; letter-spacing: .12em; }.tun-route code { display: block; max-width: 100%; overflow-wrap: anywhere; line-height: 1.45; word-break: break-word; }
  .stat-grid { display: grid; grid-template-columns: repeat(4,1fr); gap: 11px; margin-top: 12px; }.stat-grid article { min-width: 0; display: flex; align-items: flex-end; justify-content: space-between; padding: 13px 14px; }.stat-grid article>div:first-child { display: flex; flex-direction: column; gap: 5px; }.stat-grid span { color: #536a61; font-size: 7px; font-weight: 700; letter-spacing: .11em; }.stat-grid strong { font-size: 21px; font-weight: 540; }.stat-grid svg { width: 45%; height: 31px; overflow: visible; }.stat-grid polyline,.traffic-chart polyline { fill: none; stroke: #48dba6; stroke-width: 1.7; vector-effect: non-scaling-stroke; }.transport-dots { display: flex; gap: 4px; padding-bottom: 5px; }.transport-dots i { width: 5px; height: 5px; border-radius: 50%; background: #4bdca7; }
  .lan-summary { display: grid; grid-template-columns: 9px minmax(200px,1fr) auto auto auto; gap: 12px; align-items: center; width: 100%; margin-top: 12px; padding: 10px 3px; border-width: 1px 0; border-radius: 0; border-color: #1b3028; background: transparent; text-align: left; }.lan-summary:hover:not(:disabled) { border-color: #27483b; background: rgba(19,38,31,.45); }.lan-summary>span { display: flex; flex-direction: column; gap: 2px; }.lan-summary strong { font-size: 10.5px; }.lan-summary b { font-size: 11px; font-weight: 600; }.lan-summary small { color: #60766d; font-size: 8.5px; }.lan-summary>i { color: #64b899; font-size: 9px; font-style: normal; }
  .dashboard-grid { display: grid; grid-template-columns: minmax(0,1.45fr) minmax(245px,.75fr); gap: 13px; margin-top: 12px; }.panel { padding: 17px 18px; }.panel-title h3 { margin: 5px 0 0; font-size: 13px; font-weight: 540; }.legend { color: #5d726a; font-size: 8px; }.legend i { display: inline-block; width: 7px; height: 2px; margin: 0 4px 2px 9px; background: #48dba6; }.legend i:nth-child(2) { background: #51766b; }
  .traffic-chart { width: 100%; height: 80px; margin: 13px 0 5px; overflow: visible; }.traffic-chart line { stroke: #1d3029; stroke-width: .5; }.traffic-chart .bytes-out { stroke: #52796d; }.quality-row { display: flex; align-items: center; gap: 10px; }.quality-row strong { font-size: 9px; }.quality-track { flex: 1; height: 3px; border-radius: 4px; background: #1d3029; overflow: hidden; }.quality-track i { display: block; height: 100%; background: #e4b75d; }
  .text-button,.danger-text { padding: 0; border: 0; color: #5dcba3; background: transparent; font-size: 9px; }.compact-peer { display: grid; grid-template-columns: 31px 1fr 8px; align-items: center; gap: 10px; width: 100%; padding: 8px 0; border: 0; border-radius: 0; border-top: 1px solid #192b24; background: transparent; text-align: left; }.compact-peer:first-of-type { margin-top: 10px; }.compact-peer span:nth-child(2) { display: flex; flex-direction: column; gap: 2px; min-width: 0; }.compact-peer strong { overflow: hidden; font-size: 10px; text-overflow: ellipsis; }.compact-peer small { color: #586d65; font-size: 8px; }.peer-avatar,.peer-cell>i { display: grid; place-items: center; width: 30px; height: 30px; border-radius: 8px; color: #67dab0; background: #193027; font-size: 10px; font-style: normal; font-weight: 700; }.peer-avatar.large { width: 50px; height: 50px; margin: 16px 0 12px; border-radius: 14px; font-size: 17px; }.empty-mini { padding: 25px 0; color: #60756d; text-align: center; font-size: 10px; }
  .table-panel { padding: 0; overflow: hidden; }.table-panel>.panel-title { padding: 18px 19px; }.data-table { border-top: 1px solid #1c2d27; }.table-head,.table-row { display: grid; grid-template-columns: 1.7fr .65fr .6fr 1.1fr .72fr; gap: 15px; align-items: center; padding: 10px 18px; }.table-head { color: #52675f; background: #0b1713; font-size: 8px; font-weight: 700; letter-spacing: .12em; }.table-row { width: 100%; border: 0; border-bottom: 1px solid #182a23; border-radius: 0; color: #81968e; background: transparent; text-align: left; font-size: 10px; }.table-row:hover,.table-row.selected { background: #11221b; }.table-row code { overflow: hidden; color: #789087; text-overflow: ellipsis; }.peer-cell { display: flex; align-items: center; gap: 10px; min-width: 0; }.peer-cell>span { display: flex; flex-direction: column; min-width: 0; }.peer-cell strong,.peer-cell small { overflow: hidden; text-overflow: ellipsis; }.peer-cell strong { color: #dbe9e3; font-size: 10px; }.peer-cell small { color: #51665e; font-size: 8px; }.transport-tag { color: #5bd6a9; font-size: 8px; text-transform: uppercase; }.table-row>span:last-child { display: flex; align-items: center; gap: 7px; }
  .empty { min-height: 300px; display: flex; flex-direction: column; align-items: center; justify-content: center; color: #71867e; text-align: center; }.empty h3 { margin: 8px 0; color: #d7e8e1; font-size: 14px; }.empty p { max-width: 340px; font-size: 11px; }.empty-icon { display: grid; place-items: center; width: 52px; height: 52px; border: 1px solid #243a32; border-radius: 50%; color: #4bd6a3; background: #102119; font-size: 21px; }
  .detail-drawer { position: fixed; z-index: 5; top: 88px; right: 0; bottom: 0; width: 315px; padding: 24px; border-left: 1px solid #24372f; background: #0c1814; box-shadow: -20px 0 50px rgba(0,0,0,.28); }.drawer-close { position: absolute; top: 15px; right: 15px; border: 0; background: transparent; color: #71857e; font-size: 20px; }.detail-drawer h2 { margin: 0 0 3px; font-size: 17px; }.detail-drawer>code { display: block; overflow-wrap: anywhere; color: #5b7168; font-size: 8px; }.detail-drawer dl,.transport-card dl { margin: 25px 0; }.detail-drawer dl div,.transport-card dl div { display: grid; grid-template-columns: 1fr 1.3fr; gap: 8px; padding: 9px 0; border-bottom: 1px solid #1b2d26; font-size: 10px; }.detail-drawer dt,.transport-card dt { color: #566b63; }.detail-drawer dd,.transport-card dd { margin: 0; overflow-wrap: anywhere; color: #a8bbb4; text-align: right; }.danger { width: 100%; border-color: #53342e; color: #e58576; background: #241512; }
  .lan-diagnostics { margin-bottom: 18px; padding: 2px 0 16px; border-bottom: 1px solid #22382f; }.lan-diagnostics-head { display: flex; align-items: flex-start; justify-content: space-between; }.lan-diagnostics-head>div>span,.lan-diagnostics.unsupported>div>span,.skip-reasons>span:first-child { color: #61776e; font-size: 9px; font-weight: 700; letter-spacing: .16em; }.lan-diagnostics h2 { margin: 5px 0 4px; font-size: 17px; font-weight: 580; }.lan-diagnostics-head p,.lan-diagnostics.unsupported p { margin: 0; color: #647a71; font-size: 10px; }.diagnostic-metrics { display: grid; grid-template-columns: minmax(170px,1.8fr) repeat(5,minmax(75px,1fr)); gap: 18px; margin-top: 17px; padding: 13px 0; border-top: 1px solid #1b3028; border-bottom: 1px solid #1b3028; }.diagnostic-metrics>div { min-width: 0; display: flex; flex-direction: column; gap: 5px; }.diagnostic-metrics span { color: #536a61; font-size: 7.5px; font-weight: 700; letter-spacing: .1em; }.diagnostic-metrics strong { overflow: hidden; font-size: 11px; font-weight: 570; text-overflow: ellipsis; white-space: nowrap; }.diagnostic-warning { position: relative; margin-top: 13px; padding: 2px 120px 2px 12px; border-left: 2px solid #d6a955; color: #d9b66f; }.diagnostic-warning strong { font-size: 10.5px; }.diagnostic-warning p,.diagnostic-note { margin: 4px 0 0; color: #9b8760; font-size: 10px; line-height: 1.45; }.diagnostic-warning button { position: absolute; top: 1px; right: 0; padding: 6px 9px; font-size: 9px; }.diagnostic-note { color: #73877f; }.skip-reasons { display: flex; flex-wrap: wrap; gap: 8px 16px; margin-top: 12px; color: #73877f; font-size: 9px; }.skip-reasons>span:first-child { margin-right: 3px; }.skip-reasons b { color: #a5b9b1; }.lan-diagnostics.unsupported { padding-top: 2px; }.transport-grid { display: grid; grid-template-columns: repeat(2,minmax(0,1fr)); gap: 13px; }.transport-card { display: grid; grid-template-columns: 43px 1fr; gap: 15px; }.transport-icon { display: grid; place-items: center; width: 43px; height: 43px; border: 1px solid #284039; border-radius: 9px; color: #54d9a8; background: #10211b; }.transport-title { grid-column: 2; }.transport-title span:first-child { color: #5bd7aa; font-size: 9px; font-weight: 700; text-transform: uppercase; letter-spacing: .13em; }.transport-title h3 { margin: 5px 0; font-size: 13px; }.transport-card dl { grid-column: 1 / -1; margin-bottom: 0; }
  .settings-shell { max-width: 940px; margin: 0 auto; }.settings-header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 16px; padding: 0 0 15px; border-bottom: 1px solid #1b3028; }.settings-header>div:first-child { display: flex; min-width: 0; flex-direction: column; gap: 3px; }.settings-header strong { font-size: 13px; font-weight: 590; }.settings-header code { overflow: hidden; color: #60766d; font-size: 9.5px; text-overflow: ellipsis; white-space: nowrap; }.segmented { display: flex; flex: 0 0 auto; padding: 3px; border: 1px solid #20332b; border-radius: 7px; background: #091511; }.segmented button { padding: 6px 10px; border: 0; background: transparent; font-size: 10.5px; }.segmented button.active { color: #dff8ee; background: #183027; }
  .settings-layout { display: grid; grid-template-columns: 188px minmax(0,1fr); gap: 0; }.settings-nav { align-self: start; padding: 3px 20px 3px 0; border-right: 1px solid #1a2d26; }.settings-nav button { padding: 10px 11px; border-radius: 6px; font-size: 12px; line-height: 1.25; }.settings-nav button.active { color: #e9fff7; background: #13251e; }.settings-nav button em { font-size: 10.5px; }.settings-form { min-width: 0; min-height: 392px; padding: 3px 0 28px 28px; }.form-title { margin-bottom: 14px; padding-bottom: 15px; border-bottom: 1px solid #1d3029; }.form-title h2 { margin: 5px 0 6px; font-size: 18px; font-weight: 580; letter-spacing: -.01em; }.form-title p,.modal>p { max-width: 620px; margin: 0; color: #71867e; font-size: 11.5px; line-height: 1.5; }.toggle-row { position: relative; display: flex; align-items: center; justify-content: space-between; min-height: 59px; padding: 11px 0; border-bottom: 1px solid #192b24; cursor: pointer; }.toggle-row>span { display: flex; flex-direction: column; gap: 4px; }.toggle-row strong { font-size: 12px; font-weight: 580; }.toggle-row small { color: #6b8078; font-size: 10.5px; line-height: 1.35; }.toggle-row input { position: absolute; opacity: 0; }.toggle-row>i { position: relative; width: 31px; height: 17px; border-radius: 20px; background: #263630; transition: .2s; }.toggle-row>i::after { content: ""; position: absolute; top: 3px; left: 3px; width: 11px; height: 11px; border-radius: 50%; background: #778981; transition: .2s; }.toggle-row input:checked+i { background: #2c765d; }.toggle-row input:checked+i::after { left: 17px; background: #69e2b6; }.toggle-row.compact { min-height: 40px; border: 0; }.field { display: flex; flex-direction: column; gap: 6px; margin-top: 14px; }.field>span { color: #7a8f87; font-size: 10.5px; font-weight: 600; }.field input,.field select,.path-field input { width: 100%; height: 36px; padding: 0 10px; border: 1px solid #263a32; border-radius: 6px; outline: 0; color: #c8d9d2; background: #091511; font-size: 11.5px; }.field input:focus,.field select:focus,.path-field input:focus,textarea:focus { border-color: #378a6b; }.field-row { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }.info-box { margin-top: 18px; padding: 3px 0 3px 12px; border-left: 2px solid #2f6652; color: #789087; font-size: 10.5px; line-height: 1.5; }.inline-warning { display: flex; align-items: center; gap: 14px; margin: 10px 0; padding: 10px 0 10px 12px; border-left: 2px solid #d3a651; }.inline-warning>div { min-width: 0; flex: 1; }.inline-warning strong { color: #e3bd72; font-size: 10.5px; }.inline-warning p { margin: 3px 0 0; color: #9a855e; font-size: 10px; line-height: 1.4; }.inline-warning button { flex: 0 0 auto; padding: 6px 9px; color: #dfbc76; font-size: 9px; }.transport-setting { margin: 0; padding: 0 0 15px; border-bottom: 1px solid #1d3029; }.transport-setting + .transport-setting { margin-top: 2px; }
  .peer-form-title { display: flex; align-items: flex-start; justify-content: space-between; }.peer-editor { margin: 0; padding: 15px 0 18px; border-bottom: 1px solid #20362d; }.peer-editor-title { display: flex; justify-content: space-between; }.peer-editor-title strong { font-size: 11px; }.danger-text { color: #df7e70; }.empty.embedded { min-height: 180px; border-top: 1px dashed #294038; border-bottom: 1px dashed #294038; border-radius: 0; }
  .yaml-panel { padding-left: 0; }.yaml-panel textarea { width: 100%; min-height: 415px; resize: vertical; padding: 14px; border: 1px solid #233831; border-radius: 6px; outline: 0; color: #b9d8ca; background: #06100d; font: 10.5px/1.58 ui-monospace, SFMono-Regular, Menlo, monospace; tab-size: 2; }.editor-footer { display: flex; justify-content: space-between; margin-top: 8px; color: #60756d; font-size: 9px; }.inline-error,.apply-message { margin: 10px 0; padding: 11px 13px; border: 1px solid #57342e; border-radius: 6px; color: #de8b7d; background: #201411; font-size: 10.5px; }.apply-message { border-color: #2a4c3d; color: #84cbb0; background: #102019; }.review-panel { margin-top: 14px; padding: 17px 0; border-top: 1px solid #244036; border-bottom: 1px solid #244036; }.review-head h2 { margin: 5px 0 0; font-size: 15px; }.impact { padding: 5px 9px; border-radius: 20px; color: #76d4b2; background: #143025; font-size: 9px; }.impact.restart { color: #dfb567; background: #2a2214; }.diff-list { margin-top: 14px; }.diff-list>div { display: grid; grid-template-columns: minmax(120px,.65fr) 1.4fr; gap: 15px; padding: 10px 0; border-top: 1px solid #1c3028; font-size: 9px; }.diff-list>div>span { display: grid; grid-template-columns: 1fr 15px 1fr; gap: 6px; min-width: 0; }.diff-list del,.diff-list ins { overflow: hidden; color: #967b76; text-decoration: none; text-overflow: ellipsis; }.diff-list ins { color: #7bb69f; }.diff-list b { color: #536960; text-align: center; }.warnings { color: #d2ae68; font-size: 9px; }.validation-errors { margin-top: 14px; }.validation-errors>div { padding: 10px 12px; border: 1px solid #54332e; border-radius: 6px; background: #201411; }.validation-errors code { color: #e29183; font-size: 9px; }.validation-errors p { margin: 5px 0 0; color: #b9877f; font-size: 9px; line-height: 1.45; }.settings-actions { display: flex; align-items: center; gap: 7px; margin: 11px 0 18px; padding-top: 2px; }.settings-actions>span { flex: 1; }.settings-action { padding: 6px 10px; font-size: 10.5px; }.developer-settings { margin-top: 8px; padding: 13px 0 0; border-top: 1px solid #1b3028; }.developer-settings summary { cursor: pointer; color: #849991; font-size: 11px; }.developer-settings p { margin: 12px 0; color: #657a72; font-size: 10.5px; }.path-field { display: flex; gap: 8px; }.path-field input { flex: 1; }.upgrade-card { min-height: 300px; display: flex; flex-direction: column; align-items: center; justify-content: center; text-align: center; }.upgrade-card h2 { margin-bottom: 8px; font-size: 16px; }.upgrade-card p { max-width: 440px; color: #7c9088; font-size: 11px; }.upgrade-icon { display: grid; place-items: center; width: 48px; height: 48px; margin-bottom: 14px; border-radius: 50%; color: #e0b565; background: #2a2214; }.muted { color: #5f746c !important; }
  .modal-backdrop { position: fixed; z-index: 20; inset: 0; display: grid; place-items: center; background: rgba(1,6,4,.72); backdrop-filter: blur(5px); }.modal { position: relative; width: min(500px,calc(100vw - 50px)); padding: 25px; border: 1px solid #2b4138; border-radius: 13px; background: #0d1b17; box-shadow: 0 30px 80px rgba(0,0,0,.45); }.modal h2 { margin: 6px 0; font-size: 20px; }.modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 22px; padding-top: 15px; border-top: 1px solid #1d3029; }.toast { position: fixed; z-index: 30; right: 22px; bottom: 21px; display: flex; gap: 18px; align-items: center; max-width: 420px; border-color: #315447; color: #bee0d3; background: #132820; box-shadow: 0 16px 50px rgba(0,0,0,.35); font-size: 10px; }.toast span { color: #688178; }.floating-error { position: fixed; right: 25px; bottom: 24px; padding: 10px 13px; border: 1px solid #51342f; border-radius: 8px; color: #dc897b; background: #201411; font-size: 9px; }.loading { display: grid; min-height: 260px; place-items: center; color: #60756d; font-size: 11px; }
  .modal-error { display: flex; flex-direction: column; gap: 4px; margin-top: 14px; padding: 10px 12px; border: 1px solid #57342e; border-radius: 7px; color: #e29183; background: #201411; font-size: 10px; line-height: 1.4; }.modal-error strong { font-size: 10.5px; }.modal-error span { color: #c28379; overflow-wrap: anywhere; }
  .access-shell { max-width: 1020px; margin: 0 auto; }.access-summary { display: grid; grid-template-columns: minmax(0,1fr) auto; gap: 16px 24px; align-items: start; margin-bottom: 14px; padding: 18px 19px; border: 1px solid #1d3029; border-radius: 11px; background: linear-gradient(145deg, rgba(17,33,27,.94), rgba(10,22,18,.96)); }.access-summary>div:first-child>span { color: #61776e; font-size: 9px; font-weight: 700; letter-spacing: .16em; }.access-summary h2 { margin: 6px 0 5px; font-size: 18px; font-weight: 580; }.access-summary p { margin: 0; color: #71867e; font-size: 10.5px; }.access-summary>.pill { margin-top: 1px; }.access-summary-stats { grid-column: 1 / -1; display: grid; grid-template-columns: repeat(3,1fr); gap: 14px; padding-top: 13px; border-top: 1px solid #1d3029; }.access-summary-stats div { display: flex; flex-direction: column; gap: 4px; }.access-summary-stats strong { font-size: 18px; font-weight: 550; }.access-summary-stats span { color: #5d736a; font-size: 8px; text-transform: uppercase; letter-spacing: .1em; }.acl-columns { display: grid; grid-template-columns: repeat(2,minmax(0,1fr)); gap: 13px; }.acl-card { min-width: 0; }.acl-card.allow-card { border-top-color: #2e6b53; }.acl-card.deny-card { border-top-color: #704039; }.acl-path { margin: 12px 0; overflow: hidden; color: #61776e; font: 9px ui-monospace, SFMono-Regular, Menlo, monospace; text-overflow: ellipsis; white-space: nowrap; }.acl-entry-list { border-top: 1px solid #1b2d26; }.acl-entry-list>div { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-height: 42px; border-bottom: 1px solid #1b2d26; }.acl-entry-list code { min-width: 0; overflow-wrap: anywhere; color: #b8d5c9; font-size: 10px; }.acl-entry-list button { flex: 0 0 auto; }.access-peer-panel { margin-top: 13px; }.access-peer-list { margin-top: 13px; border-top: 1px solid #1b2d26; }.access-peer-list>div { display: flex; align-items: center; justify-content: space-between; gap: 14px; padding: 9px 0; border-bottom: 1px solid #1b2d26; }.access-peer-list .peer-cell { min-width: 0; }.access-peer-list .danger { width: auto; flex: 0 0 auto; padding: 6px 10px; font-size: 9px; }.access-note { margin-top: 13px; padding: 11px 13px; border-left: 2px solid #316a54; color: #71867e; background: rgba(16,36,28,.48); font-size: 10px; line-height: 1.5; }.access-note strong { color: #9bcbb9; }.access-note code { color: #a4c8ba; }
  @media (max-width: 900px) { .app-shell { grid-template-columns: 190px 1fr; }.hero-grid,.dashboard-grid { grid-template-columns: 1fr; }.tun-card { min-height: 180px; }.stat-grid { grid-template-columns: repeat(2,1fr); }.lan-summary { grid-template-columns: 9px 1fr auto; }.lan-summary>span:nth-of-type(3),.lan-summary>span:nth-of-type(4) { display: none; }.diagnostic-metrics { grid-template-columns: repeat(3,1fr); }.transport-grid { grid-template-columns: 1fr; }.content { padding-left: 20px; padding-right: 20px; }.metrics.four { grid-template-columns: repeat(2,1fr); }.settings-layout { grid-template-columns: 156px 1fr; }.settings-nav { padding-right: 12px; }.settings-form { padding-left: 20px; } }
</style>
