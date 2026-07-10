<?php

declare(strict_types=1);

namespace OCA\MkvPlayer\Service;

use OCP\AppFramework\Services\IAppConfig;

/**
 * Reads/writes the app's admin configuration and shapes it for the frontend.
 *
 * The app bundles a royalty-free, audio-only ffmpeg.wasm core and serves it same-origin (offline,
 * no external calls) — that's the default. Transcoding audio the browser can't play natively is on
 * by default and uses that bundled core. An admin can optionally opt in to loading ffmpeg from an
 * external server (e.g. a self-hosted fuller build); only then are the core/wasm URLs used.
 */
class ConfigService {
	public const KEY_TRANSCODE = 'transcode_enabled';
	public const KEY_DEBUG = 'debug';
	public const KEY_EXTERNAL = 'ffmpeg_external';
	public const KEY_CORE_URL = 'ffmpeg_core_url';
	public const KEY_WASM_URL = 'ffmpeg_wasm_url';
	public const KEY_LICENSE = 'license_key';

	public const DEFAULT_TRANSCODE = true;

	public function __construct(
		private IAppConfig $appConfig,
		private LicenseService $license,
	) {
	}

	public function isTranscodeEnabled(): bool {
		return $this->appConfig->getAppValueBool(self::KEY_TRANSCODE, self::DEFAULT_TRANSCODE);
	}

	public function isDebugEnabled(): bool {
		return $this->appConfig->getAppValueBool(self::KEY_DEBUG, false);
	}

	/** Whether to load ffmpeg from an external server instead of the bundled same-origin core. */
	public function isExternal(): bool {
		return $this->appConfig->getAppValueBool(self::KEY_EXTERNAL, false);
	}

	public function getCoreUrl(): string {
		return $this->appConfig->getAppValueString(self::KEY_CORE_URL, '');
	}

	public function getWasmUrl(): string {
		return $this->appConfig->getAppValueString(self::KEY_WASM_URL, '');
	}

	/** The stored license key (admin-only; never shipped to the frontend). */
	public function getLicenseKey(): string {
		return $this->appConfig->getAppValueString(self::KEY_LICENSE, '');
	}

	public function setLicenseKey(string $key): void {
		$this->appConfig->setAppValueString(self::KEY_LICENSE, trim($key));
	}

	/** External core/wasm URLs, but only when the external opt-in is on and both are set. */
	public function getExternalUrls(): ?array {
		if (!$this->isExternal()) {
			return null;
		}
		$core = trim($this->getCoreUrl());
		$wasm = trim($this->getWasmUrl());
		if ($core === '' || $wasm === '') {
			return null;
		}
		return ['coreURL' => $core, 'wasmURL' => $wasm];
	}

	/**
	 * The shape consumed by the frontend (the Viewer component), via initial state. When `external`
	 * is null the frontend uses the bundled same-origin core (URL built client-side).
	 *
	 * `licensed` is the validated result of the admin's license key (the raw key is never exposed
	 * here); the frontend uses it to hide the watermark for licensed instances.
	 *
	 * @return array{transcodeEnabled: bool, debug: bool, external: ?array{coreURL: string, wasmURL: string}, licensed: bool}
	 */
	public function getFrontendConfig(): array {
		return [
			'transcodeEnabled' => $this->isTranscodeEnabled(),
			'debug' => $this->isDebugEnabled(),
			'external' => $this->getExternalUrls(),
			'licensed' => $this->license->isLicensed(),
		];
	}
}
