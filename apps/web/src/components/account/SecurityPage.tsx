"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { api, ApiRequestError, unwrap, unwrapEmpty } from "@/lib/api";
import { useLogout } from "@/lib/session";
import { formText, MIN_PASSWORD_CHARS } from "@/lib/signup";
import { AccountShell } from "./AccountShell";
import { passwordChangeProblem } from "./password";

function errorText(e: unknown): string {
  if (e instanceof ApiRequestError && e.status === 429) return "Too many attempts. Try again later.";
  if (e instanceof ApiRequestError && e.status === 400) return `${e.message.replace(/^invalid request: /, "").replace(/^./, (c) => c.toUpperCase())}.`;
  return "Cannot reach the ControlHub API.";
}

function PasswordForm() {
  const qc = useQueryClient();
  const [problem, setProblem] = useState<string | null>(null);
  const change = useMutation({
    mutationFn: async (body: { current_password: string; new_password: string }) => unwrapEmpty(await api.POST("/api/v1/account/password", { body })),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["account-sessions"] }),
  });

  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const form = e.currentTarget;
    const data = new FormData(form);
    const current = formText(data, "current");
    const next = formText(data, "new");
    const p = passwordChangeProblem(current, next, formText(data, "confirm"));
    setProblem(p);
    if (p) return;
    change.mutate({ current_password: current, new_password: next }, { onSuccess: () => form.reset() });
  };

  return (
    <section aria-labelledby="password" className="card space-y-4 p-5 text-sm">
      <h2 id="password" className="text-base font-semibold">
        Password
      </h2>
      <form method="post" onSubmit={submit} className="max-w-md space-y-3" noValidate>
        <label className="block">
          <span className="label">Current password</span>
          <input name="current" type="password" autoComplete="current-password" required className="input mt-1.5 h-10 w-full" />
        </label>
        <div>
          <label className="block">
            <span className="label">New password</span>
            <input
              name="new"
              type="password"
              autoComplete="new-password"
              required
              minLength={MIN_PASSWORD_CHARS}
              aria-describedby="new-password-hint"
              className="input mt-1.5 h-10 w-full"
            />
          </label>
          <span id="new-password-hint" className="mt-1 block text-xs text-muted">
            At least {MIN_PASSWORD_CHARS} characters.
          </span>
        </div>
        <label className="block">
          <span className="label">Repeat new password</span>
          <input name="confirm" type="password" autoComplete="new-password" required className="input mt-1.5 h-10 w-full" />
        </label>
        {problem || change.error ? (
          <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-error">
            {problem ?? errorText(change.error)}
          </p>
        ) : null}
        {change.isSuccess && !problem ? (
          <p role="status" className="text-ok">
            Password changed. Your other sessions were signed out.
          </p>
        ) : null}
        <button type="submit" disabled={change.isPending} className="btn btn-primary h-9 px-4">
          Change password
        </button>
      </form>
    </section>
  );
}

function Sessions() {
  const qc = useQueryClient();
  const logout = useLogout();
  const sessions = useQuery({
    queryKey: ["account-sessions"],
    queryFn: async () => unwrap(await api.GET("/api/v1/account/sessions")),
  });
  const refresh = () => qc.invalidateQueries({ queryKey: ["account-sessions"] });
  const revoke = useMutation({
    mutationFn: async (id: string) => unwrapEmpty(await api.DELETE("/api/v1/account/sessions/{session_id}", { params: { path: { session_id: id } } })),
    onSettled: refresh,
  });
  const others = useMutation({
    mutationFn: async () => unwrap(await api.POST("/api/v1/account/sessions/revoke-others")),
    onSettled: refresh,
  });
  const list = sessions.data ?? [];
  const error = sessions.error ?? revoke.error ?? others.error;

  return (
    <section aria-labelledby="sessions" className="card overflow-hidden text-sm">
      <div className="flex flex-wrap items-center justify-between gap-2 px-5 py-4">
        <div>
          <h2 id="sessions" className="text-base font-semibold">
            Where you are signed in
          </h2>
          <p className="text-muted">Browser sessions last 12 hours. Sign out any you do not recognise.</p>
        </div>
        <button type="button" className="btn h-9 px-4" disabled={others.isPending || list.length < 2} onClick={() => others.mutate()}>
          Sign out all other sessions
        </button>
      </div>
      {error ? (
        <p role="alert" className="mx-5 mb-3 rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-error">
          {error instanceof ApiRequestError ? error.message : "Could not load sessions."}
        </p>
      ) : null}
      <table className="w-full text-left" aria-label="Active sessions">
        <thead className="table-head">
          <tr>
            <th className="px-5 py-2">Signed in</th>
            <th className="px-2">Expires</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {list.map((s) => {
            const started = new Date(s.created_at).toLocaleString();
            return (
              <tr key={s.id} className="border-t border-border">
                <td className="px-5 py-2.5">
                  {started}
                  {s.current ? <span className="badge ml-2 border-ok/50 bg-ok/10 text-ok">This browser</span> : null}
                </td>
                <td className="px-2 text-muted">{new Date(s.expires_at).toLocaleString()}</td>
                <td className="px-5 text-right">
                  {s.current ? (
                    <button type="button" className="btn btn-sm" onClick={() => void logout()}>
                      Sign out
                    </button>
                  ) : (
                    <button type="button" className="btn btn-sm btn-danger" aria-label={`Sign out the session from ${started}`} disabled={revoke.isPending} onClick={() => revoke.mutate(s.id)}>
                      Sign out
                    </button>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </section>
  );
}

export function SecurityPage() {
  return (
    <AccountShell active="security" title="Security" description="Your password and active sessions.">
      <PasswordForm />
      <Sessions />
    </AccountShell>
  );
}
