#!/usr/bin/env node

import { createServer } from "node:http";

const host = "127.0.0.1";
const port = 18080;

const server = createServer((request, response) => {
  response.setHeader("Access-Control-Allow-Origin", "*");
  response.setHeader("Access-Control-Allow-Private-Network", "true");
  response.setHeader("Access-Control-Allow-Methods", "GET, OPTIONS");
  response.setHeader("Cache-Control", "no-store");

  console.log(`${request.method} ${request.url}`);

  if (request.method === "OPTIONS") {
    response.writeHead(204);
    response.end();
    return;
  }

  if (request.method === "GET" && request.url === "/probe") {
    response.writeHead(200, { "Content-Type": "application/json" });
    response.end(JSON.stringify({ ok: true, source: "samp-cef-smoke" }));
    return;
  }

  response.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
  response.end("Not found\n");
});

server.listen(port, host, () => {
  console.log(`samp-cef smoke HTTP server listening at http://${host}:${port}`);
});
