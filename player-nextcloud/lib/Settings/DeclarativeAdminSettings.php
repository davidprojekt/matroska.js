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
				. 'cannot play natively (Vorbis, AC-3, E-AC-3, DTS core, …) are transcoded — to '
				. 'AAC-LC or Opus — with an audio-only ffmpeg.wasm that ships with this app and is '
				. 'served from your own server: no external requests. It is LGPL (AGPL-compatible); '
				. 'the lossy codecs are royalty-free or have expired core patents, except E-AC-3 '
				. '(newer — check your jurisdiction). TrueHD/DTS-HD/HE-AAC are not included; to add '
				. 'them, build a fuller ffmpeg.wasm yourself and enable the advanced option below '
				. '(see the app README).'
			),
			'fields' => [
				[
					'id' => ConfigService::KEY_TRANSCODE,
					'title' => $this->l->t('Enable audio transcoding'),
					'description' => $this->l->t('Uses the bundled royalty-free core, served from this server.'),
					'type' => DeclarativeSettingsTypes::CHECKBOX,
					'default' => ConfigService::DEFAULT_TRANSCODE,
				],
				[
					'id' => ConfigService::KEY_DEBUG,
					'title' => $this->l->t('Show debug status overlay'),
					'description' => $this->l->t('Overlays loading/status/error messages on the player. Off by default.'),
					'type' => DeclarativeSettingsTypes::CHECKBOX,
					'default' => false,
				],
				[
					'id' => ConfigService::KEY_EXTERNAL,
					'title' => $this->l->t('Advanced: load ffmpeg.wasm from an external server'),
					'description' => $this->l->t(
						'Off by default. When on, the ffmpeg core is fetched from the URLs below instead '
						. 'of the bundled one. PRIVACY: this makes each viewer\'s browser contact that '
						. 'third-party server (exposing their IP address and that they are watching a '
						. 'video); the bundled default is fully offline. Only enable it for a build you '
						. 'trust, and mind the patent obligations of any extra codecs it contains.'
					),
					'type' => DeclarativeSettingsTypes::CHECKBOX,
					'default' => false,
				],
				[
					'id' => ConfigService::KEY_CORE_URL,
					'title' => $this->l->t('External ffmpeg core URL (ffmpeg-core.js)'),
					'description' => $this->l->t('Used only when the advanced external option above is on.'),
					'type' => DeclarativeSettingsTypes::URL,
					'default' => '',
					'placeholder' => 'https://example.com/ffmpeg/ffmpeg-core.js',
				],
				[
					'id' => ConfigService::KEY_WASM_URL,
					'title' => $this->l->t('External ffmpeg wasm URL (ffmpeg-core.wasm)'),
					'type' => DeclarativeSettingsTypes::URL,
					'default' => '',
					'placeholder' => 'https://example.com/ffmpeg/ffmpeg-core.wasm',
				],
			],
		];
	}
}
