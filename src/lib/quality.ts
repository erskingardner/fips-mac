import type { MmpPeerMeasurement, MmpSnapshot, Peer } from "./types";

function validFraction(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= 1;
}

export function measuredLinkLosses(snapshot: MmpSnapshot): number[] {
  return snapshot.peers.flatMap((peer) => {
    const loss = peer.link_layer?.smoothed_loss;
    return peer.mode !== "minimal" && validFraction(loss) ? [loss] : [];
  });
}

export function averageSmoothedLinkLoss(snapshot: MmpSnapshot): number | null {
  const losses = measuredLinkLosses(snapshot);
  return losses.length > 0
    ? losses.reduce((total, loss) => total + loss, 0) / losses.length
    : null;
}

export function peerSmoothedLinkLoss(
  snapshot: MmpSnapshot,
  peer: Peer,
): number | null {
  const measurement = snapshot.peers.find((candidate) =>
    candidate.peer === peer.node_addr
    || (!!candidate.display_name && candidate.display_name === peer.display_name)
  );
  const loss = measurement?.link_layer?.smoothed_loss;
  return measurement?.mode !== "minimal" && validFraction(loss) ? loss : null;
}

export function formatPacketLoss(loss: number | null): string {
  if (loss === null) return "—";
  return Intl.NumberFormat("en", {
    style: "percent",
    minimumFractionDigits: 0,
    maximumFractionDigits: 2,
  }).format(loss);
}

export function packetLossBarWidth(loss: number | null): number {
  return loss === null ? 0 : Math.min(Math.max(loss * 100, 0), 100);
}

export function packetLossSparkPoints(losses: number[]): string {
  const recent = losses.filter(validFraction).slice(-15);
  const samples = [...Array<number>(15 - recent.length).fill(0), ...recent];
  const ceiling = Math.max(0.01, ...samples);
  return samples
    .map((sample, index) => {
      const x = (index / (samples.length - 1)) * 100;
      const y = 31 - (sample / ceiling) * 26;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
}
