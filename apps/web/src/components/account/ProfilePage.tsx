"use client";

import { useState, type FormEvent } from "react";
import { apiSentence } from "@/lib/api";
import { useSession } from "@/lib/session";
import { AccountShell } from "./AccountShell";
import { useUpdateAccount } from "./useAccount";

function ProfileForm({ initialName, email, since }: { initialName: string; email: string; since: string }) {
  const update = useUpdateAccount();
  const [name, setName] = useState(initialName);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    update.mutate({ display_name: name });
  };

  return (
    <form onSubmit={submit} className="card space-y-4 p-5 text-sm">
      <label className="block">
        <span className="label">Display name</span>
        <input
          value={name}
          onChange={(e) => {
            setName(e.target.value);
            update.reset();
          }}
          required
          maxLength={100}
          autoComplete="name"
          className="input mt-1.5 h-10 w-full max-w-md"
        />
        <span className="mt-1 block text-xs text-muted">Shown to other members of your projects.</span>
      </label>
      <label className="block">
        <span className="label">Email</span>
        <input value={email} readOnly className="input mt-1.5 h-10 w-full max-w-md opacity-70" />
        <span className="mt-1 block text-xs text-muted">Your sign-in address. It cannot be changed yet.</span>
      </label>
      <p className="text-xs text-muted">Member since {since ? new Date(since).toLocaleDateString() : "—"}.</p>
      {update.error ? (
        <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-error">
          {apiSentence(update.error, "Could not save.")}
        </p>
      ) : null}
      {update.isSuccess ? (
        <p role="status" className="text-ok">
          Saved.
        </p>
      ) : null}
      <button type="submit" disabled={update.isPending || !name.trim()} className="btn btn-primary h-9 px-4">
        Save
      </button>
    </form>
  );
}

export function ProfilePage() {
  const session = useSession();
  const user = session.data?.user;
  return (
    <AccountShell active="profile" title="Personal info" description="How you appear in karbird.">
      {user ? <ProfileForm key={user.id} initialName={user.display_name} email={user.email} since={user.created_at} /> : null}
    </AccountShell>
  );
}
