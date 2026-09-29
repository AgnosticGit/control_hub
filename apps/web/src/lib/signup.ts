// Client-side hints only; the API validates again and is the authority.

export const MIN_PASSWORD_CHARS = 12;

/** A text field of a submitted form ("" when missing or a file). */
export function formText(form: FormData, name: string): string {
  const v = form.get(name);
  return typeof v === "string" ? v : "";
}

/** First problem with the form, or null when it can be submitted. */
export function signupProblem(email: string, password: string, confirm: string): string | null {
  const e = email.trim().toLowerCase();
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(e)) return "Enter a valid email address.";
  if ([...password].length < MIN_PASSWORD_CHARS) return `Password must be at least ${MIN_PASSWORD_CHARS} characters.`;
  if (password.trim().toLowerCase() === e) return "Password must not be your email.";
  if (password !== confirm) return "Passwords do not match.";
  return null;
}

/** Where a new account lands: the integration guide of its first environment. */
export function afterSignupPath(projects: ReadonlyArray<{ id: string; environments: ReadonlyArray<{ id: string }> }>): string {
  const p = projects[0];
  const e = p?.environments[0];
  return p && e ? `/app/${encodeURIComponent(p.id)}/${encodeURIComponent(e.id)}/integration` : "/app";
}
