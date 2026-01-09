// Simple Bun static file server for the vector editor web app

const MIME_TYPES: Record<string, string> = {
  ".html": "text/html",
  ".js": "application/javascript",
  ".mjs": "application/javascript",
  ".wasm": "application/wasm",
  ".css": "text/css",
  ".json": "application/json",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".svg": "image/svg+xml",
};

function getMimeType(path: string): string {
  const ext = path.substring(path.lastIndexOf("."));
  return MIME_TYPES[ext] || "application/octet-stream";
}

const server = Bun.serve({
  port: 3000,
  async fetch(req) {
    const url = new URL(req.url);
    let path = url.pathname;

    // Default to index.html
    if (path === "/") {
      path = "/index.html";
    }

    // Serve files from the current directory (apps/web)
    const filePath = `.${path}`;
    const file = Bun.file(filePath);

    if (await file.exists()) {
      const contentType = getMimeType(path);
      return new Response(file, {
        headers: {
          "Content-Type": contentType,
        },
      });
    }

    return new Response("Not Found", { status: 404 });
  },
});

console.log(`Vector Editor running at http://localhost:${server.port}`);
