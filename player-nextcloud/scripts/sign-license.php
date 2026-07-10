<?php

declare(strict_types=1);

/**
 * Mint a license key for testing (or production, with the real private seed).
 *
 * A key is:  base64url(payload) . "." . base64url(ed25519_signature(payload))
 * where payload = "email=<addr>&nc_instance=<instanceid>" (url-encoded).
 * The signature is made with the Ed25519 private key whose PUBLIC half is embedded in
 * LicenseService::PUBLIC_KEY_HEX. See dev/license-test-keys.txt for the test seed.
 *
 * Usage:
 *   MKV_LICENSE_SEED_HEX=<32-byte-seed-hex> \
 *     php scripts/sign-license.php <email> <nc_instance>
 *
 * The nc_instance is the target Nextcloud's `instanceid` (config.php / `occ config:system:get
 * instanceid`). The generated key is only valid on that instance.
 */

if ($argc < 3) {
	fwrite(STDERR, "Usage: MKV_LICENSE_SEED_HEX=<hex> php scripts/sign-license.php <email> <nc_instance>\n");
	exit(2);
}

$seedHex = getenv('MKV_LICENSE_SEED_HEX') ?: '';
if ($seedHex === '') {
	fwrite(STDERR, "Set MKV_LICENSE_SEED_HEX to the 32-byte Ed25519 seed (hex). See dev/license-test-keys.txt.\n");
	exit(2);
}

$email = $argv[1];
$ncInstance = $argv[2];

$seed = sodium_hex2bin($seedHex);
if (strlen($seed) !== SODIUM_CRYPTO_SIGN_SEEDBYTES) {
	fwrite(STDERR, 'Seed must be ' . SODIUM_CRYPTO_SIGN_SEEDBYTES . " bytes (" . (SODIUM_CRYPTO_SIGN_SEEDBYTES * 2) . " hex chars).\n");
	exit(2);
}

$keypair = sodium_crypto_sign_seed_keypair($seed);
$secretKey = sodium_crypto_sign_secretkey($keypair);
$publicKey = sodium_crypto_sign_publickey($keypair);

// Matches LicenseService's payload shape exactly.
$payload = http_build_query(['email' => $email, 'nc_instance' => $ncInstance]);
$signature = sodium_crypto_sign_detached($payload, $secretKey);

$base64url = static fn (string $b): string => rtrim(strtr(base64_encode($b), '+/', '-_'), '=');
$key = $base64url($payload) . '.' . $base64url($signature);

echo "public key (hex): " . sodium_bin2hex($publicKey) . "\n";
echo "payload:          " . $payload . "\n";
echo "license key:      " . $key . "\n";
