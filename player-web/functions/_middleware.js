export async function onRequest(context) {
  const { request, env } = context;
  
  // 1. Only enforce basic auth on preview deployments
  // (Optional: Remove this check if you want it on production too)
  const url = new URL(request.url);
  
  // Cloudflare automatically sets environment variables for the environment
  // You can check if the host contains 'pages.dev' and isn't the main production domain
  // Alternatively, just enforce it everywhere if this project is purely for staging
  
  const authUser = env.BASIC_AUTH_USER || "admin";
  const authPass = env.BASIC_AUTH_PASS || "password123";

  // If credentials aren't set in Cloudflare dashboard, bypass to avoid locking yourself out
  if (!authUser || !authPass) {
    return context.next();
  }

  const authHeader = request.headers.get('Authorization');

  if (authHeader) {
    // Extract the base64 encoded credentials
    const [scheme, encoded] = authHeader.split(' ');
    
    if (scheme.toLowerCase() === 'basic' && encoded) {
      try {
        const decoded = atob(encoded);
        const [username, password] = decoded.split(':');

        // Verify credentials
        if (username === authUser && password === authPass) {
          return context.next(); // Credentials match, proceed to Vite static site
        }
      } catch (e) {
        // Invalid base64 encoding, fall through to prompt
      }
    }
  }

  // 2. If unauthorized, return a 401 Response with the standard Basic Auth header
  return new Response('Unauthorized', {
    status: 401,
    headers: {
      'WWW-Authenticate': 'Basic realm="Secure Preview Deployment", charset="UTF-8"',
    },
  });
}
