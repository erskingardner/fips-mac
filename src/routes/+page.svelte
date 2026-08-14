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
    ApplyResult,
    ApplyStatus,
    AppPreferences,
    ConfigSnapshot,
    InvokeError,
    MmpSnapshot,
    MonitorSnapshot,
    Peer,
    ProductPreviewStatus,
    ServiceStatus,
    Transport,
    ValidationResult,
  } from "$lib/types";
  import { disconnectConfirmation, lifecyclePresentation } from "$lib/uiPolicy";
  import {
    averageSmoothedLinkLoss,
    formatPacketLoss,
    measuredLinkLosses,
    packetLossBarWidth,
    packetLossSparkPoints,
    peerSmoothedLinkLoss,
  } from "$lib/quality";
  import { formatFipsVersion } from "$lib/format";
  import Icon from "$lib/Icon.svelte";

  type View = "overview" | "peers" | "transports" | "settings";
  type SettingsPage = "general" | "node" | "developer";
  type SettingsSection = "identity" | "network" | "discovery" | "transports" | "peers";
  type TimedSample = { at: number; value: number };

  const initialSnapshot: MonitorSnapshot = {
    preview: false,
    health: "stopped",
    detail: "Looking for the FIPS daemon…",
    socket_path: "/var/run/fips/control.sock",
    checked_at_ms: Date.now(),
    configuration_supported: false,
    service: {
      available: false,
      state: "unknown",
      enabled: false,
      loaded: false,
      running: false,
      detail: "Looking for the FIPS service controller…",
      ownership: "unknown",
      installation: "checking",
      can_migrate: false,
      registration: "not_registered",
    },
  };

  let snapshot = $state<MonitorSnapshot>(initialSnapshot);
  let activeView = $state<View>("overview");
  let settingsPage = $state<SettingsPage>("node");
  let peers = $state<Peer[]>([]);
  let transports = $state<Transport[]>([]);
  let mmp = $state<MmpSnapshot>({ peers: [], sessions: [] });
  let lossHistory = $state<TimedSample[]>([]);
  let sessionHistory = $state<TimedSample[]>([]);
  let qualityError = $state("");
  let detailLoading = $state(false);
  let detailError = $state("");
  let selectedPeer = $state<Peer | null>(null);
  let connectOpen = $state(false);
  let connectNpub = $state("");
  let connectAddress = $state("");
  let connectTransport = $state("udp");
  let actionBusy = $state(false);
  let serviceBusy = $state(false);
  let serviceTransition = $state("");
  let onboardingOpen = $state(false);
  let disableManagementOpen = $state(false);
  let installBusy = $state(false);
  let installMessage = $state("");
  let pendingInstallAction = $state<"install" | "existing">("install");
  let preferences = $state<AppPreferences>({
    show_dock_icon: true,
    open_dashboard_at_launch: true,
  });
  let toast = $state("");
  let toastSequence = $state(0);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let productPreview = $state<ProductPreviewStatus>({
    available: false,
    enabled: false,
    scenario: "managed_running",
    scenarios: [],
  });
  let previewBusy = $state(false);

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
  const guidedLanIssue = $derived(guided ? lanDiscoveryIssue(guided) : null);
  const online = $derived(snapshot.health === "healthy" || snapshot.health === "degraded");
  const lifecycle = $derived(lifecyclePresentation(snapshot.service));
  const directLinkLoss = $derived(averageSmoothedLinkLoss(mmp));
  const measuredLinkCount = $derived(measuredLinkLosses(mmp).length);
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

  function dismissToast() {
    if (toastTimer) window.clearTimeout(toastTimer);
    toastTimer = undefined;
    toast = "";
  }

  function showToast(message: string) {
    if (toastTimer) window.clearTimeout(toastTimer);
    toast = message;
    toastSequence += 1;
    toastTimer = window.setTimeout(() => {
      toast = "";
      toastTimer = undefined;
    }, 5_000);
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

  function appendTimedSample(history: TimedSample[], value: number, at: number): TimedSample[] {
    const recent = history.filter((sample) => sample.at >= at - 30_000 && sample.at !== at);
    return [...recent, { at, value }];
  }

  function ingestSnapshot(next: MonitorSnapshot) {
    snapshot = next;
    const nextStatus = (next.status ?? {}) as Record<string, unknown>;
    if (typeof nextStatus.session_count === "number") {
      sessionHistory = appendTimedSample(sessionHistory, nextStatus.session_count, next.checked_at_ms);
    }
  }

  function activeSessionSeries(): unknown[] {
    const daemonSeries = sparklines().active_sessions;
    return Array.isArray(daemonSeries) && daemonSeries.length > 0
      ? daemonSeries
      : sessionHistory.map((sample) => sample.value);
  }

  function lossSeries(): number[] {
    return lossHistory.map((sample) => sample.value);
  }

  async function refreshOverview() {
    if (!isTauri()) return;
    try {
      ingestSnapshot(await invoke<MonitorSnapshot>("get_snapshot"));
      developmentPath = snapshot.socket_path;
      await loadDetails();
      invoke("refresh_now").catch(() => {});
    } catch (error) {
      detailError = errorMessage(error);
    }
  }

  async function refreshProductPreview() {
    if (!isTauri()) return;
    try {
      productPreview = await invoke<ProductPreviewStatus>("get_product_preview");
      if (!productPreview.available && settingsPage === "developer") settingsPage = "general";
    } catch {
      productPreview = { ...productPreview, available: false, enabled: false };
      if (settingsPage === "developer") settingsPage = "general";
    }
  }

  async function setProductPreview(enabled: boolean, scenario = productPreview.scenario) {
    if (!isTauri() || !productPreview.available || previewBusy) return;
    const previous = productPreview;
    productPreview = { ...productPreview, enabled, scenario };
    previewBusy = true;
    try {
      productPreview = await invoke<ProductPreviewStatus>("set_product_preview", {
        enabled,
        scenario,
      });
      config = null;
      validation = null;
      configError = "";
      applyMessage = "";
      selectedPeer = null;
      onboardingOpen = false;
      showToast(enabled
        ? "Product Preview enabled. All node data and actions are simulated."
        : "Product Preview disabled. Showing the live local node.");
      void refreshOverview();
      if (activeView === "settings" && settingsPage === "node") void loadConfig(true);
    } catch (error) {
      productPreview = previous;
      showToast(errorMessage(error));
    } finally {
      previewBusy = false;
    }
  }

  async function copyNpub() {
    if (!isTauri()) return;
    try {
      await invoke("copy_node_npub");
      showToast("Node npub copied.");
    } catch (error) {
      showToast(errorMessage(error));
    }
  }

  async function copyMeshAddress() {
    if (!isTauri()) return;
    try {
      await invoke("copy_node_address");
      showToast("Mesh address copied.");
    } catch (error) {
      showToast(errorMessage(error));
    }
  }

  async function copyPeerNpub(peer: Peer) {
    if (!isTauri() || !peer.npub) return;
    try {
      await invoke("copy_peer_npub", { npub: peer.npub });
      showToast(`${peer.display_name || "Peer"} npub copied.`);
    } catch (error) {
      showToast(errorMessage(error));
    }
  }

  function peerAddress(peer: Peer): string | undefined {
    return peer.transport_addr ?? peer.ipv6_addr;
  }

  async function copyPeerAddress(peer: Peer) {
    const address = peerAddress(peer);
    if (!isTauri() || !address) return;
    try {
      await invoke("copy_peer_address", { address });
      showToast(`${peer.display_name || "Peer"} address copied.`);
    } catch (error) {
      showToast(errorMessage(error));
    }
  }

  async function setServiceRunning(running: boolean) {
    if (!isTauri() || !snapshot.service.available || serviceBusy) return;
    serviceBusy = true;
    serviceTransition = running ? "Starting…" : "Stopping…";
    try {
      const service = await invoke<ServiceStatus>("set_fips_service_running", { running });
      snapshot = {
        ...snapshot,
        service,
        ...(running
          ? {}
          : {
              health: "stopped" as const,
              detail: "FIPS is turned off. Use the service switch to start it.",
              status: undefined,
            }),
      };
      showToast(running ? "FIPS started." : "FIPS stopped.");
    } catch (error) {
      showToast(errorMessage(error));
    } finally {
      serviceBusy = false;
      serviceTransition = "";
      invoke("refresh_now").catch(() => {});
    }
  }

  async function restartService() {
    if (!isTauri() || !snapshot.service.available || !snapshot.service.running || serviceBusy) return;
    serviceBusy = true;
    serviceTransition = "Restarting…";
    try {
      const service = await invoke<ServiceStatus>("restart_fips_service");
      snapshot = { ...snapshot, service };
      showToast("FIPS restarted.");
    } catch (error) {
      showToast(errorMessage(error));
    } finally {
      serviceBusy = false;
      serviceTransition = "";
      invoke("refresh_now").catch(() => {});
    }
  }

  async function refreshInstallation() {
    if (!isTauri()) return;
    try {
      const service = await invoke<ServiceStatus>("get_node_installation");
      snapshot = { ...snapshot, service };
    } catch (error) {
      installMessage = errorMessage(error);
    }
  }

  async function installNode() {
    if (!isTauri() || installBusy) return;
    installBusy = true;
    pendingInstallAction = "install";
    installMessage = "Opening the standard FIPS installer…";
    try {
      const service = await invoke<ServiceStatus>("register_node_service", { migrate: false });
      snapshot = { ...snapshot, service };
      installMessage = "FIPS is installed in /usr/local and management is enabled.";
      onboardingOpen = false;
      pendingInstallAction = "install";
      showToast(installMessage);
      await refreshProductPreview();
    } catch (error) {
      installMessage = errorMessage(error);
      await refreshInstallation();
    } finally {
      installBusy = false;
      invoke("refresh_now").catch(() => {});
    }
  }

  async function useExistingNode() {
    if (!isTauri() || installBusy) return;
    installBusy = true;
    pendingInstallAction = "existing";
    installMessage = "Enabling management for the standard FIPS installation…";
    try {
      const service = await invoke<ServiceStatus>("use_existing_node");
      snapshot = { ...snapshot, service };
      onboardingOpen = false;
      pendingInstallAction = "install";
      showToast("Monitoring, lifecycle, and configuration controls are enabled.");
      await refreshProductPreview();
    } catch (error) {
      installMessage = errorMessage(error);
      await refreshInstallation();
    } finally {
      installBusy = false;
    }
  }

  async function enableExistingControls() {
    onboardingOpen = true;
    await useExistingNode();
  }

  async function repairNode() {
    if (!isTauri() || installBusy) return;
    installBusy = true;
    installMessage = "Repairing the local FIPS service…";
    try {
      const service = await invoke<ServiceStatus>("repair_node_service");
      snapshot = { ...snapshot, service };
      installMessage = "FIPS service repaired.";
      showToast(installMessage);
      await refreshProductPreview();
    } catch (error) {
      installMessage = errorMessage(error);
    } finally {
      installBusy = false;
    }
  }

  async function removeManagedNode() {
    if (!isTauri() || installBusy) return;
    disableManagementOpen = false;
    installBusy = true;
    installMessage = "Disabling app management…";
    try {
      const service = await invoke<ServiceStatus>("remove_node_service");
      snapshot = { ...snapshot, service };
      installMessage = "App management was disabled. The FIPS installation and node are unchanged.";
      showToast(installMessage);
      onboardingOpen = true;
    } catch (error) {
      installMessage = errorMessage(error);
      showToast(installMessage);
    } finally {
      installBusy = false;
    }
  }

  async function openBackgroundSettings() {
    if (!isTauri()) return;
    await invoke("open_background_settings");
  }

  async function savePreferences(next: Partial<AppPreferences>) {
    if (!isTauri()) return;
    const proposed = { ...preferences, ...next };
    try {
      preferences = await invoke<AppPreferences>("set_app_preferences", {
        showDockIcon: proposed.show_dock_icon,
        openDashboardAtLaunch: proposed.open_dashboard_at_launch,
      });
    } catch (error) {
      showToast(errorMessage(error));
    }
  }

  async function loadDetails() {
    if (!isTauri() || !online || activeView === "settings") return;
    detailLoading = true;
    detailError = "";
    const tasks: Promise<void>[] = [];
    if (activeView === "overview" || activeView === "peers") {
      tasks.push(invoke<{ peers?: Peer[] }>("get_peers")
        .then((result) => { peers = result.peers ?? []; })
        .catch((error) => { detailError = errorMessage(error); }));
      tasks.push(invoke<MmpSnapshot>("get_mmp")
        .then((result) => {
          mmp = { peers: result.peers ?? [], sessions: result.sessions ?? [] };
          qualityError = "";
          const loss = averageSmoothedLinkLoss(mmp);
          if (loss === null) lossHistory = [];
          else lossHistory = appendTimedSample(lossHistory, loss, Date.now());
        })
        .catch((error) => {
          mmp = { peers: [], sessions: [] };
          lossHistory = [];
          qualityError = errorMessage(error);
        }));
    }
    if (activeView === "overview" || activeView === "transports") {
      tasks.push(invoke<{ transports?: Transport[] }>("get_transports")
        .then((result) => { transports = result.transports ?? []; })
        .catch((error) => { detailError = errorMessage(error); }));
    }
    try {
      await Promise.all(tasks);
    } finally {
      detailLoading = false;
    }
  }

  async function selectView(view: View) {
    activeView = view;
    selectedPeer = null;
    if (view === "settings") await loadConfig();
    else await loadDetails();
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
      showToast("Connection requested.");
      await loadDetails();
    } catch (error) {
      showToast(errorMessage(error));
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
      showToast("Peer disconnected.");
      await loadDetails();
    } catch (error) {
      showToast(errorMessage(error));
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
    showToast(`UDP will listen on ${guided.udpBind} after you review and apply the draft.`);
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
          applyMessage = "The apply is still pending. FIPS will keep showing daemon health as it reconnects.";
        }
      } else {
        applyMessage = "Configuration saved. No runtime restart was needed.";
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
    if (!window.confirm("Restore the initial configuration imported or created when FIPS was installed? FIPS will restart.")) return;
    applyBusy = true;
    try {
      const result = await invoke<ApplyResult>("reset_config", { expectedRevision: config.revision });
      applyMessage = "Restoring the initial configuration and restarting FIPS…";
      const status = await waitForApply(result.apply_id);
      applyMessage = status?.state === "applied"
        ? "The configuration from when app management was first enabled is active again."
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
      showToast("Development socket updated.");
      config = null;
      await refreshOverview();
    } catch (error) {
      showToast(errorMessage(error));
    }
  }

  onMount(() => {
    if (!isTauri()) return;
    let snapshotUnlisten: UnlistenFn | undefined;
    let navigateUnlisten: UnlistenFn | undefined;
    let serviceErrorUnlisten: UnlistenFn | undefined;
    void listen<MonitorSnapshot>("monitor://snapshot", (event) => {
      ingestSnapshot(event.payload);
      developmentPath = snapshot.socket_path;
      if (document.visibilityState === "visible" && activeView !== "settings" && online) {
        void loadDetails();
      }
    }).then((unlisten) => (snapshotUnlisten = unlisten));
    void listen<string>("app://navigate", (event) => {
      if (event.payload === "onboarding") {
        onboardingOpen = true;
        void refreshInstallation();
      } else {
        void selectView(event.payload === "settings" ? "settings" : "overview");
      }
    }).then((unlisten) => (navigateUnlisten = unlisten));
    void listen<string>("service://error", (event) => {
      showToast(event.payload);
    }).then((unlisten) => (serviceErrorUnlisten = unlisten));
    void invoke<AppPreferences>("get_app_preferences").then((value) => (preferences = value));
    void refreshProductPreview();
    void refreshInstallation();
    void refreshOverview();
    return () => {
      snapshotUnlisten?.();
      navigateUnlisten?.();
      serviceErrorUnlisten?.();
      if (toastTimer) window.clearTimeout(toastTimer);
    };
  });
