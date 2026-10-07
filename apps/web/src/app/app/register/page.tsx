"use client";

import { useQueryClient } from "@tanstack/react-query";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useState } from "react";
import { AuthFrame } from "@/components/auth/AuthFrame";
import { api, ApiRequestError, unwrap } from "@/lib/api";
import { useHydrated } from "@/lib/hydrated";
import { afterSignupPath, formText, MIN_PASSWORD_CHARS, safeNext, signupProblem } from "@/lib/signup";

function errorText(err: unknown): string {
  if (!(err instanceof ApiRequestError)) return "Cannot reach the karbird API.";
  switch (err.status) {
    case 409:
      return "An account with this email already exists. Sign in instead.";
    case 429:
      return "Too many sign-ups right now. Try again later.";
    case 403:
      return err.message.includes("disabled") ? "Sign-up is disabled on this server." : "Request refused.";
    case 400:
      return err.message.replace(/^invalid request: /, "").replace(/^./, (c) => c.toUpperCase()) + ".";
    default:
      return "Cannot reach the karbird API.";
  }
}

function RegisterForm() {
  const router = useRouter();
  const next = safeNext(useSearchParams().get("next"));
  const qc = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const hydrated = useHydrated();

  // Values are read from the form on submit, so anything typed before hydration is kept.
  async function submit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const form = new FormData(e.currentTarget);
    const email = formText(form, "email");
    const password = formText(form, "password");
    const problem = signupProblem(email, password, formText(form, "confirm"));
    if (problem) {
      setError(problem);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const session = unwrap(
        await api.POST("/api/v1/session/register", {
          body: {
            email,
            password,
            display_name: formText(form, "display_name") || null,
          },
        }),
      );
      qc.setQueryData(["session"], session);
      router.replace(afterSignupPath(next));
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <AuthFrame>
      {/* method=post: a submit before hydration must never put the password into the URL. */}
      <form method="post" onSubmit={submit} className="w-full max-w-sm space-y-5" noValidate>
        <div>
          <h1 className="text-[1.75rem] leading-tight font-bold tracking-[-0.02em]">Create your karbird account</h1>
          <p className="mt-2 text-sm leading-relaxed text-muted">After registration, create your organization, project and environments in a short setup step.</p>
        </div>
        <label className="block text-sm">
          <span className="font-medium">Email</span>
          <input name="email" type="email" autoComplete="username" required maxLength={254} className="input mt-1.5 h-10 w-full" />
        </label>
        <label className="block text-sm">
          <span className="font-medium">Your name (optional)</span>
          <input name="display_name" autoComplete="name" maxLength={100} className="input mt-1.5 h-10 w-full" />
        </label>
        <div className="text-sm">
          <label className="block">
            <span className="font-medium">Password</span>
            <input
              name="password"
              type="password"
              autoComplete="new-password"
              required
              minLength={MIN_PASSWORD_CHARS}
              className="input mt-1.5 h-10 w-full"
              aria-describedby="password-hint"
            />
          </label>
          <p id="password-hint" className="mt-1 text-xs text-muted">
            At least {MIN_PASSWORD_CHARS} characters. A passphrase works well.
          </p>
        </div>
        <label className="block text-sm">
          <span className="font-medium">Repeat password</span>
          <input name="confirm" type="password" autoComplete="new-password" required className="input mt-1.5 h-10 w-full" />
        </label>
        {error ? (
          <p role="alert" className="rounded-md border border-error/40 bg-error/10 px-3 py-2 text-sm text-error">
            {error}
          </p>
        ) : null}
        <button type="submit" disabled={busy || !hydrated} className="btn btn-primary h-10 w-full text-sm">
          {busy ? "Creating account…" : "Create account"}
        </button>
        <div className="flex items-center gap-3 text-xs text-muted" aria-hidden><span className="h-px flex-1 bg-border" />or<span className="h-px flex-1 bg-border" /></div>
        <a href={`/api/v1/auth/google/start?next=${encodeURIComponent(next ?? "/app")}`} className="btn h-10 w-full justify-center text-sm">
          Continue with Google
        </a>
        <p className="text-sm text-muted">
          Already have an account?{" "}
          <Link
            href={next ? `/app/login?next=${encodeURIComponent(next)}` : "/app/login"}
            className="font-semibold text-text underline decoration-border-strong hover:decoration-text"
          >
            Sign in
          </Link>
        </p>
      </form>
    </AuthFrame>
  );
}

export default function RegisterPage() {
  return (
    <Suspense>
      <RegisterForm />
    </Suspense>
  );
}
