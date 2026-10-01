Scope: public landing (`/`, Persuade) and the entry + account screens of `/app` (login, register, project picker, onboarding, `/app/account/*`, Operate). The world replaces the shared tokens in `globals.css`, so the rest of `/app` inherits colour, type and component chrome; the map and timeline renderers only get recoloured.

Audience: on-call/SRE engineers, backend developers, small teams without SRE, developers with AI assistants. Job on the landing: understand in one viewport what ControlHub does and why it is different, then run it locally or open the app. Job in the account area: get in, pick a project, manage identity, security and projects without friction.

Proof: a synthetic, clearly labelled live demo on the landing (service map + scrubbable timeline with a fault), no invented customers, numbers or pricing.

User steer: "professional SaaS, not frivolous"; direction delegated to Impeccable after a safer re-roll.

## Direction contract

THESIS: ControlHub shows a distributed system the way a city shows its metro: services as stations on coloured lines, requests as trains moving between them, time as a timetable you can scrub. It refuses the category default of a navy dashboard with glowing cards and feature tiles.

OWN-WORLD: matte graphite ground, no gradients or glow; hairline rules; one neutral grotesk for UI and display (heavy weight, tight tracking for headlines), a mono for timestamps, IDs and commands. Colour belongs only to service lines and to status (ok / degraded / failing); primary actions are signage-style light fills with dark text. Line language everywhere: stations, interchanges, solid observed edges, dashed inferred edges.

STORY: the visitor sees their kind of system running, watches `orders` degrade, drags the timeline back to before the fault, and understands "live map + time machine + honest data + controlled change" before reading a paragraph; then runs it locally or opens the app.

FIRST VIEWPORT: left third: kicker, the existing headline, one-line lead, primary "Run it locally" and secondary "Open app". Right two thirds: the demo line map of the 7 demo services plus external clients and the payment provider, trains running, a timeline rail beneath it with a red fault segment and a draggable cursor; a small "Demo · synthetic data" label. Signature interaction: dragging the cursor replots the map deterministically (same position, same picture).

FORM: transit line diagram, safer re-roll, position 1 of the remaining conventional list; seed key 0c68a155.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
