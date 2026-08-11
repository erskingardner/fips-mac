import { describe, expect, it } from "vitest";
import { formatFipsVersion } from "./format";

describe("formatFipsVersion", () => {
  it("removes the rev label and limits the revision to six characters", () => {
    expect(formatFipsVersion("0.5.0-dev (rev a402529311)"))
      .toBe("FIPS 0.5.0-dev (a40252)");
  });

  it("does not duplicate an existing FIPS prefix", () => {
    expect(formatFipsVersion("FIPS 0.5.0-dev (abcdef1234)"))
      .toBe("FIPS 0.5.0-dev (abcdef)");
  });
});
