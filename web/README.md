# shalt.dev

Vercel front door for Shalt Space. Apex is `shalt.dev`. The desk HTML is copied from `crates/shalt/src/ui.html` at build.

- `/` — product
- `/desk` — the shalt desk
- `/api/*` — stub org/health, or proxy when `SHALT_ENGINE_URL` is set

Play does not run in this app. A later slice runs role turns in Vercel Sandbox.

```bash
npm install
npm run dev
```
