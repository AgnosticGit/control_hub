# Invite links, personal account and onboarding — design

Parts 2–4 of the RBAC/onboarding request (part 1: `2026-09-30-permissions-roles-design.md`).

## Invite links

- A member with `members.manage` creates an invite link for one role of the project (built-in or
  template). Same anti-escalation rules as assigning a role: the role's permissions must be a subset
  of the creator's, and only an owner may invite with the Owner role.
- Options: expiry 1, 7 or 30 days (default 7), optional use limit (1–1000; default unlimited).
- The secret (`chv_` + 256 random bits) is shown once; only its SHA-256 hash and a short prefix are
  stored. The link is `/app/invite#<secret>`: the fragment never reaches a server, logs or Referer.
- Preview (`POST /api/v1/invites/preview`, token in the body, no session needed) shows project,
  role, inviter and expiry. Accept (`POST /api/v1/invites/accept`) needs a browser session + CSRF.
- Accepting re-checks, in one transaction under a row lock: not revoked, not expired, uses left.
  It also re-checks that the creator still may grant the role (still a member with
  `members.manage` and the role's permissions); otherwise the link is dead (410).
- Accepting when already a member changes nothing (`already_member: true`); a use is not spent.
- Unknown links: 404; expired, revoked, used up or no longer valid: 410. Creation, revocation,
  acceptance and refusals are audited.
- Members with `members.manage` list active links (prefix, role, creator, expiry, uses) and revoke.
- Deleting a role template deletes its links (FK cascade).

## Personal account (`/app/account`)

- Overview: greeting, setup checklist (while onboarding is not finished), projects.
- Profile: display name (1–100 printable characters), email (read-only), member since.
- Security: change password (current password required, new one follows the sign-up rules; other
  sessions are signed out; wrong current passwords count towards the login throttle), list of
  active sessions (created/expires, current marked) with "Sign out" per session and
  "Sign out other sessions". No IP or user-agent is stored (no PII).
- Projects: every project with your role and environments; create a project (own new tenant,
  environment `production`, you are Owner; at most 20 owned projects); leave a project (the last
  owner cannot leave: 409); rename (needs `project.manage`, also in project settings).
- All account mutations need a browser session (API tokens stay read-only).

## Onboarding

- `users.onboarding_completed_at`; existing users are marked done by the migration.
- After sign-up the user lands on `/app/onboarding`: 1) Connect your system (the integration guide:
  ingestion token, snippets, live "receiving data" status), 2) Invite your team (invite link),
  3) All set. Every step has "Skip this step"; "Skip setup" ends it at once.
- Never blocking: nothing redirects to onboarding except the first landing after sign-up. While it
  is unfinished, the workspace sidebar and the account overview show "Finish setup".
- `PATCH /api/v1/account {onboarding_done}` marks it done (or restarts it).
- An invited user who signs up comes back to the invite page and joins; onboarding stays optional.
