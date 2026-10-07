# ADR-0021: organisations, project assignments and external authentication

Status: Accepted

Karbird keeps the existing tenant identifier as the organisation identifier. A browser user must
hold both an organisation membership and an explicit project assignment before project telemetry
is visible. Project roles continue to apply to every environment in the project.

First registration creates only an account. A transactional setup request then creates the
organisation, owner membership, first project and its chosen environments. Organisation invitations
are bound to a normalized email, contain explicit project and role assignments, expire, and store
only a token hash. Resend receives the raw token only inside the invitation URL at delivery time.

Google authentication uses Authorization Code with PKCE and a one-time, ten-minute state. Google
verifies the ID-token signature through its tokeninfo service; Karbird additionally checks issuer,
audience, expiry and `email_verified`. A verified Google email links to an existing password account
instead of creating a duplicate.

Google and Resend credentials are server-side deployment secrets. They never enter a browser build,
telemetry, audit payload or API response.

