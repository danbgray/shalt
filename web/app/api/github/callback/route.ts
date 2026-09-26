import { encodeSession, safeNext, sessionCookie } from "@/lib/session";
import { NextRequest } from "next/server";

export const dynamic = "force-dynamic";

export async function GET(req: NextRequest) {
  const origin = req.nextUrl.origin;
  const code = req.nextUrl.searchParams.get("code") || "";
  const stateRaw = req.nextUrl.searchParams.get("state") || "";
  let next = "/desk";
  try {
    const st = JSON.parse(Buffer.from(stateRaw, "base64url").toString("utf8"));
    next = safeNext(typeof st.next === "string" ? st.next : null, origin);
  } catch {
    /* keep /desk */
  }
  if (!code || !process.env.SHALT_SESSION_SECRET) {
    return Response.redirect(`${origin}/?err=github`);
  }
  const tokenRes = await fetch("https://github.com/login/oauth/access_token", {
    method: "POST",
    headers: { Accept: "application/json", "Content-Type": "application/json" },
    body: JSON.stringify({
      client_id: process.env.GITHUB_CLIENT_ID,
      client_secret: process.env.GITHUB_CLIENT_SECRET,
      code,
      redirect_uri: `${origin}/api/github/callback`,
    }),
  });
  const tokenJson = (await tokenRes.json()) as { access_token?: string; error?: string };
  const gh = tokenJson.access_token || "";
  if (!gh) {
    return Response.redirect(`${origin}/?err=github`);
  }
  const meRes = await fetch("https://api.github.com/user", {
    headers: { Authorization: `Bearer ${gh}`, "User-Agent": "shalt.dev", Accept: "application/vnd.github+json" },
  });
  const me = (await meRes.json()) as { login?: string; id?: number };
  const login = me.login || "";
  const id = me.id || 0;
  if (!login) {
    return Response.redirect(`${origin}/?err=github`);
  }
  const token = encodeSession({
    login,
    id,
    gh,
    exp: Math.floor(Date.now() / 1000) + 30 * 24 * 3600,
  });
  if (next.startsWith("http://127.0.0.1") || next.startsWith("http://localhost")) {
    const dest = `${next}${next.includes("?") ? "&" : "?"}token=${encodeURIComponent(token)}`;
    return Response.redirect(dest);
  }
  const res = Response.redirect(`${origin}${next.startsWith("/") ? next : "/desk"}`);
  res.headers.append("Set-Cookie", sessionCookie(token, origin));
  return res;
}
