export const dynamic = "force-dynamic";

export function GET() {
  const encoder = new TextEncoder();
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue(encoder.encode("event: hello\ndata: {}\n\n"));
      const t = setInterval(() => {
        try {
          controller.enqueue(encoder.encode("event: tick\ndata: {}\n\n"));
        } catch {
          clearInterval(t);
        }
      }, 20000);
    },
  });
  return new Response(stream, {
    headers: {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache, no-transform",
      Connection: "keep-alive",
    },
  });
}
