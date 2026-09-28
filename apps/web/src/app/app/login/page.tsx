"use client";

import { useQueryClient } from "@tanstack/react-query";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useState } from "react";
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
    <form onSubmit={submit} className="w-full max-w-sm space-y-4 rounded-xl border border-border bg-surface p-8">
      <div>
        <h1 className="text-xl font-semibold">Sign in to ControlHub</h1>
        <p className="mt-1 text-sm text-muted">
          Local credentials are in <code className="font-mono text-xs">.controlhub/local-credentials.json</code>.
        </p>
      </div>
      <label className="block text-sm">
        <span className="text-muted">Email</span>
        <input
          type="email"
          autoComplete="username"
          required
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 outline-none focus:border-accent"
        />
      </label>
      <label className="block text-sm">
        <span className="text-muted">Password</span>
        <input
          type="password"
          autoComplete="current-password"
          required
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          className="mt-1 w-full rounded-md border border-border bg-bg px-3 py-2 outline-none focus:border-accent"
        />
      </label>
      {error ? (
        <p role="alert" className="text-sm text-error">
          {error}
        </p>
      ) : null}
      <button
        type="submit"
        disabled={busy}
        className="w-full rounded-md bg-accent py-2 font-medium text-bg hover:bg-accent/90 disabled:opacity-60"
      >
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
