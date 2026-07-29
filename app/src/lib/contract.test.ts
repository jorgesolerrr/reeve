import { describe, expect, it } from "vitest";

import type { ApiError, ApiErrorCode } from "../generated/types";

/**
 * The frontend end of enforcement layer 4.
 *
 * `generated/types.ts` is types only, so most of this test's work happens at
 * `tsc` time: if the Rust envelope changes shape and nobody regenerates, these
 * literals stop compiling and `pnpm build` fails. The runtime assertions pin
 * the two facts the UI actually depends on — that a code is the slash-grouped
 * string it branches on, and that `details` is discriminated by `kind`.
 */
const dirty: ApiError = {
  code: "git/dirty",
  message: "commit or stash the changes first",
  details: { kind: "dirtyFiles", files: ["docs/a.md"] },
};

describe("the generated error envelope", () => {
  it("carries codes as their grouped wire form", () => {
    const code: ApiErrorCode = dirty.code;
    expect(code).toBe("git/dirty");
  });

  it("discriminates details by kind", () => {
    expect(dirty.details?.kind).toBe("dirtyFiles");
  });
});
