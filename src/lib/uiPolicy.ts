import type { ServiceStatus } from "./types";

export type LifecycleAction =
  | "controls"
  | "enable_existing"
  | "install"
  | "install_app"
  | "repair"
  | "development"
  | "unavailable";

export interface LifecyclePresentation {
  summary: string;
  action: LifecycleAction;
}

export function lifecyclePresentation(
  service: ServiceStatus,
): LifecyclePresentation {
  if (service.registration === "app_not_installed") {
    return {
      summary: "Move FIPS to Applications to continue",
      action: "install_app",
    };
  }
  if (service.registration === "bundle_incomplete") {
    return { summary: "Unavailable in development", action: "development" };
  }
  if (service.available) {
    if (service.ownership === "app_managed") {
      return {
        summary: "Ready · Standard FIPS installation",
        action: "controls",
      };
    }
    if (service.ownership === "conflict") {
      return { summary: "Installation needs repair", action: "repair" };
    }
  }
  if (service.ownership === "conflict") {
    return { summary: "Installation needs repair", action: "repair" };
  }
  if (service.ownership === "external") {
    return {
      summary: "Management not enabled · Standard FIPS installation",
      action: "enable_existing",
    };
  }
  if (
    service.ownership === "none" ||
    service.installation === "not_installed"
  ) {
    return { summary: "FIPS is not set up", action: "install" };
  }
  return { summary: "Temporarily unavailable", action: "unavailable" };
}

export function shouldOpenInstallationOnboarding(
  service: ServiceStatus,
): boolean {
  if (
    ["requires_approval", "app_not_installed"].includes(
      service.registration,
    )
  ) {
    return true;
  }
  if (service.ownership === "conflict") return true;
  if (service.ownership === "external")
    return service.registration !== "bundle_incomplete";
  return (
    service.ownership === "none" || service.installation === "not_installed"
  );
}

export function disconnectConfirmation(peerLabel: string): string {
  return `Disconnect ${peerLabel}? Existing traffic through this peer may be rerouted.`;
}
