export const dynamic = "force-dynamic";

export function GET() {
  return Response.json({
    ok: true,
    hosted: true,
    engine: Boolean(process.env.SHALT_ENGINE_URL),
    play: process.env.SHALT_ENGINE_URL ? "engine" : "not-on-this-host",
  });
}