</script>

<svelte:head><title>FIPS</title></svelte:head>

<div class="app-shell">
  <aside class="sidebar">
    <div class="brand">
      <div class="brand-mark" aria-hidden="true"><Icon name="brand" size={34} strokeWidth={1.45} /></div>
      <div><strong>FIPS</strong></div>
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
        <h1>{activeView === "overview" ? "Network overview" : activeView === "peers" ? "Peers" : activeView === "transports" ? "Transport health" : "FIPS settings"}</h1>
      </div>
      <div class="header-actions">
        <span class="checked">Checked {new Date(snapshot.checked_at_ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })}</span>
        <button class="icon-button" aria-label="Refresh" title="Refresh" onclick={refreshOverview}><Icon name="refresh" size={17} /></button>
        {#if activeView === "peers"}<button class="primary small" onclick={() => (connectOpen = true)}>Connect peer</button>{/if}
      </div>
    </header>

    {#if productPreview.enabled || snapshot.preview}
      <section class="preview-banner" aria-label="Product Preview is enabled">
        <div class="preview-banner-mark">P</div>
        <div><strong>PRODUCT PREVIEW · FAKE DATA</strong><span>No FIPS service, configuration, or network settings will be changed.</span></div>
        <label><span>Scenario</span><select disabled={previewBusy} value={productPreview.scenario} onchange={(event) => setProductPreview(true, event.currentTarget.value)}>{#each productPreview.scenarios as scenario}<option value={scenario.id}>{scenario.label}</option>{/each}</select></label>
        <button disabled={previewBusy} onclick={() => setProductPreview(false)}>{previewBusy ? "Switching…" : "Use live node"}</button>
      </section>
    {/if}

    <div class:preview-active={productPreview.enabled || snapshot.preview} class="content">
      {#if snapshot.health !== "healthy"}
        <section class="health-banner {snapshot.health}">
          <div class="health-symbol">{snapshot.health === "permission_denied" ? "⌕" : snapshot.health === "stopped" ? "○" : "!"}</div>
          <div><strong>{healthLabel}</strong><p>{snapshot.detail}</p></div>
          {#if snapshot.health === "stopped"}
            {#if snapshot.service.available && !snapshot.service.running}
              <button disabled={serviceBusy} onclick={() => setServiceRunning(true)}>{serviceBusy ? serviceTransition : "Start FIPS"}</button>
            {:else}
              <button onclick={() => selectView("settings")}>Connection settings</button>
            {/if}
          {:else if snapshot.health === "permission_denied"}
            {#if snapshot.service.ownership === "external"}<span class="hint">Local access changes may require signing out and back in.</span>{:else}<button disabled={installBusy} onclick={repairNode}>Repair access</button>{/if}
          {/if}
        </section>
      {/if}

      {#if activeView === "overview"}
        <section class="node-summary">
          <div class="node-primary">
            <div class="node-glyph"><Icon name="node" size={42} strokeWidth={1.35} /></div>
            <div class="identity-copy">
              <div class="eyebrow-row"><span>LOCAL FIPS NODE</span><span class="status-label {snapshot.health}"><i></i>{healthLabel}</span></div>
              <div class="identity-value npub-value">
                <code title={text(status.npub)}>{text(status.npub, "FIPS node")}</code>
                {#if text(status.npub) !== "—"}
                  <button class="copy-button" aria-label="Copy node npub" title="Copy node npub" onclick={copyNpub}>
                    <Icon name="copy" size={14} />
                  </button>
                {/if}
              </div>
              <div class="mesh-address-row">
                <span>MESH ADDRESS</span>
                <div class="identity-value mesh-address-value">
                  <code title={text(status.ipv6_addr)}>{text(status.ipv6_addr)}</code>
                  {#if text(status.ipv6_addr) !== "—"}
                    <button class="copy-button" aria-label="Copy mesh address" title="Copy mesh address" onclick={copyMeshAddress}>
                      <Icon name="copy" size={13} />
                    </button>
                  {/if}
                </div>
              </div>
            </div>
          </div>
          <div class="lifecycle-block">
            <div>
              <span>LIFECYCLE</span>
              <strong title={snapshot.service.detail ?? ""}>{serviceBusy ? serviceTransition : lifecycle.summary}</strong>
            </div>
            <div class="service-actions">
              {#if lifecycle.action === "controls"}
                <label class="service-switch" title={snapshot.service.running ? "Turn FIPS off" : "Turn FIPS on"}>
                  <input
                    type="checkbox"
                    aria-label={snapshot.service.running ? "Stop FIPS" : "Start FIPS"}
                    checked={snapshot.service.running}
                    disabled={serviceBusy}
                    onchange={() => setServiceRunning(!snapshot.service.running)}
                  />
                  <i></i>
                </label>
                <button class="service-restart" disabled={!snapshot.service.running || serviceBusy} title="Restart FIPS" onclick={restartService}><Icon name="refresh" size={12} /> Restart</button>
              {:else if lifecycle.action === "enable_existing"}
                <button class="service-enable" disabled={installBusy} onclick={enableExistingControls}>{installBusy ? "Enabling…" : "Enable controls"}</button>
              {:else if lifecycle.action === "development"}
                <button class="service-enable" disabled title="Install the signed FIPS app to enable lifecycle controls.">Installed app required</button>
              {:else if lifecycle.action === "install"}
                <button class="service-enable" onclick={() => (onboardingOpen = true)}>Set up FIPS</button>
              {:else if lifecycle.action === "repair"}
                <button class="service-enable" disabled={installBusy} onclick={repairNode}>{installBusy ? "Repairing…" : "Repair"}</button>
              {/if}
            </div>
          </div>
        </section>

        <section class="fact-strip" aria-label="Node operating facts">
          <div><span>UPTIME</span><strong>{humanDuration(status.uptime_secs)}</strong></div>
          <div><span>MESH ESTIMATE</span><strong>{compact(status.estimated_mesh_size)}</strong></div>
          <div><span>ROLE</span><strong>{status.is_root ? "Root" : status.is_leaf_only ? "Leaf" : "Mesh"}</strong></div>
          <div><span>IDENTITY</span><strong>{status.persistent ? "Persistent" : "Ephemeral"}</strong></div>
          <div><span>TUN</span><strong><i class="mini-dot {text(status.tun_state).toLowerCase()}"></i>{text(status.tun_name)}</strong><small>{text(status.tun_state)} · MTU {number(status.effective_ipv6_mtu, 1280)}</small></div>
          <div><span>TRANSPORTS</span><strong>{number(status.transport_count)} live</strong></div>
        </section>

        <section class="overview-section activity-section">
          <div class="section-heading"><div><span>NETWORK ACTIVITY</span><h3>Current mesh activity</h3></div><span class="legend"><i></i> In <i></i> Out</span></div>
          <div class="activity-layout">
            <div class="activity-metrics">
              <div><span>PEERS</span><strong>{number(status.peer_count)}</strong><svg viewBox="0 0 100 34" preserveAspectRatio="none"><polyline points={sparkPoints(sparklines().peer_count)} /></svg></div>
              <div><span>ACTIVE SESSIONS</span><strong>{number(status.session_count)}</strong><svg viewBox="0 0 100 34" preserveAspectRatio="none"><polyline points={sparkPoints(activeSessionSeries())} /></svg></div>
              <div><span>MESH NODES</span><strong>{compact(status.estimated_mesh_size)}</strong><svg viewBox="0 0 100 34" preserveAspectRatio="none"><polyline points={sparkPoints(sparklines().mesh_size)} /></svg></div>
            </div>
            <div class="traffic-block">
              <div class="traffic-caption"><span>TRAFFIC & QUALITY</span><small>Last 30 seconds</small></div>
              <svg class="traffic-chart" viewBox="0 0 100 42" preserveAspectRatio="none">
                <line x1="0" y1="10" x2="100" y2="10"/><line x1="0" y1="24" x2="100" y2="24"/><line x1="0" y1="38" x2="100" y2="38"/>
                <polyline class="bytes-in" points={sparkPoints(sparklines().bytes_in)} />
                <polyline class="bytes-out" points={sparkPoints(sparklines().bytes_out)} />
              </svg>
              <div class="quality-row">
                <div><span>AVERAGE DIRECT-LINK LOSS</span><small>{directLinkLoss === null ? (qualityError ? "Quality data unavailable" : "Waiting for MMP samples") : `${measuredLinkCount} measured link${measuredLinkCount === 1 ? "" : "s"}`}</small></div>
                <svg class="loss-sparkline" viewBox="0 0 100 34" preserveAspectRatio="none" aria-hidden="true"><polyline points={packetLossSparkPoints(lossSeries())} /></svg>
                <strong>{formatPacketLoss(directLinkLoss)}</strong>
                <div class="quality-track" title={directLinkLoss === null ? "No initialized MMP measurement" : `${formatPacketLoss(directLinkLoss)} average smoothed loss across measured direct links`}><i style={`width:${packetLossBarWidth(directLinkLoss)}%`}></i></div>
              </div>
            </div>
          </div>
        </section>

        <section class="overview-section route-section">
          <div class="section-heading"><div><span>CURRENT ROUTES</span><h3>Peers</h3></div><button class="text-button" onclick={() => selectView("peers")}>View all →</button></div>
          <div class="route-list">
            {#if peers.length}
              {#each peers.slice(0, 4) as peer}
                <div class="compact-peer">
                  <span class="peer-avatar">{(peer.display_name || peer.transport_type || "P").slice(0, 1).toUpperCase()}</span>
                  <div class="compact-peer-identity">
                    <button class="peer-name" aria-label={`Open details for ${peer.display_name || shortId(peer.npub)}`} onclick={() => { selectedPeer = peer; activeView = "peers"; }}>{peer.display_name || shortId(peer.npub)}</button>
                    <div class="peer-value-line npub-line">
                      <code title={peer.npub ?? undefined}>{peer.npub ?? "—"}</code>
                      {#if peer.npub}<button class="inline-copy" aria-label={`Copy npub for ${peer.display_name || "peer"}`} title="Copy npub" onclick={() => copyPeerNpub(peer)}><Icon name="copy" size={11} /></button>{/if}
                    </div>
                  </div>
                  <div class="route-meta"><span>{peer.transport_type ?? "unknown"}</span><div class="peer-value-line address-line"><code>{peerAddress(peer) ?? "—"}</code>{#if peerAddress(peer)}<button class="inline-copy" aria-label={`Copy address for ${peer.display_name || "peer"}`} title="Copy address" onclick={() => copyPeerAddress(peer)}><Icon name="copy" size={10} /></button>{/if}</div></div>
                  <span class="route-state"><i class="live-dot"></i>{peer.connectivity ?? "Connected"}</span>
                </div>
              {/each}
            {:else}<div class="empty-mini">No peers yet.</div>{/if}
          </div>
        </section>
      {:else if activeView === "peers"}
        <section class="page-section table-panel">
          <div class="section-heading"><div><span>MESH LINKS</span><h3>{peers.length} peer{peers.length === 1 ? "" : "s"}</h3></div><p>Live connections currently participating in this node's routes.</p></div>
          {#if detailLoading}<div class="loading">Refreshing peers…</div>
          {:else if peers.length === 0}<div class="empty"><div class="empty-icon">◌</div><h3>No peers</h3><p>Connect a known peer or enable LAN/Nostr discovery in Settings.</p><button class="primary" onclick={() => (connectOpen = true)}>Connect peer</button></div>
          {:else}
            <div class="data-table peer-table">
              <div class="table-head"><span>PEER</span><span>TRANSPORT</span><span>RELATION</span><span>ADDRESS</span><span>STATE</span></div>
              {#each peers as peer}
                <div class="table-row" class:selected={selectedPeer?.npub === peer.npub}>
                  <div class="peer-cell"><i>{(peer.display_name || "P").slice(0, 1).toUpperCase()}</i><div class="peer-identity"><button class="peer-name" aria-label={`Open details for ${peer.display_name || shortId(peer.npub)}`} onclick={() => (selectedPeer = peer)}>{peer.display_name || shortId(peer.npub)}</button><div class="peer-value-line npub-line"><code title={peer.npub ?? undefined}>{peer.npub ?? "—"}</code>{#if peer.npub}<button class="inline-copy" aria-label={`Copy npub for ${peer.display_name || "peer"}`} title="Copy npub" onclick={() => copyPeerNpub(peer)}><Icon name="copy" size={11} /></button>{/if}</div></div></div>
                  <span><b class="transport-tag">{peer.transport_type ?? "—"}</b></span>
                  <span>{peer.is_parent ? "Parent" : peer.is_child ? "Child" : "Peer"}</span>
                  <div class="peer-value-line address-line"><code>{peerAddress(peer) ?? "—"}</code>{#if peerAddress(peer)}<button class="inline-copy" aria-label={`Copy address for ${peer.display_name || "peer"}`} title="Copy address" onclick={() => copyPeerAddress(peer)}><Icon name="copy" size={10} /></button>{/if}</div>
                  <span class="peer-state"><i class="live-dot"></i>{peer.connectivity ?? "Connected"}</span>
                </div>
              {/each}
            </div>
          {/if}
        </section>
        {#if selectedPeer}
          <aside class="detail-drawer">
            <button class="drawer-close" onclick={() => (selectedPeer = null)}>×</button>
            <div class="peer-avatar large">{(selectedPeer.display_name || "P").slice(0, 1).toUpperCase()}</div>
            <h2>{selectedPeer.display_name || "Mesh peer"}</h2>
            <div class="drawer-npub"><code>{selectedPeer.npub}</code><button disabled={!selectedPeer.npub} onclick={() => copyPeerNpub(selectedPeer!)}><Icon name="copy" size={13} />Copy npub</button></div>
            <dl><div><dt>Connectivity</dt><dd>{selectedPeer.connectivity ?? "—"}</dd></div><div><dt>Mesh IPv6</dt><dd>{selectedPeer.ipv6_addr ?? "—"}</dd></div><div><dt>Transport</dt><dd>{selectedPeer.transport_type ?? "—"}</dd></div><div><dt>Direction</dt><dd>{selectedPeer.direction ?? "—"}</dd></div><div><dt>Tree depth</dt><dd>{selectedPeer.tree_depth ?? "—"}</dd></div><div><dt>Smoothed loss</dt><dd>{formatPacketLoss(peerSmoothedLinkLoss(mmp, selectedPeer))}</dd></div></dl>
            <button class="danger" disabled={actionBusy} onclick={() => disconnect(selectedPeer!)}>Disconnect peer</button>
          </aside>
        {/if}
      {:else if activeView === "transports"}
        <section class="page-section transport-list">
          <div class="section-heading"><div><span>TRANSPORTS</span><h3>{transports.length} configured endpoint{transports.length === 1 ? "" : "s"}</h3></div><p>Listeners and privacy transports available to this node.</p></div>
          <div class="transport-list-head"><span>TRANSPORT</span><span>STATE</span><span>LOCAL ADDRESS</span><span>MTU</span><span>ENDPOINT</span></div>
          {#each transports as transport}
            <article class="transport-row">
              <div class="transport-icon"><Icon name={transport.type === "udp" ? "udp" : transport.type === "tcp" ? "tcp" : transport.type === "tor" ? "tor" : "link"} size={21} /></div>
              <div class="transport-name"><strong>{transport.name || transport.local_addr || `Transport ${transport.transport_id}`}</strong><small>{transport.type ?? "transport"}</small></div>
              <span class="transport-state {String(transport.state).toLowerCase() === 'running' ? 'healthy' : 'degraded'}"><i></i>{transport.state ?? "Unknown"}</span>
              <code>{transport.local_addr ?? "—"}</code>
              <span>{transport.mtu ?? "—"}</span>
              <code>{transport.onion_address ?? "—"}</code>
            </article>
          {:else}
            <div class="empty"><div class="empty-icon">⇄</div><h3>No transport data</h3><p>{online ? "The daemon has no configured transport instances." : "Transport details are available when FIPS is running."}</p></div>
          {/each}
        </section>
      {:else}
        <section class="settings-shell">
          <nav class="settings-pages" class:development={productPreview.available} aria-label="Settings sections">
            <button class:active={settingsPage === "general"} onclick={() => (settingsPage = "general")}><strong>General</strong><small>Mac app and installation</small></button>
            <button class:active={settingsPage === "node"} onclick={() => (settingsPage = "node")}><strong>Node</strong><small>Network configuration</small></button>
            {#if productPreview.available}<button class:active={settingsPage === "developer"} onclick={() => (settingsPage = "developer")}><strong>Developer</strong><small>Preview and source builds</small></button>{/if}
          </nav>

          {#if settingsPage === "general"}
          <section class="application-settings settings-page">
            <div class="form-title application-title">
              <div><span>GENERAL</span><h2>Mac app and installation</h2><p>Control how the app appears and which FIPS installation it manages. The node keeps running when this window closes or the app quits.</p></div>
              <span class="ownership-badge">{snapshot.service.ownership === "app_managed" ? "Standard install · Managed" : snapshot.service.ownership === "external" ? "Standard install · Monitoring" : snapshot.service.ownership === "conflict" ? "Needs repair" : "Not installed"}</span>
            </div>
            <div class="application-grid">
              <label class="toggle-row"><span><strong>Show in Dock and App Switcher</strong><small>Run FIPS like a normal Mac app while keeping its menu-bar icon.</small></span><input type="checkbox" checked={preferences.show_dock_icon} onchange={(event) => savePreferences({ show_dock_icon: event.currentTarget.checked })}/><i></i></label>
              <label class="toggle-row"><span><strong>Open dashboard at launch</strong><small>Show this window when FIPS starts instead of staying in the menu bar.</small></span><input type="checkbox" checked={preferences.open_dashboard_at_launch} onchange={(event) => savePreferences({ open_dashboard_at_launch: event.currentTarget.checked })}/><i></i></label>
            </div>
            <div class="installation-row">
              <div><strong>FIPS node installation</strong><small>{snapshot.service.detail ?? (snapshot.service.config_path ? `Configuration: ${snapshot.service.config_path}` : "Install and manage FIPS without using Terminal.")}</small></div>
              <div>
                {#if snapshot.service.registration === "bundle_incomplete"}<button class="settings-action" disabled>Management helper unavailable</button>
                {:else if snapshot.service.ownership === "none" || snapshot.service.installation === "not_installed"}<button class="primary settings-action" onclick={() => (onboardingOpen = true)}>Install FIPS</button>
                {:else if snapshot.service.ownership === "external"}<button class="settings-action" onclick={() => (onboardingOpen = true)}>{snapshot.service.available ? "Manage installation" : "Enable controls"}</button>
                {:else if snapshot.service.ownership === "conflict"}<button class="settings-action" disabled={installBusy} onclick={repairNode}>Repair</button>
                {:else}<button class="danger-text" disabled={installBusy} onclick={() => (disableManagementOpen = true)}>Disable app management</button>{/if}
              </div>
            </div>
            {#if installMessage}<p class="install-message settings-install-message" aria-live="polite">{installMessage}</p>{/if}
          </section>

          {:else if settingsPage === "node"}
          <div class="node-settings-heading"><span>NODE CONFIGURATION</span><p>Configure identity, networking, discovery, transports, and persistent peers.</p></div>
          {#if configLoading}<div class="panel loading">Loading daemon configuration…</div>
          {:else if configError && !config}
            <article class="panel upgrade-card"><div class="upgrade-icon">↑</div><h2>Enable management to edit configuration</h2><p>{configError}</p><p class="muted">FIPS keeps using the standard configuration at <code>/usr/local/etc/fips/fips.yaml</code>. Enabling management lets this app validate, edit, and safely restart that same node.</p>{#if snapshot.service.ownership === "external"}<button disabled={installBusy} onclick={enableExistingControls}>{installBusy ? "Enabling…" : "Enable management"}</button>{:else}<button onclick={() => loadConfig(true)}>Try again</button>{/if}</article>
          {:else if config && guided}
            <div class="settings-header">
              <div><span>ACTIVE SOURCE</span><strong>Standard FIPS configuration</strong><code>{config.managed_path}</code></div>
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
              <div class="yaml-panel settings-form"><div class="form-title"><span>ADVANCED YAML</span><h2>Complete macOS configuration</h2><p>Every daemon key is available here. Secret sentinels preserve existing values without revealing them.</p></div><textarea aria-label="FIPS YAML configuration" spellcheck="false" bind:value={draftYaml} oninput={syncYamlToGuided}></textarea><div class="editor-footer"><code>{draftYaml.length.toLocaleString()} / 131,072 bytes</code><span>node.control settings are fixed when this app manages the node</span></div></div>
            {/if}

            {#if draftError}<div class="inline-error">{draftError}</div>{/if}
            {#if configError}<div class="inline-error">{configError}</div>{/if}
            {#if applyMessage}<div class="apply-message">{applyMessage}</div>{/if}

            {#if validation}
              <section class="review-panel">
                <div class="review-head"><div><span>{validation.valid ? "REVIEW CHANGES" : "VALIDATION ERRORS"}</span><h2>{validation.valid ? `${validation.diff.length} semantic change${validation.diff.length === 1 ? "" : "s"}` : "Configuration needs attention"}</h2></div>{#if validation.activation}<span class="impact {validation.activation}">{validation.activation === "restart" ? "Daemon restart" : "No runtime change"}</span>{/if}</div>
                {#if validation.errors.length}<div class="validation-errors">{#each validation.errors as error}<div><code>{error.path}</code><p>{error.message}</p></div>{/each}</div>{/if}
                {#if validation.warnings.length}<div class="warnings">{#each validation.warnings as warning}<p>⚠ {warning}</p>{/each}</div>{/if}
                {#if validation.valid}<div class="diff-list">{#each validation.diff as change}<div><code>{change.path || "/"}</code><span><del>{formatDiffValue(change.before)}</del><b>→</b><ins>{formatDiffValue(change.after)}</ins></span></div>{:else}<p class="muted">Only formatting changed. The app-owned file will be updated without restarting FIPS.</p>{/each}</div>{/if}
              </section>
            {/if}

            <div class="settings-actions">
              <button class="danger-text" disabled={applyBusy} onclick={resetConfig}>Restore initial configuration</button>
              <span></span>
              <button class="settings-action" disabled={applyBusy} onclick={() => { draftYaml = config!.yaml; guided = readGuidedDraft(draftYaml); validation = null; }}>Discard changes</button>
              {#if validation?.valid}<button class="primary settings-action" disabled={applyBusy} onclick={applyDraft}>{applyBusy ? "Applying…" : "Apply configuration"}</button>{:else}<button class="primary settings-action" disabled={applyBusy || !!draftError} onclick={reviewConfig}>{validation ? "Validate again" : "Review changes"}</button>{/if}
            </div>
          {/if}

          {:else}
          <section class="developer-settings settings-page">
            <div class="form-title"><span>DEVELOPER</span><h2>Product Preview and socket override</h2><p>These tools appear only in <code>tauri dev</code>. Every app mode otherwise detects the FIPS installation already running on this Mac.</p></div>
            <div class="preview-settings">
              <label class="toggle-row compact" class:busy={previewBusy}><span><strong>Product Preview</strong><small>Exercise the complete app with simulated data and actions without touching a real FIPS node.</small></span><input type="checkbox" checked={productPreview.enabled} disabled={previewBusy} onchange={(event) => setProductPreview(event.currentTarget.checked)}/><i></i></label>
              <label class="field"><span>Preview scenario</span><select disabled={!productPreview.enabled || previewBusy} value={productPreview.scenario} onchange={(event) => setProductPreview(true, event.currentTarget.value)}>{#each productPreview.scenarios as scenario}<option value={scenario.id}>{scenario.label}</option>{/each}</select></label>
            </div>
            <div class="developer-connection"><div><strong>Control socket override</strong><p>Use this only when testing a nonstandard FIPS configuration. Socket location does not determine whether FIPS was built from source or installed from a release; normal detection probes every supported macOS location.</p></div><div class="path-field"><input disabled={productPreview.enabled} bind:value={developmentPath}/><button disabled={productPreview.enabled} onclick={changeSocketPath}>Connect</button></div></div>
          </section>
          {/if}
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

{#if onboardingOpen}
  <div class="modal-backdrop onboarding-backdrop" role="presentation">
    <section class="modal onboarding-modal" aria-labelledby="onboarding-title">
      <button class="drawer-close" aria-label="Close onboarding" onclick={() => (onboardingOpen = false)}>×</button>
      <div class="onboarding-mark"><Icon name="node" size={38} strokeWidth={1.3} /></div>
      {#if snapshot.service.registration === "requires_approval"}
        <span>ONE-TIME MACOS APPROVAL</span>
        <h2 id="onboarding-title">Allow FIPS in Background Items</h2>
        <p>macOS requires an administrator to approve FIPS’s management helper. The node itself remains the standard <code>com.fips.daemon</code> installation in <code>/usr/local</code>.</p>
        <ol class="approval-steps"><li>Open System Settings.</li><li>Under “Allow in the Background,” enable FIPS.</li><li>Return here and continue.</li></ol>
        <div class="modal-actions"><button onclick={openBackgroundSettings}>Open System Settings</button><button class="primary" disabled={installBusy} onclick={() => pendingInstallAction === "existing" ? useExistingNode() : installNode()}>{installBusy ? "Checking…" : "I’ve approved it"}</button></div>
      {:else if snapshot.service.registration === "bundle_incomplete"}
        <span>MONITOR-ONLY BUILD</span>
        <h2 id="onboarding-title">Management is not in this build</h2>
        <p><code>tauri dev</code> is monitor-only because it is not a signed application bundle. Use the fast local packaged build or a release build to test installation, lifecycle, and configuration management.</p>
        <div class="modal-actions"><button class="primary" onclick={() => (onboardingOpen = false)}>Continue monitoring</button></div>
      {:else if snapshot.service.ownership === "external" || snapshot.service.installation === "standard"}
        <span>STANDARD FIPS INSTALLATION FOUND</span>
        <h2 id="onboarding-title">Use the node already on this Mac</h2>
        <p>FIPS found <code>com.fips.daemon</code> and <code>/usr/local/etc/fips/fips.yaml</code>. Nothing will be moved or reinstalled. Enable the management helper to start, stop, restart, and edit this same node.</p>
        <div class="modal-actions"><button onclick={() => (onboardingOpen = false)}>Not now</button><button class="primary" disabled={installBusy} onclick={useExistingNode}>{installBusy ? "Enabling…" : "Enable management"}</button></div>
      {:else if snapshot.service.ownership === "conflict"}
        <span>INSTALLATION NEEDS ATTENTION</span>
        <h2 id="onboarding-title">Two FIPS services are active</h2>
        <p>Only one node may own the local sockets, ports, TUN interface, and DNS configuration. FIPS can stop the duplicate and restore the selected installation.</p>
        <div class="modal-actions"><button onclick={() => (onboardingOpen = false)}>Cancel</button><button class="primary" disabled={installBusy} onclick={repairNode}>{installBusy ? "Repairing…" : "Repair installation"}</button></div>
      {:else}
        <span>WELCOME TO FIPS</span>
        <h2 id="onboarding-title">Run a FIPS node on this Mac</h2>
        <p>FIPS will open the standard macOS installer and use the normal upstream layout: binaries in <code>/usr/local/bin</code>, configuration in <code>/usr/local/etc/fips</code>, and the <code>com.fips.daemon</code> LaunchDaemon.</p>
        <div class="install-summary"><div><Icon name="node" size={18}/><span><strong>Standard installer</strong><small>The same package and paths documented by FIPS.</small></span></div><div><Icon name="settings" size={18}/><span><strong>One installation</strong><small>No app-private copy or migration path.</small></span></div><div><Icon name="overview" size={18}/><span><strong>Managed here</strong><small>Monitor, configure, start, stop, and restart.</small></span></div></div>
        <div class="modal-actions"><button onclick={() => (onboardingOpen = false)}>Not now</button><button class="primary" disabled={installBusy} onclick={installNode}>{installBusy ? "Installer open…" : "Open FIPS Installer"}</button></div>
      {/if}
      {#if installMessage}<p class="install-message">{installMessage}</p>{/if}
    </section>
  </div>
{/if}

{#if disableManagementOpen}
  <div class="modal-backdrop" role="presentation" onclick={(event) => event.target === event.currentTarget && (disableManagementOpen = false)}>
    <div class="modal" role="alertdialog" aria-modal="true" aria-labelledby="disable-management-title">
      <span>DISABLE APP MANAGEMENT</span>
      <h2 id="disable-management-title">Keep FIPS running independently?</h2>
      <p>The standard FIPS installation, configuration, identity, and current running state will remain unchanged. This app will continue monitoring the node, but lifecycle and configuration controls will be unavailable until management is enabled again.</p>
      <div class="modal-actions"><button onclick={() => (disableManagementOpen = false)}>Cancel</button><button class="danger" style="width: auto" disabled={installBusy} onclick={removeManagedNode}>{installBusy ? "Disabling…" : "Disable management"}</button></div>
    </div>
  </div>
{/if}

{#if toast}{#key toastSequence}<button class="toast" aria-live="polite" onclick={dismissToast}>{toast}<span>×</span></button>{/key}{/if}

<style>
  :global(*) { box-sizing: border-box; }
  :global(html) { --mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", monospace; background: #07100e; color-scheme: dark; }
  :global(body) { margin: 0; min-width: 820px; min-height: 600px; overflow: hidden; font-family: -apple-system, BlinkMacSystemFont, "SF Pro Display", "Helvetica Neue", sans-serif; color: #eaf4ef; background: #07100e; -webkit-font-smoothing: antialiased; }
  :global(button), :global(input), :global(select), :global(textarea) { font: inherit; }
  :global(button) { color: inherit; }
  .app-shell { height: 100vh; display: grid; grid-template-columns: 218px 1fr; background: #08120f; }.app-shell code { font-family: var(--mono); font-variant-numeric: tabular-nums; }.checked,nav button em,.sidebar-node small,.activity-metrics strong,.quality-row>strong { font-family: var(--mono); font-variant-numeric: tabular-nums; }
  .sidebar { position: relative; display: flex; flex-direction: column; padding: 27px 17px 20px; border-right: 1px solid #1a2924; background: #060f0c; }
  .brand { display: flex; align-items: center; gap: 13px; padding: 0 9px 29px; }
  .brand > div:last-child { display: flex; flex-direction: column; line-height: 1.05; }
  .brand strong { color: #f6fcf9; font-size: 18px; letter-spacing: .07em; }
  .brand span { color: #799188; font-size: 12px; letter-spacing: .14em; text-transform: uppercase; }
  .brand-mark { width: 34px; height: 34px; color: #59e5b1; filter: drop-shadow(0 0 8px rgba(81,224,172,.18)); }
  nav { display: flex; flex-direction: column; gap: 2px; }
  nav button { display: flex; align-items: center; gap: 12px; width: 100%; padding: 10px 12px; border: 0; border-radius: 4px; color: #83958f; background: transparent; text-align: left; cursor: pointer; transition: .16s; }
  nav button:hover { color: #d8e9e2; background: #0d1a16; } nav button.active { color: #e9fff7; background: #101f1a; }
  nav button em { margin-left: auto; font-size: 10px; font-style: normal; color: #5f746c; }.nav-icon { display: grid; width: 18px; place-items: center; color: #6e827b; }.active .nav-icon { color: #51e0ac; }
  .sidebar-node { margin-top: auto; display: flex; gap: 10px; align-items: center; padding: 15px 10px 2px; border-top: 1px solid #1b2c26; }
  .sidebar-node div { display: flex; flex-direction: column; gap: 2px; }.sidebar-node strong { font-size: 12px; font-weight: 600; }.sidebar-node small { font-size: 10px; color: #62776f; }
  .status-dot,.mini-dot,.live-dot { display: inline-block; flex: 0 0 auto; width: 8px; height: 8px; border-radius: 50%; background: #67746f; box-shadow: 0 0 0 3px rgba(103,116,111,.12); }.status-dot.healthy,.live-dot,.mini-dot.running { background: #4bdda6; box-shadow: 0 0 0 3px rgba(75,221,166,.1), 0 0 12px rgba(75,221,166,.35); }.status-dot.degraded { background: #e4b85d; }.status-dot.permission_denied,.status-dot.incompatible { background: #e47869; }
  main { min-width: 0; position: relative; overflow: hidden; }
  .topbar { height: 88px; display: flex; align-items: center; justify-content: space-between; padding: 0 31px; border-bottom: 1px solid #1a2924; background: #08120f; }
  .topbar p,.form-title>span,.review-head>div>span,.modal>span,.settings-header>div>span { margin: 0 0 4px; font-size: 9px; line-height: 1; font-weight: 700; letter-spacing: .16em; color: #61776e; }
  h1,h2,h3,p { margin-top: 0; }.topbar h1 { margin: 0; font-size: 21px; font-weight: 570; letter-spacing: -.02em; }.header-actions { display: flex; align-items: center; gap: 10px; }.checked { color: #52665e; font-size: 10px; }
  button { border: 1px solid #293a34; border-radius: 5px; padding: 8px 13px; background: #111e1a; cursor: pointer; transition: border-color .16s, background .16s, transform .12s; } button:hover:not(:disabled) { border-color: #3c5a50; background: #172721; } button:active:not(:disabled) { transform: translateY(1px); } button:disabled { opacity: .45; cursor: default; }
  button.primary { border-color: #4adea6; color: #06110d; background: #52e3ad; font-weight: 650; } button.primary:hover:not(:disabled) { background: #6bedbc; border-color: #6bedbc; }.small { padding: 7px 11px; font-size: 11px; }.icon-button { display: grid; width: 32px; height: 32px; padding: 0; place-items: center; color: #88a097; }
  .preview-banner { display: flex; height: 52px; align-items: center; gap: 10px; padding: 0 30px; border-bottom: 1px solid #725c23; color: #f3d785; background: linear-gradient(90deg,#342910,#241f10); box-shadow: 0 8px 22px rgba(0,0,0,.14); }.preview-banner-mark { display: grid; width: 25px; height: 25px; place-items: center; border: 1px solid #a48634; border-radius: 6px; color: #171204; background: #e7c55d; font-size: 11px; font-weight: 800; }.preview-banner>div:nth-child(2) { display: flex; min-width: 260px; flex: 1; flex-direction: column; gap: 2px; }.preview-banner strong { font-size: 9px; letter-spacing: .12em; }.preview-banner>div span { color: #a99559; font-size: 9px; }.preview-banner label { display: flex; align-items: center; gap: 7px; }.preview-banner label>span { color: #9f8a4d; font-size: 8px; text-transform: uppercase; letter-spacing: .1em; }.preview-banner select { min-width: 205px; padding: 5px 27px 5px 8px; border: 1px solid #685728; border-radius: 6px; color: #e7d49c; background: #211b0c; font-size: 9px; }.preview-banner button { padding: 5px 9px; border-color: #685728; color: #d9c582; background: #29210d; font-size: 9px; }
  .content { position: relative; height: calc(100vh - 88px); padding: 26px 32px 40px; overflow: auto; }.content.preview-active { height: calc(100vh - 140px); }
  .health-banner { display: flex; align-items: center; gap: 13px; margin-bottom: 18px; padding: 12px 15px; border: 1px solid #59482c; border-radius: 9px; background: #1e1a11; }.health-banner.permission_denied,.health-banner.incompatible { border-color: #59322d; background: #201311; }.health-banner strong { font-size: 12px; }.health-banner p { margin: 2px 0 0; color: #a59476; font-size: 11px; }.health-banner button,.health-banner .hint { margin-left: auto; font-size: 10px; }.health-symbol { display: grid; place-items: center; width: 27px; height: 27px; border-radius: 50%; color: #e9bd67; background: #322a18; }
  .service-actions { display: flex; align-items: center; gap: 8px; }.service-switch { position: relative; display: block; width: 31px; height: 17px; cursor: pointer; }.service-switch input { position: absolute; opacity: 0; pointer-events: none; }.service-switch i { position: absolute; inset: 0; border-radius: 20px; background: #263630; transition: .2s; }.service-switch i::after { content: ""; position: absolute; top: 3px; left: 3px; width: 11px; height: 11px; border-radius: 50%; background: #778981; transition: .2s; }.service-switch input:checked+i { background: #2c765d; }.service-switch input:checked+i::after { left: 17px; background: #69e2b6; }.service-switch input:focus-visible+i { outline: 2px solid #58cda3; outline-offset: 2px; }.service-switch:has(input:disabled) { cursor: default; opacity: .45; }.service-restart,.service-enable { display: flex; align-items: center; gap: 5px; padding: 4px 7px; border-color: #263a32; color: #8da198; background: transparent; font-size: 9px; }.service-restart:hover:not(:disabled),.service-enable:hover:not(:disabled) { color: #cce4da; background: #13251f; }
  .identity-copy { min-width: 0; flex: 1; }.identity-value { display: flex; min-width: 0; align-items: center; gap: 8px; }.identity-value code { white-space: nowrap; }.npub-value code { color: #e7f3ee; font-size: clamp(12px,1.45vw,18px); font-weight: 560; line-height: 1.35; letter-spacing: -.015em; }.mesh-address-row { display: flex; align-items: center; gap: 12px; margin-top: 8px; }.mesh-address-row>span { flex: 0 0 auto; color: #536a61; font-size: 8px; font-weight: 700; letter-spacing: .12em; }.mesh-address-value code { color: #718980; font-size: clamp(9px,1vw,11px); line-height: 1.4; }.copy-button { display: grid; flex: 0 0 25px; width: 25px; height: 25px; padding: 0; place-items: center; border-color: transparent; color: #718980; background: transparent; }.copy-button:hover:not(:disabled) { color: #66ddb1; background: #13251f; }.node-glyph { display: grid; flex: 0 0 52px; height: 52px; place-items: center; color: #52dfa9; }
  .legend { color: #5d726a; font-size: 8px; }.legend i { display: inline-block; width: 7px; height: 2px; margin: 0 4px 2px 9px; background: #48dba6; }.legend i:nth-child(2) { background: #51766b; }
  .traffic-chart { width: 100%; height: 80px; margin: 13px 0 5px; overflow: visible; }.traffic-chart line { stroke: #1d3029; stroke-width: .5; }.traffic-chart .bytes-out { stroke: #52796d; }.quality-row { display: grid; grid-template-columns: minmax(145px,1fr) 76px auto minmax(70px,.8fr); align-items: center; gap: 12px; }.quality-row>div:first-child { display: flex; min-width: 0; flex-direction: column; gap: 3px; }.quality-row span { color: #586e65; font-size: 8px; font-weight: 700; letter-spacing: .12em; }.quality-row small { overflow: hidden; color: #536960; font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }.quality-row strong { min-width: 38px; font-size: 10px; text-align: right; }.loss-sparkline { width: 76px; height: 24px; overflow: visible; }.loss-sparkline polyline { fill: none; stroke: #e4b75d; stroke-width: 1.5; vector-effect: non-scaling-stroke; }.quality-track { height: 3px; border-radius: 4px; background: #1d3029; overflow: hidden; }.quality-track i { display: block; height: 100%; background: #e4b75d; }
  .text-button,.danger-text { padding: 0; border: 0; color: #5dcba3; background: transparent; font-size: 9px; }.compact-peer { display: grid; grid-template-columns: 31px minmax(360px,2.4fr) minmax(110px,.7fr) minmax(80px,.52fr); align-items: center; gap: 12px; width: 100%; min-width: 0; padding: 11px 2px; border-bottom: 1px solid #192b24; background: transparent; text-align: left; }.compact-peer:hover { background: #0c1a15; }.compact-peer-identity,.route-meta { display: flex; min-width: 0; flex-direction: column; gap: 4px; }.peer-name { width: fit-content; max-width: 100%; overflow: hidden; padding: 0; border: 0; border-radius: 0; color: #dbe9e3; background: transparent; font-size: 12px; font-weight: 620; text-align: left; text-overflow: ellipsis; white-space: nowrap; }.peer-name:hover:not(:disabled) { color: #67dfb2; background: transparent; }.peer-name:focus-visible { outline: 1px solid #3d8068; outline-offset: 3px; }.peer-value-line { display: flex; min-width: 0; align-items: center; gap: 4px; }.peer-value-line code { min-width: 0; color: #70877e; font-size: 9px; line-height: 1.35; white-space: nowrap; }.npub-line code { overflow: hidden; color: #7f978e; font-size: 9.5px; letter-spacing: -.02em; text-overflow: ellipsis; }.inline-copy { display: grid; flex: 0 0 22px; width: 22px; height: 22px; padding: 0; place-items: center; border-color: transparent; color: #647c73; background: transparent; }.inline-copy:hover:not(:disabled) { color: #65dfb0; background: #13251f; }.route-meta>span { color: #82978f; font-size: 8px; font-weight: 650; text-transform: uppercase; }.route-meta .address-line code,.address-line code { overflow: hidden; text-overflow: ellipsis; }.route-state { display: flex; align-items: center; justify-content: flex-end; gap: 7px; color: #71867e; font-size: 9px; }.peer-avatar,.peer-cell>i { display: grid; place-items: center; width: 30px; height: 30px; border-radius: 6px; color: #67dab0; background: #13271f; font-size: 10px; font-style: normal; font-weight: 700; }.peer-avatar.large { width: 50px; height: 50px; margin: 16px 0 12px; border-radius: 10px; font-size: 17px; }.empty-mini { padding: 25px 0; color: #60756d; text-align: center; font-size: 10px; }
  .table-panel { padding: 0; overflow-x: auto; }.table-panel>.section-heading { padding-bottom: 18px; }.data-table { border-top: 1px solid #1c2d27; }.peer-table { min-width: 700px; }.table-head,.table-row { display: grid; grid-template-columns: minmax(360px,2.8fr) minmax(54px,.42fr) minmax(52px,.4fr) minmax(100px,.75fr) minmax(76px,.58fr); gap: 10px; align-items: center; padding: 10px 2px; }.table-head { color: #52675f; font-size: 8px; font-weight: 700; letter-spacing: .12em; }.table-row { width: 100%; border: 0; border-bottom: 1px solid #182a23; border-radius: 0; color: #81968e; background: transparent; text-align: left; font-size: 9px; }.table-row:hover,.table-row.selected { background: #0d1e18; }.peer-cell { display: flex; align-items: center; gap: 10px; min-width: 0; }.peer-identity { display: flex; min-width: 0; flex-direction: column; gap: 4px; }.transport-tag { color: #5bd6a9; font-size: 8px; text-transform: uppercase; }.peer-state { display: flex; align-items: center; gap: 7px; white-space: nowrap; }
  .empty { min-height: 300px; display: flex; flex-direction: column; align-items: center; justify-content: center; color: #71867e; text-align: center; }.empty h3 { margin: 8px 0; color: #d7e8e1; font-size: 14px; }.empty p { max-width: 340px; font-size: 11px; }.empty-icon { display: grid; place-items: center; width: 52px; height: 52px; border: 1px solid #243a32; border-radius: 50%; color: #4bd6a3; background: #102119; font-size: 21px; }
  .detail-drawer { position: fixed; z-index: 5; top: 88px; right: 0; bottom: 0; width: 315px; padding: 24px; border-left: 1px solid #24372f; background: #0c1814; box-shadow: -20px 0 50px rgba(0,0,0,.28); }.drawer-close { position: absolute; top: 15px; right: 15px; border: 0; background: transparent; color: #71857e; font-size: 20px; }.detail-drawer h2 { margin: 0 0 7px; font-size: 17px; }.drawer-npub { padding: 9px 0 13px; border-bottom: 1px solid #1b2d26; }.drawer-npub code { display: block; overflow-wrap: anywhere; color: #80968d; font-size: 9px; line-height: 1.45; }.drawer-npub button { display: flex; align-items: center; gap: 6px; margin-top: 9px; padding: 5px 8px; border-color: #2b453b; color: #72dcb3; background: #10231c; font-size: 9px; }.detail-drawer dl { margin: 17px 0 25px; }.detail-drawer dl div { display: grid; grid-template-columns: 1fr 1.3fr; gap: 8px; padding: 9px 0; border-bottom: 1px solid #1b2d26; font-size: 10px; }.detail-drawer dt { color: #566b63; }.detail-drawer dd { margin: 0; overflow-wrap: anywhere; color: #a8bbb4; text-align: right; }.danger { width: 100%; border-color: #53342e; color: #e58576; background: #241512; }
  .page-section { width: 100%; }.section-heading { display: flex; align-items: flex-end; justify-content: space-between; gap: 24px; }.section-heading>div { display: flex; flex-direction: column; gap: 5px; }.section-heading>div>span,.eyebrow-row>span:first-child,.lifecycle-block span,.fact-strip span,.activity-metrics span,.traffic-caption>span,.transport-list-head { color: #5b7168; font-size: 8px; font-weight: 700; letter-spacing: .14em; }.section-heading h3 { margin: 0; font-size: 14px; font-weight: 560; }.section-heading>p { max-width: 390px; margin: 0; color: #60756d; font-size: 10px; line-height: 1.45; text-align: right; }
  .node-summary { display: flex; min-width: 0; flex-direction: column; padding: 4px 0 18px; border-bottom: 1px solid #1c3028; }.node-primary { display: flex; min-width: 0; width: 100%; align-items: center; gap: 18px; }.eyebrow-row { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-bottom: 10px; }.status-label { display: flex; align-items: center; gap: 6px; color: #91a39c; font-size: 9px; }.status-label i,.transport-state i { width: 6px; height: 6px; border-radius: 50%; background: #66766f; }.status-label.healthy i,.transport-state.healthy i { background: #4bdda6; }.status-label.degraded i,.status-label.stopped i,.transport-state.degraded i { background: #e4b85d; }.status-label.permission_denied i,.status-label.incompatible i { background: #e47869; }.lifecycle-block { display: flex; width: calc(100% - 70px); min-width: 0; align-items: center; justify-content: space-between; gap: 18px; margin: 18px 0 0 70px; padding-top: 13px; border-top: 1px solid #182a23; }.lifecycle-block>div:first-child { display: flex; flex-direction: row; align-items: baseline; gap: 12px; }.lifecycle-block strong { font-size: 10px; font-weight: 560; }
  .fact-strip { display: grid; grid-template-columns: repeat(6,minmax(0,1fr)); padding: 18px 0; border-bottom: 1px solid #1c3028; }.fact-strip>div { display: flex; min-width: 0; flex-direction: column; gap: 5px; padding: 0 16px; }.fact-strip>div:first-child { padding-left: 0; }.fact-strip>div+div { border-left: 1px solid #182a23; }.fact-strip strong { display: flex; align-items: center; gap: 7px; overflow: hidden; font-size: 12px; font-weight: 560; text-overflow: ellipsis; white-space: nowrap; }.fact-strip small { overflow: hidden; color: #536960; font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }.fact-strip>div:nth-child(1) strong,.fact-strip>div:nth-child(2) strong,.fact-strip>div:nth-child(5) strong,.fact-strip>div:nth-child(5) small,.fact-strip>div:nth-child(6) strong { font-family: var(--mono); font-variant-numeric: tabular-nums; }
  .overview-section { padding: 23px 0; border-bottom: 1px solid #1c3028; }.activity-layout { display: grid; grid-template-columns: minmax(0,.9fr) minmax(280px,1.1fr); gap: 28px; margin-top: 20px; }.activity-metrics { display: grid; grid-template-columns: repeat(3,1fr); }.activity-metrics>div { display: grid; grid-template-rows: auto auto 34px; gap: 6px; padding: 3px 18px; }.activity-metrics>div:first-child { padding-left: 0; }.activity-metrics>div+div { border-left: 1px solid #182a23; }.activity-metrics strong { font-size: 22px; font-weight: 520; }.activity-metrics svg { width: 100%; height: 34px; overflow: visible; }.activity-metrics polyline,.traffic-chart polyline { fill: none; stroke: #48dba6; stroke-width: 1.7; vector-effect: non-scaling-stroke; }.traffic-block { padding-left: 28px; border-left: 1px solid #1c3028; }.traffic-caption { display: flex; justify-content: space-between; }.traffic-caption small { color: #536960; font-size: 8px; }
  .route-list { margin-top: 14px; border-top: 1px solid #192b24; }
  .transport-list>.section-heading { padding-bottom: 20px; }.transport-list-head,.transport-row { display: grid; grid-template-columns: 36px minmax(150px,1.15fr) minmax(80px,.65fr) minmax(130px,1fr) minmax(48px,.35fr) minmax(140px,1.1fr); align-items: center; gap: 12px; }.transport-list-head { padding: 10px 2px; border-top: 1px solid #1c3028; border-bottom: 1px solid #1c3028; }.transport-list-head span:first-child { grid-column: 1 / 3; }.transport-row { min-height: 58px; padding: 9px 2px; border-bottom: 1px solid #192b24; }.transport-icon { display: grid; width: 30px; height: 30px; place-items: center; color: #54d9a8; }.transport-name { display: flex; min-width: 0; flex-direction: column; gap: 3px; }.transport-name strong,.transport-name small,.transport-row code { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }.transport-name strong { font-size: 11px; font-weight: 570; }.transport-name small { color: #586d65; font-size: 8px; text-transform: uppercase; }.transport-state { display: flex; align-items: center; gap: 7px; color: #81968e; font-size: 9px; }.transport-row code,.transport-row>span { color: #71867e; font-size: 9px; }
  .panel { border-top: 1px solid #1c3028; border-bottom: 1px solid #1c3028; }.settings-shell { max-width: 980px; margin: 0 auto; }.settings-pages { display: grid; grid-template-columns: repeat(2,1fr); margin: -8px 0 26px; border-bottom: 1px solid #1b3028; }.settings-pages.development { grid-template-columns: repeat(3,1fr); }.settings-pages button { display: flex; min-height: 56px; flex-direction: column; align-items: flex-start; gap: 4px; padding: 10px 4px 12px; border: 0; border-bottom: 2px solid transparent; border-radius: 0; background: transparent; }.settings-pages button:hover { background: transparent; }.settings-pages button.active { border-bottom-color: #50dca8; color: #e9fff7; background: transparent; }.settings-pages strong { font-size: 12px; font-weight: 590; }.settings-pages small { color: #5e736b; font-size: 9px; }.settings-page { padding-top: 2px; }.settings-header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 16px; padding: 0 0 15px; border-bottom: 1px solid #1b3028; }.settings-header>div:first-child { display: flex; min-width: 0; flex-direction: column; gap: 3px; }.settings-header strong { font-size: 13px; font-weight: 590; }.settings-header code { overflow: hidden; color: #60766d; font-size: 9.5px; text-overflow: ellipsis; white-space: nowrap; }.segmented { display: flex; flex: 0 0 auto; border-bottom: 1px solid #20332b; }.segmented button { padding: 7px 10px; border: 0; border-bottom: 2px solid transparent; border-radius: 0; background: transparent; font-size: 10.5px; }.segmented button.active { border-bottom-color: #50dca8; color: #dff8ee; background: transparent; }
  .application-settings { margin-bottom: 25px; padding-bottom: 22px; border-bottom: 1px solid #20342c; }.application-title { display: flex; align-items: flex-start; justify-content: space-between; }.ownership-badge { flex: 0 0 auto; margin-top: 4px; color: #78c9aa; font-size: 9px; font-weight: 650; }.application-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 0 24px; border-top: 1px solid #192b24; }.installation-row { display: flex; align-items: center; justify-content: space-between; min-height: 64px; border-bottom: 1px solid #192b24; }.installation-row>div:first-child { display: flex; min-width: 0; flex-direction: column; gap: 4px; }.installation-row strong { font-size: 12px; }.installation-row small { max-width: 610px; overflow: hidden; color: #6b8078; font-size: 10.5px; text-overflow: ellipsis; white-space: nowrap; }.node-settings-heading { display: flex; align-items: baseline; gap: 13px; margin-bottom: 18px; }.node-settings-heading>span { color: #61776e; font-size: 9px; font-weight: 700; letter-spacing: .16em; }.node-settings-heading p { margin: 0; color: #60756d; font-size: 10.5px; }
  .settings-layout { display: grid; grid-template-columns: 188px minmax(0,1fr); gap: 0; }.settings-nav { align-self: start; padding: 3px 22px 3px 0; border-right: 1px solid #1a2d26; }.settings-nav button { padding: 10px 4px; border-radius: 0; font-size: 12px; line-height: 1.25; }.settings-nav button:hover { background: transparent; }.settings-nav button.active { color: #59dfae; background: transparent; }.settings-nav button em { font-size: 10.5px; }.settings-form { min-width: 0; min-height: 392px; padding: 3px 0 28px 28px; }.form-title { margin-bottom: 14px; padding-bottom: 15px; border-bottom: 1px solid #1d3029; }.form-title h2 { margin: 5px 0 6px; font-size: 18px; font-weight: 580; letter-spacing: -.01em; }.form-title p,.modal>p { max-width: 620px; margin: 0; color: #71867e; font-size: 11.5px; line-height: 1.5; }.toggle-row { position: relative; display: flex; align-items: center; justify-content: space-between; min-height: 59px; padding: 11px 0; border-bottom: 1px solid #192b24; cursor: pointer; }.toggle-row>span { display: flex; flex-direction: column; gap: 4px; }.toggle-row strong { font-size: 12px; font-weight: 580; }.toggle-row small { color: #6b8078; font-size: 10.5px; line-height: 1.35; }.toggle-row input { position: absolute; opacity: 0; }.toggle-row>i { position: relative; flex: 0 0 auto; width: 31px; height: 17px; border-radius: 20px; background: #263630; transition: .2s; }.toggle-row>i::after { content: ""; position: absolute; top: 3px; left: 3px; width: 11px; height: 11px; border-radius: 50%; background: #778981; transition: .2s; }.toggle-row input:checked+i { background: #2c765d; }.toggle-row input:checked+i::after { left: 17px; background: #69e2b6; }.toggle-row input:focus-visible+i { outline: 2px solid #58cda3; outline-offset: 2px; }.toggle-row.busy { cursor: wait; opacity: .68; }.toggle-row.compact { min-height: 40px; border: 0; }.field { display: flex; flex-direction: column; gap: 6px; margin-top: 14px; }.field>span { color: #7a8f87; font-size: 10.5px; font-weight: 600; }.field input,.field select,.path-field input { width: 100%; height: 36px; padding: 0 10px; border: 1px solid #263a32; border-radius: 4px; outline: 0; color: #c8d9d2; background: #091511; font-size: 11.5px; }.field input:focus,.field select:focus,.path-field input:focus,textarea:focus { border-color: #378a6b; }.field-row { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }.info-box { margin-top: 18px; padding: 3px 0 3px 12px; border-left: 2px solid #2f6652; color: #789087; font-size: 10.5px; line-height: 1.5; }.inline-warning { display: flex; align-items: center; gap: 14px; margin: 10px 0; padding: 10px 0 10px 12px; border-left: 2px solid #d3a651; }.inline-warning>div { min-width: 0; flex: 1; }.inline-warning strong { color: #e3bd72; font-size: 10.5px; }.inline-warning p { margin: 3px 0 0; color: #9a855e; font-size: 10px; line-height: 1.4; }.inline-warning button { flex: 0 0 auto; padding: 6px 9px; color: #dfbc76; font-size: 9px; }.transport-setting { margin: 0; padding: 0 0 15px; border-bottom: 1px solid #1d3029; }.transport-setting + .transport-setting { margin-top: 2px; }
  .peer-form-title { display: flex; align-items: flex-start; justify-content: space-between; }.peer-editor { margin: 0; padding: 15px 0 18px; border-bottom: 1px solid #20362d; }.peer-editor-title { display: flex; justify-content: space-between; }.peer-editor-title strong { font-size: 11px; }.danger-text { color: #df7e70; }.empty.embedded { min-height: 180px; border-top: 1px dashed #294038; border-bottom: 1px dashed #294038; border-radius: 0; }
  .yaml-panel { padding-left: 0; }.yaml-panel textarea { width: 100%; min-height: 415px; resize: vertical; padding: 14px; border: 1px solid #233831; border-radius: 4px; outline: 0; color: #b9d8ca; background: #06100d; font: 10.5px/1.58 ui-monospace, SFMono-Regular, Menlo, monospace; tab-size: 2; }.editor-footer { display: flex; justify-content: space-between; margin-top: 8px; color: #60756d; font-size: 9px; }.inline-error,.apply-message { margin: 10px 0; padding: 11px 13px; border: 1px solid #57342e; border-radius: 4px; color: #de8b7d; background: #201411; font-size: 10.5px; }.apply-message { border-color: #2a4c3d; color: #84cbb0; background: #102019; }.review-panel { margin-top: 14px; padding: 17px 0; border-top: 1px solid #244036; border-bottom: 1px solid #244036; }.review-head h2 { margin: 5px 0 0; font-size: 15px; }.impact { color: #76d4b2; font-size: 9px; font-weight: 650; }.impact.restart { color: #dfb567; }.diff-list { margin-top: 14px; }.diff-list>div { display: grid; grid-template-columns: minmax(120px,.65fr) 1.4fr; gap: 15px; padding: 10px 0; border-top: 1px solid #1c3028; font-size: 9px; }.diff-list>div>span { display: grid; grid-template-columns: 1fr 15px 1fr; gap: 6px; min-width: 0; }.diff-list del,.diff-list ins { overflow: hidden; color: #967b76; text-decoration: none; text-overflow: ellipsis; }.diff-list ins { color: #7bb69f; }.diff-list b { color: #536960; text-align: center; }.warnings { color: #d2ae68; font-size: 9px; }.validation-errors { margin-top: 14px; }.validation-errors>div { padding: 10px 12px; border-left: 2px solid #54332e; color: #de8b7d; }.validation-errors code { color: #e29183; font-size: 9px; }.validation-errors p { margin: 5px 0 0; color: #b9877f; font-size: 9px; line-height: 1.45; }.settings-actions { display: flex; align-items: center; gap: 7px; margin: 11px 0 18px; padding-top: 2px; }.settings-actions>span { flex: 1; }.settings-action { padding: 6px 10px; font-size: 10.5px; }.developer-settings { padding-top: 2px; }.developer-settings p { margin: 12px 0; color: #657a72; font-size: 10.5px; }.developer-connection { display: grid; grid-template-columns: minmax(0,1fr) minmax(300px,.9fr); align-items: end; gap: 26px; padding-top: 18px; border-top: 1px solid #1b3028; }.developer-connection strong { font-size: 12px; }.developer-connection p { max-width: 520px; margin: 5px 0 0; line-height: 1.45; }.preview-settings { display: grid; grid-template-columns: 1.4fr 1fr; gap: 22px; margin: 12px 0 22px; padding: 12px 0 18px; border-bottom: 1px solid #1b3028; }.preview-settings .toggle-row { border: 0; }.preview-settings .field { margin: 0; }.path-field { display: flex; gap: 8px; }.path-field input { flex: 1; }.upgrade-card { min-height: 300px; display: flex; flex-direction: column; align-items: center; justify-content: center; text-align: center; }.upgrade-card h2 { margin-bottom: 8px; font-size: 16px; }.upgrade-card p { max-width: 440px; color: #7c9088; font-size: 11px; }.upgrade-icon { display: grid; place-items: center; width: 48px; height: 48px; margin-bottom: 14px; border-radius: 50%; color: #e0b565; background: #2a2214; }.muted { color: #5f746c !important; }
  .modal-backdrop { position: fixed; z-index: 20; inset: 0; display: grid; place-items: center; background: rgba(1,6,4,.72); backdrop-filter: blur(5px); }.modal { position: relative; width: min(500px,calc(100vw - 50px)); padding: 25px; border: 1px solid #2b4138; border-radius: 13px; background: #0d1b17; box-shadow: 0 30px 80px rgba(0,0,0,.45); }.modal h2 { margin: 6px 0; font-size: 20px; }.modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 22px; padding-top: 15px; border-top: 1px solid #1d3029; }.toast { position: fixed; z-index: 30; right: 22px; bottom: 21px; display: flex; gap: 18px; align-items: center; max-width: 420px; border-color: #315447; color: #bee0d3; background: #132820; box-shadow: 0 16px 50px rgba(0,0,0,.35); font-size: 10px; animation: toast-life 5s ease both; }.toast span { color: #688178; }@keyframes toast-life { 0% { opacity: 0; transform: translateY(5px); } 4%,90% { opacity: 1; transform: translateY(0); } 100% { opacity: 0; transform: translateY(3px); } }.floating-error { position: fixed; right: 25px; bottom: 24px; padding: 10px 13px; border: 1px solid #51342f; border-radius: 8px; color: #dc897b; background: #201411; font-size: 9px; }.loading { display: grid; min-height: 260px; place-items: center; color: #60756d; font-size: 11px; }
  .onboarding-backdrop { background: rgba(1,6,4,.83); }.onboarding-modal { width: min(610px,calc(100vw - 50px)); padding: 30px; }.onboarding-modal>span { color: #5fd7aa; font-size: 9px; font-weight: 750; letter-spacing: .18em; }.onboarding-modal h2 { margin: 8px 0; font-size: 23px; }.onboarding-modal>p { max-width: none; }.onboarding-modal code { color: #80aa9a; }.onboarding-mark { display: grid; width: 52px; height: 52px; margin-bottom: 20px; place-items: center; border: 1px solid #285041; border-radius: 14px; color: #59dba9; background: #10261e; }.install-summary { display: grid; grid-template-columns: repeat(3,1fr); gap: 12px; margin-top: 22px; }.install-summary>div { display: flex; align-items: flex-start; gap: 9px; padding-top: 12px; border-top: 1px solid #20362d; color: #55cfa1; }.install-summary span { display: flex; flex-direction: column; gap: 4px; }.install-summary strong { color: #dcebe5; font-size: 10.5px; }.install-summary small { color: #687e75; font-size: 9px; line-height: 1.4; }.approval-steps { margin: 20px 0 0; padding: 14px 14px 14px 34px; border-top: 1px solid #20362d; border-bottom: 1px solid #20362d; color: #94a79f; font-size: 10.5px; line-height: 1.8; }.install-message { margin: 13px 0 0 !important; color: #84cbb0 !important; font-size: 10px !important; }.settings-install-message { padding-top: 10px; border-top: 1px solid #192b24; }
  @media (max-width: 900px) { .app-shell { grid-template-columns: 190px 1fr; }.content { padding-left: 20px; padding-right: 20px; }.lifecycle-block { width: 100%; margin-left: 0; }.fact-strip { grid-template-columns: repeat(3,1fr); row-gap: 18px; }.fact-strip>div:nth-child(4) { padding-left: 0; border-left: 0; }.activity-layout { grid-template-columns: 1fr; }.traffic-block { padding: 20px 0 0; border-top: 1px solid #1c3028; border-left: 0; }.compact-peer { grid-template-columns: 31px minmax(360px,1fr) minmax(80px,.55fr); }.compact-peer .route-meta { display: none; }.peer-table .table-head,.peer-table .table-row { grid-template-columns: minmax(360px,1fr) minmax(54px,.42fr) minmax(76px,.58fr); }.peer-table .table-head>span:nth-child(3),.peer-table .table-head>span:nth-child(4),.peer-table .table-row>span:nth-child(3),.peer-table .table-row>.address-line { display: none; }.transport-list-head,.transport-row { grid-template-columns: 32px minmax(140px,1fr) minmax(75px,.6fr) minmax(120px,1fr); }.transport-list-head span:nth-child(4),.transport-list-head span:nth-child(5),.transport-row>span:nth-child(5),.transport-row>code:nth-child(6) { display: none; }.settings-layout { grid-template-columns: 156px 1fr; }.settings-nav { padding-right: 12px; }.settings-form { padding-left: 20px; }.developer-connection { grid-template-columns: 1fr; }.application-grid { grid-template-columns: 1fr; } }
</style>
