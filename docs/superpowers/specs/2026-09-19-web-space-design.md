# Shalt Space — hosted multi-tenant desk

Status: draft (desk UX in this pass; hosting protocol next).
Date: 2026-09-19

## Product

Shalt Space is its own product. The CLI and `shalt ui` stay local (loopback, no login). The shareable, collaborative desk is a hosted Space — not the unsigned macOS `.app`, which Gatekeeper rejects after WhatsApp/AirDrop quarantine.

Rivlet is the company. The origin root is **`shalt.dev`**. Isolation and cookies are defined against that eTLD+1. `shalt.rivlet.ai` may 301 here.

## Tenants

A Space is one org: humans, projects, ledger, journal, board, keys, job queue. On disk it is a dedicated home (`/var/shalt/spaces/{id}/`), never a shared `~/.shalt`. Play for Space A cannot see Space B’s queue or keys. Each Space runs as its own OS user or container.

| Host | Role |
|---|---|
| `shalt.dev` | control plane (create/pick Space). No Space session cookie. Deployed on Vercel. |
| `{slug}.shalt.dev` | that Space’s desk. Host-only cookie. |
| `{slug}-files.shalt.dev` | mockups / tenant HTML. No session cookie. |

Play does **not** run as a Vercel Function. The desk and login do. A Play turn runs in a Vercel Sandbox (or `shalt connect` on a member machine). Local `shalt ui` is unchanged.

## Identity (v1)

**GitHub** is the door. Sign-in and repo connect are the same grant. Google/X can be added later; they do not replace GitHub for bringing a tree.

- OAuth at `shalt.dev` (`read:user`, `repo`). Session is a signed cookie (browser) or Bearer (CLI). GitHub access token is encrypted at rest in that session — never logged, never returned by `/api/me`.
- `shalt login` opens GitHub via shalt.dev and stores `~/.shalt/credentials.toml` (token never printed).
- `shalt onboard github.com/org/repo` clones with **local git** (SSH keys as usual) and wraps the tree. No hosted token required.
- Connecting a private repo on the host uses the GitHub grant. Cloning on a laptop uses the user's git.
- Local `shalt ui` does not require GitHub.

Invite-only Spaces remain available (share a link that still requires Google/X to bind a person). Public self-serve without an invite is later.

## Collaboration

Shared truth is spec, ledger, journal, board. Not live cursors on the magazine. Several humans on one Space. Play is still slot-limited per Space.

BYO agents: `shalt connect` dials **out** to `https://{slug}.apex` with a Space-scoped Bearer and claims jobs for that Space only. Local Ollama may sit on the user’s Tailscale mesh; the cloud never opens inbound to their GPU. Hosted API keys never leave the Space home; connector keys never leave the laptop.

## Desk (this pass)

- Project-plan journey names (Desk, Traveler, …) are links that open that storyboard, not the org home.
- Storyboards step inside the current journey (numbered beats, journey tabs, arrow keys even while the mockup iframe is focused).
- Edit on the plan opens a Ghost-style markdown / preview split.
- The waiting-on-you sheet can be dismissed; the floating **Waiting on you · Answer** bar cannot.

## Key decisions

1. **Own product, own eTLD+1** — sibling products on `rivlet.ai` must not share cookies or XSS fate.
2. **Google and X OAuth** — one-click team login; Bearer tokens remain for agents.
3. **Process-per-Space isolation** — tenants are not rows in one `~/.shalt`.
4. **Connectors dial out** — Tailscale is the user’s GPU mesh, not the host’s inbound path.
5. **Local ui unchanged** — `127.0.0.1`, no login.

## PR plan

1. **Desk navigation and plan editor** (this change) — `ui.html`, mockup inject. No hosting.
2. **`shalt host` + token auth** — non-loopback bind requires a Space token; cookie + Bearer; 401 never dumps keys.
3. **Apex OAuth (Google, X)** — create/pick Space; host-only session on `{slug}`.
4. **Per-Space process + files origin** — isolated homes, mockups on cookieless host.
5. **`shalt connect`** — claim jobs, run locally, post artifacts; Tailscale optional on the client.
