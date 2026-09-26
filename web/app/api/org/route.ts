export const dynamic = "force-dynamic";

export function GET() {
  return Response.json({
    name: "shalt.dev",
    projects: [],
    stacks: [{ id: "rust", label: "Rust", support: "supported" }],
    parallel: { local: 0, cloud: 0, cap_local: 1, cap_cloud: 6 },
    diagram: "",
    pipeline: "",
    hosted: true,
  });
}
