import { MIN_PASSWORD_CHARS } from "@/lib/signup";

/** First problem with the change-password form, or null. The API checks again (and the email rule). */
export function passwordChangeProblem(current: string, next: string, confirm: string): string | null {
  if (!current) return "Enter your current password.";
  if ([...next].length < MIN_PASSWORD_CHARS) return `New password must be at least ${MIN_PASSWORD_CHARS} characters.`;
  if (next === current) return "The new password must differ from the current one.";
  if (next !== confirm) return "New passwords do not match.";
  return null;
}
