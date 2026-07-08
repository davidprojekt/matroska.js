<?php

declare(strict_types=1);

namespace OCA\MkvPlayer\Settings;

use OCA\MkvPlayer\AppInfo\Application;
use OCA\MkvPlayer\Service\ConfigService;
use OCP\IL10N;
use OCP\Settings\DeclarativeSettingsTypes;
use OCP\Settings\IDeclarativeSettingsForm;

/**
 * Admin settings for the optional ffmpeg.wasm audio transcoder. No ffmpeg core ships with the
 * app, so an administrator points these at an ffmpeg.wasm build (ESM single-thread core + wasm).
 * Declarative form: Nextcloud renders the UI and stores the values in appconfig under the field
 * ids — the same keys ConfigService reads (see ConfigService::KEY_*).
 */
class DeclarativeAdminSettings implements IDeclarativeSettingsForm {
	public function __construct(
		private IL10N $l,
	) {
	}

	public function getSchema(): array {
		return [
			'id' => 'mkvplayer_ffmpeg',
			'priority' => 50,
			'section_type' => DeclarativeSettingsTypes::SECTION_TYPE_ADMIN,
			'section_id' => Application::APP_ID,
			'storage_type' => DeclarativeSettingsTypes::STORAGE_TYPE_INTERNAL,
			'title' => $this->l->t('In-browser audio transcoding (ffmpeg.wasm)'),
			'description' => $this->l->t(
				'Matroska video always plays via in-browser remuxing. Audio codecs the browser '
				. 'cannot play natively (e.g. AC-3, DTS) can optionally be transcoded with '
				. 'ffmpeg.wasm. No ffmpeg core ships with this app — enable transcoding and provide '
				. 'the URLs of an ffmpeg.wasm build (single-thread ESM core + wasm). The URLs must '
				. 'be reachable by the browser (same-origin, or a host that sends permissive CORS).'
			),
			'fields' => [
				[
					'id' => ConfigService::KEY_TRANSCODE,
					'title' => $this->l->t('Enable audio transcoding'),
					'description' => $this->l->t('Requires both URLs below to be set.'),
					'type' => DeclarativeSettingsTypes::CHECKBOX,
					'default' => ConfigService::DEFAULT_TRANSCODE,
				],
				[
					'id' => ConfigService::KEY_CORE_URL,
					'title' => $this->l->t('ffmpeg core URL (ffmpeg-core.js)'),
					'type' => DeclarativeSettingsTypes::URL,
					'default' => ConfigService::DEFAULT_CORE_URL,
					'placeholder' => ConfigService::DEFAULT_CORE_URL,
				],
				[
					'id' => ConfigService::KEY_WASM_URL,
					'title' => $this->l->t('ffmpeg wasm URL (ffmpeg-core.wasm)'),
					'type' => DeclarativeSettingsTypes::URL,
					'default' => ConfigService::DEFAULT_WASM_URL,
					'placeholder' => ConfigService::DEFAULT_WASM_URL,
				],
				[
					'id' => ConfigService::KEY_DEBUG,
					'title' => $this->l->t('Show debug status overlay'),
					'description' => $this->l->t('Overlays loading/status/error messages on the player. Off by default.'),
					'type' => DeclarativeSettingsTypes::CHECKBOX,
					'default' => false,
				],
			],
		];
	}
}
