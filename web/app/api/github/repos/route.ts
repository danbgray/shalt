import { sessionFromRequest } from "@/lib/session";

export const dynamic = "force-dynamic";

export async function GET(req: Request) {
  const s = sessionFromRequest(req);
  if (!s) {
    return Response.json({ error: "sign in with GitHub" }, { status: 401 });
  }
  const r = await fetch("https://api.github.com/user/repos?per_page=50&sort=updated", {
    headers: {
      Authorization: `Bearer ${s.gh}`,
      "User-Agent": "shalt.dev",
      Accept: "application/vnd.github+json",
    },
  });
  if (!r.ok) {
    return Response.json({ error: "github refused the list" }, { status: 502 });
  }
  const rows = (await r.json()) as Array<{
    full_name: string;
    html_url: string;
    clone_url: string;
    private: boolean;
    description: string | null;
  }>;
  return Response.json({
    repos: rows.map((x) => ({
      name: x.full_name,
      url: x.html_url,
      clone: x.clone_url,
      private: x.private,
      description: x.description || "",
    })),
  });
}
