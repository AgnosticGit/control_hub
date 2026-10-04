"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useSyncExternalStore } from "react";
import { Icon, Logo } from "@/components/ui/Icon";
import { api, ApiRequestError, isUnauthenticated, unwrap } from "@/lib/api";
import { currentInviteSecret, forgetInviteSecret, INVITE_PATH, rememberInviteSecret } from "@/lib/invite";

const noSubscribe = () => () => {};
const NEXT = encodeURIComponent(INVITE_PATH);

function problem(e: unknown): string {
  if (e instanceof ApiRequestError && e.status === 404) return "This invite link is not valid. Ask for a new one.";
  if (e instanceof ApiRequestError && e.status === 410) {
    return `${e.message.replace(/^gone: /, "").replace(/^./, (c) => c.toUpperCase())}. Ask for a new link.`;
  }
  return "Cannot reach the karbird API.";
}

function Card({ children }: { children: React.ReactNode }) {
  return (
    <main className="flex min-h-screen items-center justify-center p-6">
      <div className="card w-full max-w-md space-y-5 p-8 text-sm">{children}</div>
    </main>
  );
}

export default function InvitePage() {
  const router = useRouter();
  const qc = useQueryClient();
  // undefined while rendering on the server; null when there is no usable secret.
  const secret = useSyncExternalStore(noSubscribe, currentInviteSecret, () => undefined);

  useEffect(() => {
    if (secret) rememberInviteSecret(secret);
  }, [secret]);

  const session = useQuery({
    queryKey: ["session"],
    retry: false,
    queryFn: async () => unwrap(await api.GET("/api/v1/session")),
  });
  const signedOut = session.isError && isUnauthenticated(session.error);

  const preview = useQuery({
    queryKey: ["invite-preview", secret, session.data?.user.id ?? null],
    enabled: Boolean(secret) && !session.isPending,
    retry: false,
    queryFn: async () => unwrap(await api.POST("/api/v1/invites/preview", { body: { token: secret ?? "" } })),
  });

  const accept = useMutation({
    mutationFn: async () => unwrap(await api.POST("/api/v1/invites/accept", { body: { token: secret ?? "" } })),
    onSuccess: async (r) => {
      forgetInviteSecret();
      await qc.invalidateQueries({ queryKey: ["session"] });
      const project = encodeURIComponent(r.project_id);
      router.replace(r.environment_id ? `/app/${project}/${encodeURIComponent(r.environment_id)}` : "/app");
    },
  });

  if (secret === undefined || (secret && !preview.isError && (!preview.data || session.isPending))) {
    return (
      <Card>
        <p className="text-muted">Loading the invite…</p>
      </Card>
    );
  }

  if (!secret || preview.isError) {
    return (
      <Card>
        <div className="flex flex-col items-center text-center">
          <Logo className="h-11 w-11" />
          <h1 className="mt-4 text-xl font-semibold">Invite link</h1>
        </div>
        <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-error">
          {secret ? problem(preview.error) : "This invite link is incomplete. Open the full link you were sent."}
        </p>
        <Link href="/app" className="btn h-10 w-full">
          Go to karbird
        </Link>
      </Card>
    );
  }

  const p = preview.data;
  if (!p) return null;
  return (
    <Card>
      <div className="flex flex-col items-center text-center">
        <Logo className="h-11 w-11" />
        <h1 className="mt-4 text-xl font-semibold">Join {p.project_name}</h1>
        <p className="mt-2 text-muted">
          <span className="text-text">{p.invited_by}</span> invited you to join as <span className="text-text">{p.role_name}</span>.
        </p>
        <p className="mt-1 text-xs text-muted">The link expires {new Date(p.expires_at).toLocaleString()}.</p>
      </div>

      {accept.error ? (
        <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-error">
          {problem(accept.error)}
        </p>
      ) : null}

      {signedOut || !p.signed_in ? (
        <div className="space-y-2">
          <Link href={`/app/login?next=${NEXT}`} className="btn btn-primary h-10 w-full">
            Sign in to join
          </Link>
          <Link href={`/app/register?next=${NEXT}`} className="btn h-10 w-full">
            Create an account
          </Link>
        </div>
      ) : (
        <div className="space-y-2">
          {p.already_member ? <p className="rounded-lg border border-border bg-surface-2 px-3 py-2">You are already a member of this project.</p> : null}
          <button type="button" className="btn btn-primary h-10 w-full" disabled={accept.isPending} onClick={() => accept.mutate()}>
            <Icon name="arrowRight" className="h-4 w-4" />
            {p.already_member ? "Open project" : `Join ${p.project_name}`}
          </button>
          <p className="text-center text-xs text-muted">Signed in as {session.data?.user.email}.</p>
        </div>
      )}
    </Card>
  );
}
