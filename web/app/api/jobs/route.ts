export function GET() {
  return Response.json({ jobs: [] });
}

export function POST() {
  return Response.json(
    { error: "Play is not on this Vercel deployment. Use a shalt engine or shalt connect." },
    { status: 501 },
  );
}
