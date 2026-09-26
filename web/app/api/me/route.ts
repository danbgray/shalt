import { sessionFromRequest } from "@/lib/session";

export const dynamic = "force-dynamic";

export function GET(req: Request) {
  const s = sessionFromRequest(req);
  if (!s) {
    return Response.json({ login: null }, { status: 401 });
  }
  return Response.json({ login: s.login, id: s.id });
}
