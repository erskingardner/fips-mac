import type { Health } from "./types";

export function shouldOpenOnboarding(health: Health): boolean {
  return health === "stopped" || health === "permission_denied";
}

export function disconnectConfirmation(peerLabel: string): string {
  return `Disconnect ${peerLabel}? Existing traffic through this peer may be rerouted.`;
}
