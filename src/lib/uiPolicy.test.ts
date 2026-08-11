import { describe, expect, it } from "vitest";
import { disconnectConfirmation, shouldOpenOnboarding } from "./uiPolicy";

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
