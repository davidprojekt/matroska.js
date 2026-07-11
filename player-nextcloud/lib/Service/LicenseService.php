<?php

declare(strict_types=1);

namespace OCA\MkvPlayer\Service;

use OCP\AppFramework\Services\IAppConfig;
use OCP\IConfig;

/**
 * Validates the (optional) paid-license key and derives the purchase URL.
 *
 * A license key is a short, signed token that the administrator pastes into the admin settings.
 * It is an Ed25519 signature over a url-encoded payload that names the licensed email and the
 * Nextcloud instance the license is bound to:
 *
 *     payload   = "email=<addr>&nc_instance=<instanceid>"   (application/x-www-form-urlencoded)
 *     key       = base64url(payload) . "." . base64url(ed25519_signature(payload))
 *
 * Validation verifies the signature against our public key and then requires the payload's
 * `nc_instance` to equal *this* instance's id — so a key issued for one instance can't be reused
 * on another. The raw key is NEVER handed to the end-user frontend; only the boolean result of
 * this validation is (see ConfigService::getFrontendConfig).
 */
class LicenseService {
	/**
	 * Ed25519 public key (hex, 32 bytes) used to verify license signatures.
	 *
	 * NOTE: this is a generated TEST key — replace it with your production public key. The matching
	 * private seed lives in dev/license-test-keys.txt (gitignored) and is used by
	 * scripts/sign-license.php to mint test keys.
	 */
	public const PUBLIC_KEY_HEX = '535f9230fd3c5c2a0ff386b699c421657b3a3225c866353ac1dccae833902413';

	/**
	 * Purchase page (landing site handles Nextcloud orders). getBuyUrl() substitutes the current
	 * instance id into the %NC% placeholder.
	 */
	public const BUY_URL = 'https://matroska.davidschneider.xyz/nextcloud?nc_instance=%NC%#pricing';

	public function __construct(
		private IConfig $config,
		private IAppConfig $appConfig,
	) {
	}

	/** This Nextcloud instance's id (from config.php `instanceid`). */
	public function getInstanceId(): string {
		return (string)$this->config->getSystemValue('instanceid', '');
	}

	/**
	 * Verify a license key against our public key and this instance's id.
	 *
	 * @return array{valid: bool, email: ?string} `valid` is true only when the signature checks out
	 *   AND the payload's nc_instance matches this instance. `email` is the licensed address (for
	 *   display) when present, regardless of validity. Malformed input returns invalid, never throws.
	 */
	public function validate(string $key): array {
		$invalid = ['valid' => false, 'email' => null];

		$key = trim($key);
		if ($key === '') {
			return $invalid;
		}

		$parts = explode('.', $key);
		if (count($parts) !== 2) {
			return $invalid;
		}

		$payload = self::base64UrlDecode($parts[0]);
		$signature = self::base64UrlDecode($parts[1]);
		if ($payload === null || $signature === null || strlen($signature) !== SODIUM_CRYPTO_SIGN_BYTES) {
			return $invalid;
		}

		$publicKey = @sodium_hex2bin(self::PUBLIC_KEY_HEX);
		if (strlen($publicKey) !== SODIUM_CRYPTO_SIGN_PUBLICKEYBYTES) {
			return $invalid;
		}

		try {
			$ok = sodium_crypto_sign_verify_detached($signature, $payload, $publicKey);
		} catch (\SodiumException $e) {
			return $invalid;
		}

		$fields = [];
		parse_str($payload, $fields);
		$email = isset($fields['email']) ? (string)$fields['email'] : null;
		$ncInstance = isset($fields['nc_instance']) ? (string)$fields['nc_instance'] : '';

		$instanceId = $this->getInstanceId();
		$valid = $ok && $instanceId !== '' && hash_equals($instanceId, $ncInstance);

		return ['valid' => $valid, 'email' => $email];
	}

	/** Whether the stored key is a valid license for this instance. */
	public function isLicensed(): bool {
		return $this->validate($this->getStoredKey())['valid'];
	}

	/** The stored license key (admin-only; never exposed to the Viewer frontend). */
	public function getStoredKey(): string {
		return $this->appConfig->getAppValueString(ConfigService::KEY_LICENSE, '');
	}

	/** Purchase URL with this instance's id appended, for the admin "Buy" link. */
	public function getBuyUrl(): string {
		return str_replace('%NC%', rawurlencode($this->getInstanceId()), self::BUY_URL);
	}

	/** Base64url decode (no padding), returning null on invalid input. */
	private static function base64UrlDecode(string $s): ?string {
		$s = strtr($s, '-_', '+/');
		$pad = strlen($s) % 4;
		if ($pad > 0) {
			$s .= str_repeat('=', 4 - $pad);
		}
		$decoded = base64_decode($s, true);
		return $decoded === false ? null : $decoded;
	}
}
