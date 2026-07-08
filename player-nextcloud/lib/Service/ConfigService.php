<?php

declare(strict_types=1);

namespace OCA\MkvPlayer\Service;

use OCP\AppFramework\Services\IAppConfig;

/**
 * Reads/writes the app's admin configuration and shapes it for the frontend. No ffmpeg core is
 * bundled with the app; an administrator points these at an ffmpeg.wasm build (single-thread
 * ESM core + wasm). Until both URLs are set and transcoding is enabled, audio transcoding stays
 * off and unsupported audio codecs simply don't play (video still remuxes).
 */
class ConfigService {
	public const KEY_CORE_URL = 'ffmpeg_core_url';
	public const KEY_WASM_URL = 'ffmpeg_wasm_url';
	public const KEY_TRANSCODE = 'transcode_enabled';
	public const KEY_DEBUG = 'debug';

	// Default to the public single-thread ESM @ffmpeg/core on jsDelivr, with transcoding enabled
	// out of the box. jsDelivr sends permissive CORS, and CspListener adds its origin to
	// connect-src. Admins can point these at a self-hosted/free-codec build instead.
	public const DEFAULT_CORE_URL = 'https://cdn.jsdelivr.net/npm/@ffmpeg/core@0.12.10/dist/esm/ffmpeg-core.js';
	public const DEFAULT_WASM_URL = 'https://cdn.jsdelivr.net/npm/@ffmpeg/core@0.12.10/dist/esm/ffmpeg-core.wasm';
	public const DEFAULT_TRANSCODE = true;

	public function __construct(
		private IAppConfig $appConfig,
	) {
	}

	public function getCoreUrl(): string {
		return $this->appConfig->getAppValueString(self::KEY_CORE_URL, self::DEFAULT_CORE_URL);
	}

	public function getWasmUrl(): string {
		return $this->appConfig->getAppValueString(self::KEY_WASM_URL, self::DEFAULT_WASM_URL);
	}

	public function isTranscodeEnabled(): bool {
		return $this->appConfig->getAppValueBool(self::KEY_TRANSCODE, self::DEFAULT_TRANSCODE);
	}

	public function isDebugEnabled(): bool {
		return $this->appConfig->getAppValueBool(self::KEY_DEBUG, false);
	}

	public function setConfig(string $coreUrl, string $wasmUrl, bool $transcodeEnabled): void {
		$this->appConfig->setAppValueString(self::KEY_CORE_URL, trim($coreUrl));
		$this->appConfig->setAppValueString(self::KEY_WASM_URL, trim($wasmUrl));
		$this->appConfig->setAppValueBool(self::KEY_TRANSCODE, $transcodeEnabled);
	}

	/**
	 * The shape consumed by the frontend (main.js / the Viewer component), via initial state.
	 *
	 * @return array{ffmpeg: array{coreURL: string, wasmURL: string}, transcodeEnabled: bool, debug: bool}
	 */
	public function getFrontendConfig(): array {
		$core = $this->getCoreUrl();
		$wasm = $this->getWasmUrl();
		return [
			'ffmpeg' => ['coreURL' => $core, 'wasmURL' => $wasm],
			// Only actually transcode when enabled AND both URLs are configured.
			'transcodeEnabled' => $this->isTranscodeEnabled() && $core !== '' && $wasm !== '',
			'debug' => $this->isDebugEnabled(),
		];
	}
}
