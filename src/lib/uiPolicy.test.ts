import { describe, expect, it } from "vitest";
import type { ServiceStatus } from "./types";
import {
  disconnectConfirmation,
  lifecyclePresentation,
  shouldOpenOnboarding,
} from "./uiPolicy";

function service(overrides: Partial<ServiceStatus>): ServiceStatus {
  return {
    available: false,
    state: "unknown",
    enabled: false,
    loaded: false,
    running: false,
    ownership: "unknown",
    installation: "checking",
    can_migrate: false,
    registration: "not_registered",
    ...overrides,
  };
}

describe("onboarding policy", () => {
  it("opens for installation and permission problems only", () => {
    expect(shouldOpenOnboarding("stopped")).toBe(true);
    expect(shouldOpenOnboarding("permission_denied")).toBe(true);
    expect(shouldOpenOnboarding("healthy")).toBe(false);
    expect(shouldOpenOnboarding("degraded")).toBe(false);
    expect(shouldOpenOnboarding("incompatible")).toBe(false);
  });
});

describe("destructive peer actions", () => {
  it("names the peer and routing impact in the confirmation", () => {
    expect(disconnectConfirmation("office-gateway")).toBe(
      "Disconnect office-gateway? Existing traffic through this peer may be rerouted.",
    );
  });
});

describe("lifecycle control presentation", () => {
  it("offers controls without implying that a monitored node is unavailable", () => {
    expect(lifecyclePresentation(service({
      ownership: "external",
      installation: "standard",
    }))).toEqual({
      summary: "Management not enabled · Standard FIPS installation",
      action: "enable_existing",
    });
  });

  it("explains that development builds remain monitor-only", () => {
    expect(lifecyclePresentation(service({
      ownership: "external",
      installation: "standard",
      registration: "bundle_incomplete",
    }))).toEqual({
      summary: "Unavailable in development",
      action: "development",
    });
  });

  it("identifies ready controls for the standard managed installation", () => {
    expect(lifecyclePresentation(service({
      available: true,
      ownership: "app_managed",
      installation: "standard",
    })).summary).toBe("Ready · Standard FIPS installation");
  });

  it("offers repair instead of lifecycle controls for conflicting services", () => {
    expect(lifecyclePresentation(service({
      available: true,
      ownership: "conflict",
      installation: "conflict",
    }))).toEqual({
      summary: "Installation needs repair",
      action: "repair",
    });
  });
});
