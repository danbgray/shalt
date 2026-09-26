export function GET() {
  return Response.json({ grok: false, openai: false, anthropic: false, slots: [] });
}

export function POST() {
  return Response.json(
    { error: "Play and keys are not on this host. Point SHALT_ENGINE_URL at a shalt engine, or use shalt connect." },
    { status: 501 },
  );
}
