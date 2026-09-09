import { handleBusiness } from '@/server/business';
async function handle(request: Request) {
  const server = process.env.AMUX_SERVER_URL;
  if (!server)
    return Response.json(
      {
        error:
          'This hosted view needs a reachable Amux server. The local Business app is connected to your existing server.',
      },
      { status: 503 },
    );
  return handleBusiness(request, (path, init = {}) =>
    fetch(new URL(path, server), {
      ...init,
      headers: {
        ...init.headers,
        ...(process.env.AMUX_AUTH_TOKEN
          ? { Authorization: 'Bearer ' + process.env.AMUX_AUTH_TOKEN }
          : {}),
      },
      redirect: 'error',
    }),
  );
}
export const GET = handle;
export const POST = handle;
export const PATCH = handle;
