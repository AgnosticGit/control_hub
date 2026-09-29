"use client";

import { useQueryClient } from "@tanstack/react-query";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useState } from "react";
import { Logo } from "@/components/ui/Icon";
import { api, ApiRequestError, unwrap } from "@/lib/api";

function safeNext(next: string | null): string {
  // Only same-app relative paths; never absolute or protocol-relative URLs.
  return next && next.startsWith("/app") && !next.startsWith("//") ? next : "/app";
}

function LoginForm() {
  const router = useRouter();
  const params = useSearchParams();
  const qc = useQueryClient();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const session = unwrap(await api.POST("/api/v1/session/login", { body: { email, password } }));
      qc.setQueryData(["session"], session);
      router.replace(safeNext(params.get("next")));
    } catch (err) {
      if (err instanceof ApiRequestError && err.status === 429) setError("Too many attempts. Try again later.");
      else if (err instanceof ApiRequestError && err.status === 401) setError("Invalid email or password.");
      else setError("Cannot reach the ControlHub API.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="card w-full max-w-sm space-y-5 p-8">
      <div className="flex flex-col items-center text-center">
        <Logo className="h-11 w-11" />
        <h1 className="mt-4 text-xl font-semibold">Sign in to ControlHub</h1>
        <p className="mt-1 text-sm text-muted">
          Local credentials are in <code className="font-mono text-xs">.controlhub/local-credentials.json</code>.
        </p>
      </div>
      <label className="block text-sm">
        <span className="label">Email</span>
        <input
          type="email"
          autoComplete="username"
          required
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          className="input mt-1.5 h-10 w-full"
        />
      </label>
      <label className="block text-sm">
        <span className="label">Password</span>
        <input
          type="password"
          autoComplete="current-password"
          required
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          className="input mt-1.5 h-10 w-full"
        />
      </label>
      {error ? (
        <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-sm text-error">
          {error}
        </p>
      ) : null}
      <button type="submit" disabled={busy} className="btn btn-primary h-10 w-full text-sm">
        {busy ? "Signing in…" : "Sign in"}
      </button>
    </form>
  );
}

export default function LoginPage() {
  return (
    <main className="flex min-h-screen items-center justify-center p-6">
      <Suspense>
        <LoginForm />
      </Suspense>
    </main>
  );
}
