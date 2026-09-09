import http from 'node:http';
import https from 'node:https';
import type { Plugin } from 'vite';
import { handleBusiness } from './business';
// Development transport only. Production can reverse-proxy /api/business to the same adapter,
// or use the Worker route with an authenticated, externally reachable Amux URL.
export function businessBridge(): Plugin {
  return {
    name: 'amux-business-bridge',
    configureServer(server) {
      server.middlewares.use(async (req, res, next) => {
        if (!req.url?.startsWith('/api/business/')) return next();
        const host = req.headers.host || '';
        if (!/^(localhost|127\.0\.0\.1|\[::1\])(:\d+)?$/.test(host)) {
          res.writeHead(403);
          res.end('Local access only');
          return;
        }
        const origin = req.headers.origin;
        if (
          origin &&
          origin !== `http://${host}` &&
          origin !== `https://${host}`
        ) {
          res.writeHead(403);
          res.end('Same-origin access required');
          return;
        }
        if (
          req.method !== 'GET' &&
          req.headers['content-type']?.split(';')[0] !== 'application/json'
        ) {
          res.writeHead(415);
          res.end('JSON required');
          return;
        }
        const upstream = new URL(
          process.env.AMUX_SERVER_URL || 'https://localhost:8824',
        );
        if (!['http:', 'https:'].includes(upstream.protocol)) {
          res.writeHead(503);
          res.end('Invalid Amux server URL');
          return;
        }
        const chunks: Buffer[] = [];
        let size = 0;
        for await (const chunk of req) {
          size += chunk.length;
          if (size > 100_000) {
            res.writeHead(413);
            res.end('Request too large');
            return;
          }
          chunks.push(chunk);
        }
        const request = new Request(`http://${host}${req.url}`, {
          method: req.method,
          headers: { 'Content-Type': 'application/json' },
          ...(req.method !== 'GET' && req.method !== 'HEAD'
            ? { body: Buffer.concat(chunks).toString() }
            : {}),
        });
        const result = await handleBusiness(
          request,
          async (path, init = {}) =>
            new Promise<Response>((resolve, reject) => {
              const target = new URL(path, upstream);
              const headers: Record<string, string> = {
                Accept: 'application/json',
                ...(init.headers as Record<string, string>),
              };
              if (
                process.env.AMUX_AUTH_TOKEN &&
                process.env.AMUX_AUTH_TOKEN !== 'none'
              )
                headers.Authorization = 'Bearer ' + process.env.AMUX_AUTH_TOKEN;
              // Preserve explicit login from the existing owner session; never expose it to client JavaScript.
              if (req.headers.authorization)
                headers.Authorization = req.headers.authorization;
              if (req.headers.cookie) headers.Cookie = req.headers.cookie;
              const local = ['localhost', '127.0.0.1', '[::1]'].includes(
                target.hostname,
              );
              const upstreamRequest = (
                target.protocol === 'https:' ? https : http
              ).request(
                target,
                {
                  method: init.method || 'GET',
                  headers,
                  ...(target.protocol === 'https:'
                    ? { rejectUnauthorized: !local }
                    : {}),
                },
                (response) => {
                  const data: Buffer[] = [];
                  response.on('data', (chunk) => data.push(chunk));
                  response.on('end', () =>
                    resolve(
                      new Response(Buffer.concat(data).toString(), {
                        status: response.statusCode || 502,
                        headers: { 'Content-Type': 'application/json' },
                      }),
                    ),
                  );
                },
              );
              upstreamRequest.setTimeout(15000, () =>
                upstreamRequest.destroy(new Error('upstream_timeout')),
              );
              upstreamRequest.on('error', reject);
              if (init.body) upstreamRequest.write(init.body);
              upstreamRequest.end();
            }),
        );
        res.writeHead(result.status, Object.fromEntries(result.headers));
        res.end(await result.text());
      });
    },
  };
}
