import { createHmac, createHash, createCipheriv, createDecipheriv, randomBytes } from "node:crypto";

const COOKIE = "shalt_session";

function secret() {
  const s = process.env.SHALT_SESSION_SECRET || "";
  if (!s) return "";
  return createHash("sha256").update(s).digest();
}

export type Session = {
  login: string;
  id: number;
  gh: string;
  exp: number;
};

export function configured() {
  return Boolean(process.env.GITHUB_CLIENT_ID && process.env.GITHUB_CLIENT_SECRET && process.env.SHALT_SESSION_SECRET);
}

export function encodeSession(s: Session): string {
  const key = secret();
  const iv = randomBytes(12);
  const cipher = createCipheriv("aes-256-gcm", key, iv);
  const pt = Buffer.from(JSON.stringify(s), "utf8");
  const enc = Buffer.concat([cipher.update(pt), cipher.final()]);
  const tag = cipher.getAuthTag();
  const body = Buffer.concat([iv, tag, enc]).toString("base64url");
  const mac = createHmac("sha256", key).update(body).digest("base64url");
  return `${body}.${mac}`;
}

export function decodeSession(raw: string | undefined | null): Session | null {
  if (!raw || !secret().length) return null;
  const [body, mac] = raw.split(".");
  if (!body || !mac) return null;
  const key = secret();
  const expect = createHmac("sha256", key).update(body).digest("base64url");
  if (expect !== mac) return null;
  const buf = Buffer.from(body, "base64url");
  if (buf.length < 29) return null;
  const iv = buf.subarray(0, 12);
  const tag = buf.subarray(12, 28);
  const enc = buf.subarray(28);
  try {
    const decipher = createDecipheriv("aes-256-gcm", key, iv);
    decipher.setAuthTag(tag);
    const pt = Buffer.concat([decipher.update(enc), decipher.final()]);
    const s = JSON.parse(pt.toString("utf8")) as Session;
    if (!s.login || s.exp < Date.now() / 1000) return null;
    return s;
  } catch {
    return null;
  }
}

export function sessionFromRequest(req: Request): Session | null {
  const auth = req.headers.get("authorization") || "";
  if (auth.toLowerCase().startsWith("bearer ")) {
    return decodeSession(auth.slice(7).trim());
  }
  const cookie = req.headers.get("cookie") || "";
  const hit = cookie.split(";").map((p) => p.trim()).find((p) => p.startsWith(COOKIE + "="));
  if (!hit) return null;
  return decodeSession(decodeURIComponent(hit.slice(COOKIE.length + 1)));
}

export function sessionCookie(token: string, origin: string) {
  const secure = origin.startsWith("https:") ? "; Secure" : "";
  return `${COOKIE}=${token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000${secure}`;
}

export function safeNext(next: string | null, origin: string): string {
  if (!next) return "/desk";
  if (next.startsWith("/") && !next.startsWith("//")) return next;
  try {
    const u = new URL(next);
    if (u.hostname === "127.0.0.1" || u.hostname === "localhost") return next;
    if (u.origin === origin) return next;
  } catch {
    /* ignore */
  }
  return "/desk";
}
