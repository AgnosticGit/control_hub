// Invite links carry their secret in the URL fragment (`/app/invite#chv_…`): browsers never send
// the fragment to a server, so it stays out of access logs and Referer headers.

export const INVITE_PATH = "/app/invite";
/** Keeps the secret while the visitor signs in or signs up (per tab, cleared after joining). */
export const INVITE_STORAGE_KEY = "karbird.invite";

const SECRET = /^(?:chv_|koi_)[A-Za-z0-9_-]{16,124}$/;

export function isOrganizationInvite(secret: string): boolean {
  return secret.startsWith("koi_");
}

export function inviteLink(origin: string, secret: string): string {
  return `${origin}${INVITE_PATH}#${secret}`;
}

/** The secret from a location hash (`#chv_…`), or null if it is not one. */
export function secretFromHash(hash: string): string | null {
  let s: string;
  try {
    s = decodeURIComponent(hash.replace(/^#/, "")).trim();
  } catch {
    return null;
  }
  return SECRET.test(s) ? s : null;
}

export function isInviteSecret(s: string | null | undefined): s is string {
  return typeof s === "string" && SECRET.test(s);
}

/** The secret from this page's hash, or the one remembered in this tab. Browser only. */
export function currentInviteSecret(): string | null {
  const fromHash = secretFromHash(window.location.hash);
  if (fromHash) return fromHash;
  try {
    const s = window.sessionStorage.getItem(INVITE_STORAGE_KEY);
    return isInviteSecret(s) ? s : null;
  } catch {
    return null;
  }
}

/** Remembers the secret for this tab and removes it from the address bar and history. */
export function rememberInviteSecret(secret: string) {
  try {
    window.sessionStorage.setItem(INVITE_STORAGE_KEY, secret);
  } catch {
    return;
  }
  if (window.location.hash) window.history.replaceState(null, "", INVITE_PATH);
}

export function forgetInviteSecret() {
  try {
    window.sessionStorage.removeItem(INVITE_STORAGE_KEY);
  } catch {
    // Nothing stored.
  }
}

export const EXPIRY_CHOICES = [
  { days: 1, label: "1 day" },
  { days: 7, label: "7 days" },
  { days: 30, label: "30 days" },
] as const;

/** "5 of 10 uses" / "3 uses" for the invite list. */
export function usesLabel(uses: number, maxUses: number | null | undefined): string {
  if (maxUses == null) return `${uses} ${uses === 1 ? "use" : "uses"}`;
  return `${uses} of ${maxUses} ${maxUses === 1 ? "use" : "uses"}`;
}
