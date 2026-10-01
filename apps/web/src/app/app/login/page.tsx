"use client";

import { useQueryClient } from "@tanstack/react-query";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useState } from "react";
import { AuthFrame } from "@/components/auth/AuthFrame";
import { api, ApiRequestError, unwrap } from "@/lib/api";
import { useHydrated } from "@/lib/hydrated";
import { formText, safeNext } from "@/lib/signup";

function LoginForm() {
  const router = useRouter();
  const params = useSearchParams();
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
    setBusy(true);
    setError(null);
    try {
      const session = unwrap(await api.POST("/api/v1/session/login", { body: { email, password } }));
      qc.setQueryData(["session"], session);
      router.replace(safeNext(params.get("next")) ?? "/app");
    } catch (err) {
      if (err instanceof ApiRequestError && err.status === 429) setError("Too many attempts. Try again later.");
      else if (err instanceof ApiRequestError && err.status === 401) setError("Invalid email or password.");
      else setError("Cannot reach the ControlHub API.");
    } finally {
      setBusy(false);
    }
  }

  return (
    // method=post: a submit before hydration must never put the password into the URL.
    <form method="post" onSubmit={submit} className="w-full max-w-sm space-y-5">
      <div>
        <h1 className="text-[1.75rem] leading-tight font-bold tracking-[-0.02em]">Sign in to ControlHub</h1>
        <p className="mt-2 text-sm leading-relaxed text-muted">
          Locally, sign in with the owner account from <code className="font-mono text-xs text-text">.controlhub/owner.env</code>.
        </p>
      </div>
      <label className="block text-sm">
        <span className="font-medium">Email</span>
        <input name="email" type="email" autoComplete="username" required className="input mt-1.5 h-10 w-full" />
      </label>
      <label className="block text-sm">
        <span className="font-medium">Password</span>
        <input name="password" type="password" autoComplete="current-password" required className="input mt-1.5 h-10 w-full" />
      </label>
      {error ? (
        <p role="alert" className="rounded-md border border-error/40 bg-error/10 px-3 py-2 text-sm text-error">
          {error}
        </p>
      ) : null}
      <button type="submit" disabled={busy || !hydrated} className="btn btn-primary h-10 w-full text-sm">
        {busy ? "Signing in…" : "Sign in"}
      </button>
      <p className="text-sm text-muted">
        New to ControlHub?{" "}
        <Link
          href={safeNext(params.get("next")) ? `/app/register?next=${encodeURIComponent(params.get("next") ?? "")}` : "/app/register"}
          className="font-semibold text-text underline decoration-border-strong hover:decoration-text"
        >
          Create an account
        </Link>
      </p>
    </form>
  );
}

export default function LoginPage() {
  return (
    <AuthFrame>
      <Suspense>
        <LoginForm />
      </Suspense>
    </AuthFrame>
  );
}
