import { describe, expect, it } from "vitest";
import {
  formatDiffValue,
  isLoopbackUdpBind,
  lanDiscoveryIssue,
  readGuidedDraft,
  writeGuidedDraft,
} from "./configDraft";

const sample = `node:
  identity:
    persistent: true
    nsec: <redacted:preserve>
  control:
    enabled: true
    socket_path: /var/run/fips/control.sock
tun:
  enabled: true
  name: fips0
  mtu: 1280
dns:
  enabled: true
  port: 5354
transports:
  udp:
    bind_addr: 0.0.0.0:2121
peers: []
`;

describe("shared configuration draft", () => {
  it("round-trips guided edits without dropping advanced or secret values", () => {
    const guided = readGuidedDraft(sample);
    guided.leafOnly = true;
    guided.dnsPort = 5454;
    const yaml = writeGuidedDraft(sample, guided);

    expect(yaml).toContain("leaf_only: true");
    expect(yaml).toContain("port: 5454");
    expect(yaml).toContain("socket_path: /var/run/fips/control.sock");
    expect(yaml).toContain("nsec: <redacted:preserve>");
  });

  it("keeps advanced transport keys while guided fields change", () => {
    const withAdvanced = sample.replace(
      "bind_addr: 0.0.0.0:2121",
      "bind_addr: 0.0.0.0:2121\n    outbound_only: true",
    );
    const guided = readGuidedDraft(withAdvanced);
    guided.udpBind = "127.0.0.1:2122";
    const yaml = writeGuidedDraft(withAdvanced, guided);
    expect(yaml).toContain("outbound_only: true");
    expect(yaml).toContain("127.0.0.1:2122");
  });

  it("never exposes preserved secrets in diff labels", () => {
    expect(formatDiffValue("<redacted:preserve>")).toBe("secret preserved");
  });

  it("detects IPv4, IPv6, and hostname loopback UDP binds", () => {
    expect(isLoopbackUdpBind("127.0.0.1:2121")).toBe(true);
    expect(isLoopbackUdpBind("[::1]:2121")).toBe(true);
    expect(isLoopbackUdpBind("localhost:2121")).toBe(true);
    expect(isLoopbackUdpBind("0.0.0.0:2121")).toBe(false);
    expect(isLoopbackUdpBind("192.168.1.10:2121")).toBe(false);
  });

  it("offers an explicit LAN-safe bind without silently changing exposure", () => {
    const guided = readGuidedDraft(sample);
    guided.lanDiscovery = true;
    guided.udpBind = "127.0.0.1:4242";

    expect(lanDiscoveryIssue(guided)).toEqual({
      kind: "loopback_only",
      message:
        "127.0.0.1:4242 only accepts traffic from this Mac. LAN peers can see the advertisement but cannot connect.",
      suggestedBind: "0.0.0.0:4242",
    });

    guided.udpEnabled = false;
    expect(lanDiscoveryIssue(guided)?.kind).toBe("missing_udp");
  });
});
