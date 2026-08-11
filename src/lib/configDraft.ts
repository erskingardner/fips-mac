import { parse, stringify } from "yaml";

type UnknownMap = Record<string, unknown>;

export interface GuidedAddress {
  transport: string;
  addr: string;
  raw: UnknownMap;
}

export interface GuidedPeer {
  npub: string;
  alias: string;
  viaNostr: boolean;
  connectPolicy: string;
  addresses: GuidedAddress[];
  raw: UnknownMap;
}

export interface GuidedDraft {
  persistent: boolean;
  leafOnly: boolean;
  logLevel: string;
  tunEnabled: boolean;
  tunName: string;
  tunMtu: number;
  dnsEnabled: boolean;
  dnsPort: number;
  nostrDiscovery: boolean;
  lanDiscovery: boolean;
  udpEnabled: boolean;
  udpBind: string;
  tcpEnabled: boolean;
  tcpBind: string;
  peers: GuidedPeer[];
}

function map(value: unknown): UnknownMap {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as UnknownMap)
    : {};
}

function bool(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

function text(value: unknown, fallback: string): string {
  return typeof value === "string" ? value : fallback;
}

function number(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

export function readGuidedDraft(yaml: string): GuidedDraft {
  const root = map(parse(yaml));
  const node = map(root.node);
  const identity = map(node.identity);
  const rendezvous = map(node.rendezvous);
  const nostr = map(rendezvous.nostr);
  const lan = map(rendezvous.lan);
  const tun = map(root.tun);
  const dns = map(root.dns);
  const transports = map(root.transports);
  const udp = map(transports.udp);
  const tcp = map(transports.tcp);
  const rawPeers = Array.isArray(root.peers) ? root.peers : [];

  return {
    persistent: bool(identity.persistent, false),
    leafOnly: bool(node.leaf_only, false),
    logLevel: text(node.log_level, "info"),
    tunEnabled: bool(tun.enabled, true),
    tunName: text(tun.name, "fips0"),
    tunMtu: number(tun.mtu, 1280),
    dnsEnabled: bool(dns.enabled, true),
    dnsPort: number(dns.port, 5354),
    nostrDiscovery: bool(nostr.enabled, false),
    lanDiscovery: bool(lan.enabled, false),
    udpEnabled: "udp" in transports,
    udpBind: text(udp.bind_addr, "0.0.0.0:2121"),
    tcpEnabled: "tcp" in transports,
    tcpBind: text(tcp.bind_addr, "0.0.0.0:8443"),
    peers: rawPeers.map((value) => {
      const peer = map(value);
      const rawAddresses = Array.isArray(peer.addresses) ? peer.addresses : [];
      return {
        npub: text(peer.npub, ""),
        alias: text(peer.alias, ""),
        viaNostr: bool(peer.via_nostr, false),
        connectPolicy: text(peer.connect_policy, "auto_connect"),
        addresses: rawAddresses.map((address) => {
          const raw = map(address);
          return {
            transport: text(raw.transport, "udp"),
            addr: text(raw.addr, ""),
            raw,
          };
        }),
        raw: peer,
      };
    }),
  };
}

export function writeGuidedDraft(yaml: string, draft: GuidedDraft): string {
  const root = map(parse(yaml));
  const node = map(root.node);
  const identity = map(node.identity);
  const rendezvous = map(node.rendezvous);
  const nostr = map(rendezvous.nostr);
  const lan = map(rendezvous.lan);
  const tun = map(root.tun);
  const dns = map(root.dns);
  const transports = map(root.transports);

  identity.persistent = draft.persistent;
  node.identity = identity;
  node.leaf_only = draft.leafOnly;
  node.log_level = draft.logLevel;
  nostr.enabled = draft.nostrDiscovery;
  lan.enabled = draft.lanDiscovery;
  rendezvous.nostr = nostr;
  rendezvous.lan = lan;
  node.rendezvous = rendezvous;
  root.node = node;

  tun.enabled = draft.tunEnabled;
  tun.name = draft.tunName;
  tun.mtu = draft.tunMtu;
  root.tun = tun;
  dns.enabled = draft.dnsEnabled;
  dns.port = draft.dnsPort;
  root.dns = dns;

  if (draft.udpEnabled) {
    transports.udp = { ...map(transports.udp), bind_addr: draft.udpBind };
  } else {
    delete transports.udp;
  }
  if (draft.tcpEnabled) {
    transports.tcp = { ...map(transports.tcp), bind_addr: draft.tcpBind };
  } else {
    delete transports.tcp;
  }
  root.transports = transports;

  root.peers = draft.peers.map((peer) => ({
    ...peer.raw,
    npub: peer.npub,
    ...(peer.alias ? { alias: peer.alias } : {}),
    via_nostr: peer.viaNostr,
    connect_policy: peer.connectPolicy,
    addresses: peer.addresses.map((address) => ({
      ...address.raw,
      transport: address.transport,
      addr: address.addr,
    })),
  }));

  return stringify(root, { lineWidth: 0 });
}

export function newGuidedPeer(): GuidedPeer {
  return {
    npub: "",
    alias: "",
    viaNostr: false,
    connectPolicy: "auto_connect",
    addresses: [{ transport: "udp", addr: "", raw: {} }],
    raw: {},
  };
}

export function formatDiffValue(value: unknown): string {
  if (value === undefined) return "not set";
  if (value === "<redacted:preserve>") return "secret preserved";
  if (typeof value === "string") return value;
  return JSON.stringify(value);
}
