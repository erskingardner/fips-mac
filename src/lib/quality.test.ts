import { describe, expect, it } from "vitest";
import type { MmpSnapshot } from "./types";
import {
  averageSmoothedLinkLoss,
  formatPacketLoss,
  measuredLinkLosses,
  packetLossBarWidth,
  packetLossSparkPoints,
  peerSmoothedLinkLoss,
} from "./quality";

const snapshot: MmpSnapshot = {
  peers: [
    {
      peer: "a1",
      display_name: "Measured peer",
      mode: "full",
      link_layer: { loss_rate: 0.8, smoothed_loss: 0.05 },
    },
    {
      peer: "b2",
      display_name: "Warming up",
      mode: "full",
      link_layer: { loss_rate: 0.2 },
    },
    {
      peer: "c3",
      display_name: "Minimal peer",
      mode: "minimal",
      link_layer: { smoothed_loss: 0 },
    },
  ],
  sessions: [],
};

describe("MMP packet-loss presentation", () => {
  it("uses initialized smoothed measurements and ignores raw or unavailable values", () => {
    expect(measuredLinkLosses(snapshot)).toEqual([0.05]);
    expect(averageSmoothedLinkLoss(snapshot)).toBe(0.05);
  });

  it("returns no aggregate when no link has completed an MMP measurement", () => {
    expect(averageSmoothedLinkLoss({ peers: [], sessions: [] })).toBeNull();
  });

  it("converts FIPS fractions into percentages", () => {
    expect(formatPacketLoss(0.05)).toBe("5%");
    expect(formatPacketLoss(0.0012)).toBe("0.12%");
    expect(formatPacketLoss(null)).toBe("—");
    expect(packetLossBarWidth(0.05)).toBe(5);
  });

  it("zero-fills a new packet-loss history from the left edge", () => {
    const points = packetLossSparkPoints([0.004]);
    expect(points.split(" ")[0]).toBe("0.0,31.0");
    expect(points.split(" ").slice(0, -1).every((point) => point.endsWith(",31.0"))).toBe(true);
  });

  it("matches per-peer measurements by node address", () => {
    expect(peerSmoothedLinkLoss(snapshot, { node_addr: "a1" })).toBe(0.05);
    expect(peerSmoothedLinkLoss(snapshot, { node_addr: "b2" })).toBeNull();
  });
});
