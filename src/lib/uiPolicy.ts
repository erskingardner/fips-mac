import type { Health, ServiceStatus } from "./types";

export type LifecycleAction =
  | "controls"
  | "enable_existing"
  | "install"
  | "repair"
  | "development"
  | "unavailable";

export interface LifecyclePresentation {
  summary: string;
  action: LifecycleAction;
}

export function lifecyclePresentation(service: ServiceStatus): LifecyclePresentation {
  if (service.available) {
    if (service.ownership === "app_managed") {
      return { summary: "Ready · Standard FIPS installation", action: "controls" };
    }
    if (service.ownership === "conflict") {
      return { summary: "Installation needs repair", action: "repair" };
    }
  }
  if (service.ownership === "conflict") {
    return { summary: "Installation needs repair", action: "repair" };
  }
  if (service.ownership === "external") {
    if (service.registration === "bundle_incomplete") {
      return { summary: "Unavailable in development", action: "development" };
    }
    return {
      summary: "Management not enabled · Standard FIPS installation",
      action: "enable_existing",
    };
  }
  if (service.ownership === "none" || service.installation === "not_installed") {
    return { summary: "FIPS is not set up", action: "install" };
  }
  return { summary: "Temporarily unavailable", action: "unavailable" };
}

export function shouldOpenOnboarding(health: Health): boolean {
  return health === "stopped" || health === "permission_denied";
}

export function disconnectConfirmation(peerLabel: string): string {
  return `Disconnect ${peerLabel}? Existing traffic through this peer may be rerouted.`;
}
