<?php

declare(strict_types=1);

namespace OCA\MkvPlayer\Settings;

use OCA\MkvPlayer\AppInfo\Application;
use OCA\MkvPlayer\Service\ConfigService;
use OCA\MkvPlayer\Service\LicenseService;
use OCP\AppFramework\Http\TemplateResponse;
use OCP\AppFramework\Services\IInitialState;
use OCP\Settings\ISettings;
use OCP\Util;

/**
 * License admin settings — a custom (Vue) form in the MKV Player admin section, alongside the
 * declarative ffmpeg form. It renders a masked key field, a save/validate action, and a "Buy"
 * link. The raw key is admin-only: only whether a key is stored and its validity/email are seeded
 * to the page (never to the Viewer frontend — see ConfigService::getFrontendConfig).
 */
class LicenseAdminSettings implements ISettings {
	public function __construct(
		private IInitialState $initialState,
		private ConfigService $config,
		private LicenseService $license,
	) {
	}

	public function getForm(): TemplateResponse {
		Util::addScript(Application::APP_ID, 'mkvplayer-admin-settings');
		Util::addStyle(Application::APP_ID, 'mkvplayer-admin-settings');

		$stored = $this->config->getLicenseKey();
		$validation = $this->license->validate($stored);

		$this->initialState->provideInitialState('license', [
			'hasKey' => $stored !== '',
			'valid' => $validation['valid'],
			'email' => $validation['email'],
			'buyUrl' => $this->license->getBuyUrl(),
			'instanceId' => $this->license->getInstanceId(),
		]);

		return new TemplateResponse(Application::APP_ID, 'admin-settings');
	}

	public function getSection(): string {
		return Application::APP_ID;
	}

	public function getPriority(): int {
		return 40;
	}
}
