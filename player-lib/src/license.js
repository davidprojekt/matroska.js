// Offline license verification for mkv-player-ui.
//
// A valid license removes the default "matroska.js" watermark and lets the caller show their own
// (via the `watermark` option). Verification is fully offline and makes no network request: an
// Ed25519 signature over a small payload, checked in the browser against the built-in public key.
//
// KEY FORMAT
//   base64url(payload) + "." + base64url(ed25519_signature(payload))
//   where `payload` is a URL-encoded query string, currently just "email=<addr>".
//
// The signature is verified but NOT bound to any origin (signature-only): a valid key unlocks on
// any site. The signing SEED must stay secret; the public key below is meant to be published.
//
// ⚠️ PLACEHOLDER KEY — replace PUBLIC_KEY_HEX with your production public key before selling
// licenses. This is the shared dev/test key (matches player-nextcloud); its seed is dev-only.
const PUBLIC_KEY_HEX = '535f9230fd3c5c2a0ff386b699c421657b3a3225c866353ac1dccae833902413';

const hexToBytes = (hex) => {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
};

const b64urlToBytes = (s) => {
  s = s.replace(/-/g, '+').replace(/_/g, '/');
  s += '='.repeat((4 - (s.length % 4)) % 4);
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
};

/**
 * Verify a license key's Ed25519 signature against the built-in public key. Resolves `true` only
 * for a well-formed, correctly-signed key. Never throws: any error (malformed key, or a platform
 * whose WebCrypto lacks Ed25519) resolves `false`, leaving the default watermark in place.
 */
export async function verifyLicense(key) {
  try {
    if (typeof key !== 'string' || !key.includes('.')) return false;
    const [payloadB64, sigB64] = key.split('.', 2);
    const message = b64urlToBytes(payloadB64);
    const signature = b64urlToBytes(sigB64);
    if (signature.length !== 64) return false;
    const subtle = globalThis.crypto?.subtle;
    if (!subtle) return false;
    const pub = await subtle.importKey('raw', hexToBytes(PUBLIC_KEY_HEX), { name: 'Ed25519' }, false, [
      'verify',
    ]);
    return await subtle.verify('Ed25519', pub, signature, message);
  } catch {
    return false;
  }
}
