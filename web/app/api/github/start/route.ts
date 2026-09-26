import { configured, safeNext } from "@/lib/session";
import { NextRequest } from "next/server";

export const dynamic = "force-dynamic";

export function GET(req: NextRequest) {
  const origin = req.nextUrl.origin;
  const next = safeNext(req.nextUrl.searchParams.get("next"), origin);
  if (!configured()) {
    if (next.startsWith("http://127.0.0.1") || next.startsWith("http://localhost")) {
      return Response.redirect(`${next}${next.includes("?") ? "&" : "?"}token=not-configured`);
    }
    return new Response(
      `<!doctype html><meta charset="utf-8"><title>Connect GitHub</title>
<style>body{font:18px/1.45 Iowan Old Style,Palatino,serif;max-width:36rem;margin:18vh auto;padding:0 24px;background:#0f1011;color:#eeeef0}a{color:#5e6ad2}code{font:14px ui-monospace,Menlo,monospace}</style>
<h1>Connect GitHub</h1>
<p>Create a GitHub OAuth App at <a href="https://github.com/settings/developers">github.com/settings/developers</a>.</p>
<p>Homepage: <code>${origin}</code><br>Callback: <code>${origin}/api/github/callback</code></p>
<p>Set <code>GITHUB_CLIENT_ID</code>, <code>GITHUB_CLIENT_SECRET</code>, and <code>SHALT_SESSION_SECRET</code> on the Vercel project.</p>
<p>Until then, from a machine with git:</p>
<pre>shalt onboard github.com/org/repo</pre>`,
      { headers: { "Content-Type": "text/html; charset=utf-8" } },
    );
  }
  const state = Buffer.from(JSON.stringify({ next, t: Date.now() })).toString("base64url");
  const url = new URL("https://github.com/login/oauth/authorize");
  url.searchParams.set("client_id", process.env.GITHUB_CLIENT_ID || "");
  url.searchParams.set("redirect_uri", `${origin}/api/github/callback`);
  url.searchParams.set("scope", "read:user repo");
  url.searchParams.set("state", state);
  return Response.redirect(url.toString());
}
