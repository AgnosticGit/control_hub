import { describe, expect, it } from "vitest";
import { inviteLink, isInviteSecret, secretFromHash, usesLabel } from "./invite";

const SECRET = "chv_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_";

describe("invite links", () => {
  it("put the secret in the fragment, never in the path or query", () => {
    const link = inviteLink("https://hub.example", SECRET);
    expect(link).toBe(`https://hub.example/app/invite#${SECRET}`);
    const url = new URL(link);
    expect(url.pathname).toBe("/app/invite");
    expect(url.search).toBe("");
    expect(url.hash).toBe(`#${SECRET}`);
  });

  it("read the secret back from the hash", () => {
    expect(secretFromHash(`#${SECRET}`)).toBe(SECRET);
    expect(secretFromHash(`#${encodeURIComponent(SECRET)} `)).toBe(SECRET);
  });

  it("ignore anything that is not an invite secret", () => {
    for (const bad of ["", "#", "#%E0%A4%A", "#chv_short", "#cha_AbCdEfGhIjKlMnOpQrSt", "#chv_<script>alert(1)</script>xxxxxxxx", `#chv_${"a".repeat(200)}`]) {
      expect(secretFromHash(bad)).toBeNull();
    }
    expect(isInviteSecret(SECRET)).toBe(true);
    expect(isInviteSecret(null)).toBe(false);
    expect(isInviteSecret("chs_AbCdEfGhIjKlMnOpQrSt")).toBe(false);
  });
});

describe("usesLabel", () => {
  it("shows the limit when there is one", () => {
    expect(usesLabel(0, null)).toBe("0 uses");
    expect(usesLabel(1, undefined)).toBe("1 use");
    expect(usesLabel(2, 10)).toBe("2 of 10 uses");
    expect(usesLabel(0, 1)).toBe("0 of 1 use");
  });
});
