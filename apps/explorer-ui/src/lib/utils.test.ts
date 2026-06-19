import { describe, expect, test } from "bun:test";
import { formatNumber, pluralize, shortHash } from "./utils";

describe("format helpers", () => {
  test("shortHash preserves compact values", () => {
    expect(shortHash("abc123", 3, 3)).toBe("abc123");
  });

  test("shortHash truncates long hashes", () => {
    expect(shortHash("1234567890abcdef", 4, 4)).toBe("1234...cdef");
  });

  test("formatNumber applies separators", () => {
    expect(formatNumber(1234567)).toBe("1,234,567");
  });

  test("pluralize selects singular and plural labels", () => {
    expect(pluralize(1, "block")).toBe("1 block");
    expect(pluralize(2, "block")).toBe("2 blocks");
  });
});
