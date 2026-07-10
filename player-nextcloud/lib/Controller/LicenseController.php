<?php

declare(strict_types=1);

namespace OCA\MkvPlayer\Controller;

use OCA\MkvPlayer\Service\ConfigService;
use OCA\MkvPlayer\Service\LicenseService;
use OCP\AppFramework\Controller;
use OCP\AppFramework\Http\DataResponse;
use OCP\IRequest;

/**
 * Saves and validates the admin's license key. Nextcloud controller routes are admin-only and
 * CSRF-protected by default (there is no #[NoAdminRequired] here to relax that), so only an admin
 * with a valid request token can call this. Returns only the validation result — the key itself is
 * stored server-side and never echoed back to the client.
 */
class LicenseController extends Controller {
	public function __construct(
		string $appName,
		IRequest $request,
		private ConfigService $config,
		private LicenseService $license,
	) {
		parent::__construct($appName, $request);
	}

	/**
	 * Persist the given key and report whether it is a valid license for this instance.
	 *
	 * @return DataResponse<array{valid: bool, email: ?string}>
	 */
	public function save(string $key): DataResponse {
		$this->config->setLicenseKey($key);
		return new DataResponse($this->license->validate($key));
	}
}
