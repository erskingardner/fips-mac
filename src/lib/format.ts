export function formatFipsVersion(value: unknown): string {
  if (typeof value !== "string" || value.trim() === "") return "FIPS node";

  const version = value.trim().replace(/^FIPS\s+/i, "");
  const revision = version.match(/^(.*?)\s*\(\s*(?:rev\s+)?([0-9a-f]{6,})\s*\)$/i);
  if (!revision) return `FIPS ${version}`;

  return `FIPS ${revision[1].trim()} (${revision[2].slice(0, 6)})`;
}
